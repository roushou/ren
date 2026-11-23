//! `ren project`: the projects this host's daemon has registered (`add`,
//! `list`, `archive`, `unarchive`), and a project's declared contract
//! (`.ren/project.toml`) at work on this host: its checks (`check`), a
//! preview of a worktree's branch (`preview up`, `preview down`), as Ren's
//! own (`cargo xtask preview`) runs for Ren, and a registered project's
//! canary and promotion.
//!
//! A preview builds the worktree, starts its serve step as the transient
//! unit `ren-preview-<worktree>-serve` on the worktree's ports, waits for
//! its health path, and records it on each sent work item the branch's
//! commits name (`Work-Item: <id>`), or that `--work` gives, so the Work
//! view opens the marked page on it. Removing the worktree stops it.
//!
//! Every step runs with Ren's package set (`ren packages`) first on its
//! search path, as Ren's runs do; a package the contract names
//! (`packages`) that the package set or the host lacks refuses the command
//! before any step runs.
//!
//! This module is the one place the CLI runs a project's declared steps.

mod canary;
mod registry;

use self::{
    canary::ProjectFactory,
    registry::{AddArgs, ListArgs, NameArgs, ProjectArgs, Registry},
};
use super::history::TargetArgs;
use crate::{context::Context, management::Management};
use anyhow::{Context as _, Result, anyhow, bail};
use clap::{Args, Subcommand};
use ren_daemon::packages::{HostPrograms, PackageStore};
use ren_depot::package_set::{Generation, Lack};
use ren_gateway::api::{
    control::request::{ControlRequest, PreviewWorkRequest, WorkChange},
    read::{ReadBody, work::WorkPreviewBody},
};
use ren_git::Git;
use ren_harness::host::HostEnv;
use ren_network::tailscale::{NodeStatus, Route, ServeStatus};
use ren_project::{Expanded, Place, ProjectContract};
use ren_protocol::label::Stored;
use ren_recorder::{
    id::WorkItemId,
    work::{Landing, PreviewUrl, WorkStateKind},
};
use ren_release::Revision;
use ren_store::{
    query::{PageLimit, Shelf},
    read::ReadRequest,
};
use ren_supervisor::Worktree;
use std::{
    path::{Path, PathBuf},
    process::{ExitCode, Stdio},
    time::Duration,
};

/// Run a project's declared checks, or preview a worktree's branch.
#[derive(Args)]
pub struct ProjectCmd {
    #[command(subcommand)]
    action: ProjectAction,
}

#[derive(Subcommand)]
enum ProjectAction {
    /// Register a repository as a project.
    Add(AddArgs),
    /// The registered projects, with what is under way for each.
    List(ListArgs),
    /// Archive a project: it keeps its name and repository, and is not
    /// built.
    Archive(NameArgs),
    Unarchive(NameArgs),
    /// Assign a project the sessions no project owns whose folder is in
    /// its repository or is one of its worktrees.
    Adopt(NameArgs),
    /// Run the checks `.ren/project.toml` declares, in order; fails when one
    /// does.
    Check(CheckArgs),
    /// A preview of this worktree's branch.
    Preview {
        #[command(subcommand)]
        action: PreviewAction,
    },
    /// One canary pass: fetch the project's main, build it, serve it,
    /// check it, and deploy it if the operator's policy says so.
    Canary(CanaryArgs),
    /// Deploy the canary's checked build now, whatever the policy says.
    Promote(ProjectArgs),
    /// Where the project's release stands.
    Release(ProjectArgs),
}

#[derive(Args)]
struct CanaryArgs {
    #[command(flatten)]
    project: ProjectArgs,
    #[arg(long, default_value = "origin")]
    remote: String,
    #[arg(long, default_value = "main")]
    branch: String,
}

#[derive(Args)]
struct CheckArgs {
    /// The project's folder; by default the worktree this runs in, else
    /// the current folder.
    #[arg(long)]
    dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum PreviewAction {
    /// Build this worktree, serve it on its ports, and record it on the
    /// work items its branch addresses.
    Up(PreviewArgs),
    /// Stop it, and take it off those work items.
    Down(PreviewArgs),
}

#[derive(Args)]
struct PreviewArgs {
    /// Publish it on the tailnet too, on the port its page opens on.
    #[arg(long)]
    publish: bool,
    /// Work items to record it on, besides those the branch's commits name.
    #[arg(long = "work", value_name = "ID")]
    work: Vec<WorkItemId>,
    #[command(flatten)]
    target: TargetArgs,
}

/// Where a project's steps run, and the worktree it is, when it is one.
struct ProjectPlace {
    place: Place,
    worktree: Option<Worktree>,
}

/// The one place the CLI runs a project's steps and its preview's unit,
/// with the search path each gets.
struct Steps {
    /// Ren's package set's programs, then the caller's search path; `None`
    /// keeps the caller's.
    path: Option<String>,
}

impl ProjectCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        match self.action {
            ProjectAction::Add(args) => Registry::add(ctx, args).await,
            ProjectAction::List(args) => Registry::list(ctx, args).await,
            ProjectAction::Archive(args) => Registry::archive(ctx, args).await,
            ProjectAction::Unarchive(args) => Registry::unarchive(ctx, args).await,
            ProjectAction::Adopt(args) => Registry::adopt(ctx, args).await,
            ProjectAction::Check(args) => Self::check(ctx, args),
            ProjectAction::Preview { action } => match action {
                PreviewAction::Up(args) => Self::up(ctx, args).await,
                PreviewAction::Down(args) => Self::down(ctx, args).await,
            },
            ProjectAction::Canary(args) => {
                let project = Registry::resolve(ctx, &args.project).await?;
                ProjectFactory::new(ctx, &project)?
                    .pass(ctx, &args.remote, &args.branch)
                    .await
            }
            ProjectAction::Promote(args) => {
                let project = Registry::resolve(ctx, &args).await?;
                ProjectFactory::new(ctx, &project)?.promote(ctx).await
            }
            ProjectAction::Release(args) => {
                let project = Registry::resolve(ctx, &args).await?;
                ProjectFactory::new(ctx, &project)?.show()
            }
        }
    }

    pub(super) fn contract(dir: &Path) -> Result<ProjectContract> {
        let path = dir.join(ProjectContract::FILE);
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "{} declares how Ren builds, serves, and checks a project; none at {}",
                ProjectContract::FILE,
                path.display()
            )
        })?;
        Ok(ProjectContract::read(&text)?)
    }

    fn check(ctx: &Context, args: CheckArgs) -> Result<ExitCode> {
        let place = ProjectPlace::read(&ctx.env, args.dir)?;
        let contract = Self::contract(&place.place.worktree)?;
        if contract.checks().is_empty() {
            println!("{} declares no checks.", ProjectContract::FILE);
            return Ok(ExitCode::SUCCESS);
        }
        let steps = Steps::of(ctx, &contract)?;
        let mut failed = Vec::new();
        for check in contract.checks() {
            println!("==> {}", check.name());
            let step = check.step().expand(&place.place)?;
            if !steps.run(&step, &place.place.worktree)? {
                failed.push(check.name().to_owned());
            }
        }
        if failed.is_empty() {
            println!("Every check passed.");
            Ok(ExitCode::SUCCESS)
        } else {
            println!("Failed: {}", failed.join(", "));
            Ok(ExitCode::FAILURE)
        }
    }

    async fn up(ctx: &Context, args: PreviewArgs) -> Result<ExitCode> {
        let place = ProjectPlace::read(&ctx.env, None)?;
        let worktree = place.worktree()?;
        let contract = Self::contract(&place.place.worktree)?;
        let serve = contract.serve().with_context(|| {
            format!(
                "{} declares no [serve] step to preview",
                ProjectContract::FILE
            )
        })?;
        let steps = Steps::of(ctx, &contract)?;
        for step in contract.build() {
            let step = step.expand(&place.place)?;
            println!("==> {}", step.argv.join(" "));
            if !steps.run(&step, &place.place.worktree)? {
                bail!("a build step failed; the preview is not started");
            }
        }
        let unit = Self::unit(worktree);
        Steps::stop(&unit)?;
        let step = serve.step().expand(&place.place)?;
        steps.start(&unit, &step, &place.place.worktree)?;
        let open = serve.open(&place.place)?;
        Self::healthy(&open, serve.health()).await?;
        let mut origin: PreviewUrl = open
            .parse()
            .with_context(|| format!("{open} is not an origin a work item keeps"))?;
        if args.publish {
            let port = origin
                .port()
                .with_context(|| format!("{open} names no port to publish"))?;
            origin = Steps::publish(port)?;
        }
        let revision = Self::revision(&place.place.worktree)?;
        let items = Self::addressed(ctx, &args, &place.place.worktree).await?;
        let target = super::control::ControlTarget::select(&args.target, ctx)?;
        for item in &items {
            let answer = target
                .send(&ControlRequest::Work {
                    item: *item,
                    change: WorkChange::Preview(PreviewWorkRequest {
                        preview: Some(WorkPreviewBody {
                            ui: origin.clone(),
                            revision: revision.clone(),
                        }),
                    }),
                })
                .await?;
            if !answer.status.is_success() {
                bail!("recording the preview on {item} answered {}", answer.status);
            }
        }
        println!("Preview of {revision}: {origin}");
        match items.len() {
            0 => println!(
                "No work item in a session is named yet: commit with a `Work-Item:` trailer, or pass --work."
            ),
            count => println!(
                "Recorded on {count} work item(s); each opens it beside production on its project's Overview."
            ),
        }
        Ok(ExitCode::SUCCESS)
    }

    async fn down(ctx: &Context, args: PreviewArgs) -> Result<ExitCode> {
        let place = ProjectPlace::read(&ctx.env, None)?;
        let worktree = place.worktree()?;
        let contract = Self::contract(&place.place.worktree)?;
        if let Some(quiesce) = contract.quiesce() {
            let step = quiesce.expand(&place.place)?;
            println!("==> {}", step.argv.join(" "));
            Steps::of(ctx, &contract)?.run(&step, &place.place.worktree)?;
        }
        Steps::stop(&Self::unit(worktree))?;
        let port = contract
            .serve()
            .and_then(|serve| serve.open(&place.place).ok())
            .and_then(|open| open.parse::<PreviewUrl>().ok())
            .and_then(|open| open.port());
        if let (true, Some(port)) = (args.publish, port) {
            Steps::unpublish(port)?;
        }
        let target = super::control::ControlTarget::select(&args.target, ctx)?;
        let listed = Self::listed(ctx, &args).await?;
        for item in Self::addressed(ctx, &args, &place.place.worktree).await? {
            // Only this preview's, by the port its page opened on.
            let ours = listed.iter().any(|entry| {
                entry.id == item
                    && entry
                        .preview
                        .as_ref()
                        .is_some_and(|preview| preview.ui.port() == port)
            });
            if ours {
                target
                    .send(&ControlRequest::Work {
                        item,
                        change: WorkChange::Preview(PreviewWorkRequest { preview: None }),
                    })
                    .await?;
            }
        }
        println!("The preview is down.");
        Ok(ExitCode::SUCCESS)
    }

    /// The unit the worktree's preview runs as: the one removing the
    /// worktree stops.
    pub(super) fn unit(worktree: &Worktree) -> String {
        worktree.preview_unit()
    }

    /// Waits up to a minute for `open` to answer `health`.
    pub(super) async fn healthy(open: &str, health: &str) -> Result<()> {
        let url = format!("{}{health}", open.trim_end_matches('/'));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?;
        for _ in 0..120 {
            if let Ok(response) = client.get(&url).send().await
                && response.status().is_success()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        bail!("{url} did not answer in a minute; see `journalctl --user -u 'ren-preview-*'`")
    }

    /// The build the worktree's checkout is: its commit, modified or not.
    fn revision(dir: &Path) -> Result<Revision> {
        let git = Git::inherited();
        Ok(Revision::Commit {
            commit: git.head_commit(dir)?,
            tree: git.tree(dir)?,
        })
    }

    /// Work items under way on this host's daemon.
    async fn listed(
        ctx: &Context,
        args: &PreviewArgs,
    ) -> Result<Vec<ren_gateway::api::read::work::WorkItemView>> {
        let management: Management = args.target.gateway(ctx)?;
        let body = management
            .serve(&ReadRequest::WorkItems {
                limit: PageLimit::LARGEST,
                shelf: Shelf::Current,
                project: None,
            })
            .await?;
        match body {
            ReadBody::WorkItems(items) => Ok(items.work),
            ReadBody::Calls(_)
            | ReadBody::Call(_)
            | ReadBody::Recordings(_)
            | ReadBody::Turns(_)
            | ReadBody::Turn(_)
            | ReadBody::CallActivity(_)
            | ReadBody::Report(_)
            | ReadBody::Usage(_)
            | ReadBody::Search(_)
            | ReadBody::Exposures(_)
            | ReadBody::Runs(_)
            | ReadBody::Run(_)
            | ReadBody::Conversation(_)
            | ReadBody::WorkItem(_) => {
                bail!("the daemon answered a work listing with another read")
            }
        }
    }

    /// The sent work items this branch addresses: those its recent commits
    /// name, and those given.
    async fn addressed(ctx: &Context, args: &PreviewArgs, dir: &Path) -> Result<Vec<WorkItemId>> {
        let listed = Self::listed(ctx, args).await?;
        let git = Git::inherited();
        let head = git.head_commit(dir)?;
        let log = git.log(dir, &head, 200)?;
        let named = Landing::named(log.iter().map(|(_, message)| message.as_str()), &args.work);
        Ok(named
            .into_iter()
            .filter(|item| {
                listed.iter().any(|entry| {
                    entry.id == *item && entry.state == Stored::Known(WorkStateKind::Sent)
                })
            })
            .collect())
    }
}

impl ProjectPlace {
    /// The worktree this runs in, read from `REN_WORKTREE` alone and
    /// checked to be one Ren made: its data folder and ports are the ones
    /// Ren gives its slot. Without one, `dir` (or the current folder), with
    /// no ports.
    fn read(env: &HostEnv, dir: Option<PathBuf>) -> Result<Self> {
        let worktree = env
            .absolute_path("REN_WORKTREE")
            .and_then(|folder| folder.parent().and_then(|root| Worktree::of(root, &folder)));
        match (dir, worktree) {
            (None, Some(worktree)) => Ok(Self {
                place: Place {
                    worktree: worktree.folder(),
                    data: worktree.data(),
                    ports: Some(worktree.ports()),
                },
                worktree: Some(worktree),
            }),
            (dir, _) => {
                let dir = match dir {
                    Some(dir) => dir,
                    None => std::env::current_dir()?,
                };
                Ok(Self {
                    place: Place {
                        data: dir.join(".ren-data"),
                        worktree: dir,
                        ports: None,
                    },
                    worktree: None,
                })
            }
        }
    }

    fn worktree(&self) -> Result<&Worktree> {
        self.worktree.as_ref().context(
            "a preview runs in a worktree Ren made: REN_WORKTREE names a folder of Ren's worktrees, `<repo>-<slot>`",
        )
    }
}

impl Steps {
    /// The steps of `contract` on this host: Ren's current package set
    /// goes first on their search path. Refused when a package the
    /// contract names is lacking, or the package set it would come from
    /// cannot be read.
    pub(super) fn of(ctx: &Context, contract: &ProjectContract) -> Result<Self> {
        let config = ctx.config()?;
        let store =
            PackageStore::without_downloads(ctx.data_dir(&config)?.join(PackageStore::FOLDER));
        let caller = Self {
            path: ctx.env.get("PATH").map(str::to_owned),
        };
        let set = match store.current() {
            Ok(set) => set,
            Err(error) if contract.packages().is_empty() => {
                eprintln!(
                    "Ren's package set does not read ({error}); steps run with your PATH alone."
                );
                return Ok(caller);
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "{} names packages, and Ren's package set does not read",
                        ProjectContract::FILE
                    )
                });
            }
        };
        let search = ctx.env.search_path();
        let lacks = set.lacks(
            &config.packages.depot()?,
            &HostPrograms::new(search.clone()),
            contract.packages(),
        );
        if !lacks.is_empty() {
            bail!(Self::lacking(&lacks));
        }
        if set.generation() == Generation::NONE {
            return Ok(caller);
        }
        let bin = store.bin(set.generation());
        let dirs: Vec<PathBuf> = std::iter::once(bin.clone())
            .chain(search.into_iter().filter(|dir| *dir != bin))
            .collect();
        let path = std::env::join_paths(dirs)?
            .into_string()
            .map_err(|_| anyhow!("the search path is not text"))?;
        let installs: Vec<String> = set
            .installs()
            .map(|install| format!("{} {}", install.package, install.version))
            .collect();
        println!(
            "==> Packages: generation {} ({})",
            set.generation(),
            installs.join(", ")
        );
        Ok(Self { path: Some(path) })
    }

    /// What to tell the operator about `lacks`, one line each.
    fn lacking(lacks: &[Lack]) -> String {
        let lines: Vec<String> = lacks
            .iter()
            .map(|lack| match lack {
                Lack::Unknown(_) => {
                    format!("  {lack}: add it to the daemon's ren.toml as a [[packages]] entry")
                }
                Lack::NotInstalled(id) => format!("  {lack}: run `ren packages install {id}`"),
                Lack::NotOnHost { .. } => format!("  {lack}: install it on this host"),
            })
            .collect();
        format!(
            "{} names packages its steps cannot have:\n{}",
            ProjectContract::FILE,
            lines.join("\n")
        )
    }

    /// Runs `step` in `dir`, in the foreground; whether it succeeded.
    pub(super) fn run(&self, step: &Expanded, dir: &Path) -> Result<bool> {
        Self::status(step, dir, self.path.as_deref())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the one place the CLI runs a project's declared steps"
    )]
    fn status(step: &Expanded, dir: &Path, path: Option<&str>) -> Result<bool> {
        let [program, args @ ..] = step.argv.as_slice() else {
            bail!("an empty command");
        };
        let mut command = std::process::Command::new(program);
        if let Some(path) = path {
            command.env("PATH", path);
        }
        let status = command
            .args(args)
            .envs(step.env.iter().map(|(name, value)| (name, value)))
            .current_dir(dir)
            .stdin(Stdio::null())
            .status()
            .with_context(|| format!("cannot start {program}"))?;
        Ok(status.success())
    }

    /// Starts `step` as the transient user unit `unit`, in `dir`, with the
    /// steps' search path: a unit otherwise gets the user manager's, which
    /// lacks Ren's package set and the programs in the operator's home.
    pub(super) fn start(&self, unit: &str, step: &Expanded, dir: &Path) -> Result<()> {
        let mut argv = vec![
            "systemd-run".to_owned(),
            "--user".to_owned(),
            "--collect".to_owned(),
            "--quiet".to_owned(),
            format!("--unit={unit}"),
            format!("--working-directory={}", dir.display()),
        ];
        if let Some(path) = &self.path {
            argv.push(format!("--setenv=PATH={path}"));
        }
        argv.extend(
            step.env
                .iter()
                .map(|(name, value)| format!("--setenv={name}={value}")),
        );
        argv.push("--".to_owned());
        argv.extend(step.argv.iter().cloned());
        let start = Expanded {
            argv,
            env: Vec::new(),
        };
        if self.run(&start, dir)? {
            Ok(())
        } else {
            bail!("systemd-run did not start {unit}")
        }
    }

    /// Stops `unit` if it runs; one that does not is fine.
    pub(super) fn stop(unit: &str) -> Result<()> {
        for verb in ["stop", "reset-failed"] {
            let argv = ["systemctl", "--user", verb, unit]
                .map(str::to_owned)
                .to_vec();
            Self::quiet(&argv)?;
        }
        Ok(())
    }

    /// Publishes the loopback `port` on the tailnet on the same port, and
    /// returns its origin there. A port Serve already uses for anything
    /// else, or exposes through Funnel, is refused, never replaced.
    fn publish(port: u16) -> Result<PreviewUrl> {
        let target = format!("http://127.0.0.1:{port}");
        let node = NodeStatus::parse(&Self::output(&["tailscale", "status", "--json"])?)?;
        if let Some(state) = node.not_running() {
            bail!("Tailscale is {state}, not running: run `tailscale up` first");
        }
        match Self::route(port, &target)? {
            Route::Free => {
                let argv = [
                    "tailscale",
                    "serve",
                    "--bg",
                    &format!("--https={port}"),
                    &target,
                ]
                .map(str::to_owned)
                .to_vec();
                let serve = Expanded {
                    argv,
                    env: Vec::new(),
                };
                if !Self::status(&serve, Path::new("/"), None)? {
                    bail!("tailscale serve did not publish port {port}");
                }
            }
            Route::Target => {}
            route @ (Route::Funnel { .. } | Route::Other { .. } | Route::TcpForward) => {
                bail!("Tailscale Serve port {port} is in use ({route}); it is not replaced")
            }
        }
        let dns = node
            .dns_name()
            .context("Tailscale reports no MagicDNS name for this node")?;
        let origin = format!("https://{dns}:{port}");
        origin
            .parse()
            .with_context(|| format!("{origin} is not an origin a work item keeps"))
    }

    /// Removes the Serve route on `port` only while it still proxies to
    /// the preview's loopback port; any other route is left.
    fn unpublish(port: u16) -> Result<()> {
        let target = format!("http://127.0.0.1:{port}");
        match Self::route(port, &target)? {
            Route::Target => {
                let argv = ["tailscale", "serve", &format!("--https={port}"), "off"]
                    .map(str::to_owned)
                    .to_vec();
                Self::quiet(&argv).map(drop)
            }
            Route::Free | Route::Funnel { .. } | Route::Other { .. } | Route::TcpForward => Ok(()),
        }
    }

    /// How Serve routes `port` now.
    fn route(port: u16, target: &str) -> Result<Route> {
        let status =
            ServeStatus::parse(&Self::output(&["tailscale", "serve", "status", "--json"])?)?;
        Ok(status.route(port, target))
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the one place the CLI runs a project's declared steps"
    )]
    fn quiet(argv: &[String]) -> Result<bool> {
        let [program, args @ ..] = argv else {
            bail!("an empty command");
        };
        Ok(std::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false))
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the one place the CLI runs a project's declared steps"
    )]
    fn output(argv: &[&str]) -> Result<String> {
        let [program, args @ ..] = argv else {
            bail!("an empty command");
        };
        let output = std::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("cannot run {program}"))?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::ProjectPlace;
    use ren_harness::host::HostEnv;
    use std::path::{Path, PathBuf};

    #[test]
    fn a_worktree_is_the_place_its_runs_are_told() {
        let env = HostEnv::from_pairs([
            ("REN_WORKTREE", "/data/worktrees/acme-web-2"),
            ("REN_WORKTREE_PORTS", "1-2"),
            ("REN_WORKTREE_DATA", "/home/op"),
        ]);
        let place = ProjectPlace::read(&env, None).unwrap();
        // The slot's own ports and data, whatever else the variables say.
        assert_eq!(place.place.ports, Some((20_100, 20_199)));
        assert_eq!(
            place.place.data,
            Path::new("/data/worktrees/.data/acme-web-2")
        );
        assert_eq!(
            place.worktree().unwrap().preview_unit(),
            "ren-preview-acme-web-2-serve"
        );
        // A folder given, or no worktree, has no ports.
        let place = ProjectPlace::read(&env, Some(PathBuf::from("/src/acme"))).unwrap();
        assert_eq!(place.place.ports, None);
        assert_eq!(place.place.worktree, Path::new("/src/acme"));
        assert!(place.worktree().is_err());
    }
}
