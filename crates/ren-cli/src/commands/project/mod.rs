//! `ren project`: a project's declared contract (`ren-project.toml`) at
//! work on this host: its checks (`check`), and a preview of a workspace's
//! branch (`preview up`, `preview down`), as Ren's own (`cargo xtask
//! preview`) runs for Ren.
//!
//! A preview builds the workspace, starts its serve step as the transient
//! unit `ren-preview-<workspace>-serve` on the workspace's ports, waits for
//! its health path, and records it on each sent work item the branch's
//! commits name (`Work-Item: <id>`), or that `--work` gives, so the Work
//! view opens the marked page on it. Removing the workspace stops it.
//!
//! This module is the one place the CLI runs a project's declared steps.

mod canary;

use self::canary::ProjectFactory;
use super::history::TargetArgs;
use crate::{context::Context, management::Management};
use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use ren_gateway::api::{
    control::request::{ControlRequest, PreviewWorkRequest, WorkChange},
    read::{ReadBody, work::WorkPreviewBody},
};
use ren_git::Git;
use ren_harness::host::HostEnv;
use ren_project::{Expanded, Place, ProjectContract};
use ren_protocol::label::Stored;
use ren_recorder::{
    id::WorkItemId,
    work::{Landing, PreviewUrl, WorkStateKind},
};
use ren_release::{Revision, Tree};
use ren_store::{
    query::{PageLimit, Shelf},
    read::ReadRequest,
};
use ren_supervisor::Workspace;
use std::{
    path::{Path, PathBuf},
    process::{ExitCode, Stdio},
    time::Duration,
};

/// Run a project's declared checks, or preview a workspace's branch.
#[derive(Args)]
pub struct ProjectCmd {
    #[command(subcommand)]
    action: ProjectAction,
}

#[derive(Subcommand)]
enum ProjectAction {
    /// Run the checks `ren-project.toml` declares, in order; fails when one
    /// does.
    Check(CheckArgs),
    /// A preview of this workspace's branch.
    Preview {
        #[command(subcommand)]
        action: PreviewAction,
    },
    /// One canary pass: fetch the project's main, build it, serve it,
    /// check it, and deploy it if the operator's policy says so.
    Canary(CanaryArgs),
    /// Deploy the canary's checked build now, whatever the policy says.
    Promote(DirArgs),
    /// Where the project's release stands.
    Release(DirArgs),
}

#[derive(Args)]
struct CanaryArgs {
    #[command(flatten)]
    dir: DirArgs,
    #[arg(long, default_value = "origin")]
    remote: String,
    #[arg(long, default_value = "main")]
    branch: String,
}

#[derive(Args)]
struct DirArgs {
    /// The project's repository; by default the current folder.
    #[arg(long)]
    dir: Option<PathBuf>,
}

#[derive(Args)]
struct CheckArgs {
    /// The project's folder; by default the workspace this runs in, else
    /// the current folder.
    #[arg(long)]
    dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum PreviewAction {
    /// Build this workspace, serve it on its ports, and record it on the
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

/// Where a project's steps run, and the workspace it is, when it is one.
struct ProjectPlace {
    place: Place,
    workspace: Option<Workspace>,
}

/// The one place the CLI runs a project's steps and its preview's unit.
struct Steps;

impl ProjectCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        match self.action {
            ProjectAction::Check(args) => Self::check(ctx, args),
            ProjectAction::Preview { action } => match action {
                PreviewAction::Up(args) => Self::up(ctx, args).await,
                PreviewAction::Down(args) => Self::down(ctx, args).await,
            },
            ProjectAction::Canary(args) => {
                ProjectFactory::new(ctx, args.dir.dir)?
                    .pass(&args.remote, &args.branch)
                    .await
            }
            ProjectAction::Promote(args) => ProjectFactory::new(ctx, args.dir)?.promote(),
            ProjectAction::Release(args) => ProjectFactory::new(ctx, args.dir)?.show(),
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
        let contract = Self::contract(&place.place.workspace)?;
        if contract.checks().is_empty() {
            println!("{} declares no checks.", ProjectContract::FILE);
            return Ok(ExitCode::SUCCESS);
        }
        let mut failed = Vec::new();
        for check in contract.checks() {
            println!("==> {}", check.name());
            let step = check.step().expand(&place.place)?;
            if !Steps::run(&step, &place.place.workspace)? {
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
        let workspace = place.workspace()?;
        let contract = Self::contract(&place.place.workspace)?;
        let serve = contract.serve().with_context(|| {
            format!(
                "{} declares no [serve] step to preview",
                ProjectContract::FILE
            )
        })?;
        for step in contract.build() {
            let step = step.expand(&place.place)?;
            println!("==> {}", step.argv.join(" "));
            if !Steps::run(&step, &place.place.workspace)? {
                bail!("a build step failed; the preview is not started");
            }
        }
        let unit = Self::unit(workspace);
        Steps::stop(&unit)?;
        let step = serve.step().expand(&place.place)?;
        Steps::start(&unit, &step, &place.place.workspace)?;
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
        let revision = Self::revision(&place.place.workspace)?;
        let items = Self::addressed(ctx, &args, &place.place.workspace).await?;
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
                "Recorded on {count} work item(s); the Work view opens it beside production."
            ),
        }
        Ok(ExitCode::SUCCESS)
    }

    async fn down(ctx: &Context, args: PreviewArgs) -> Result<ExitCode> {
        let place = ProjectPlace::read(&ctx.env, None)?;
        let workspace = place.workspace()?;
        let contract = Self::contract(&place.place.workspace)?;
        if let Some(quiesce) = contract.quiesce() {
            let step = quiesce.expand(&place.place)?;
            println!("==> {}", step.argv.join(" "));
            Steps::run(&step, &place.place.workspace)?;
        }
        Steps::stop(&Self::unit(workspace))?;
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
        for item in Self::addressed(ctx, &args, &place.place.workspace).await? {
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

    /// The unit the workspace's preview runs as: the one removing the
    /// workspace stops.
    pub(super) fn unit(workspace: &Workspace) -> String {
        let [_, _, serve] = workspace.preview_units();
        serve
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

    /// The build the workspace's checkout is: its commit, modified or not.
    fn revision(dir: &Path) -> Result<Revision> {
        let git = Git::inherited();
        let commit = git.head_commit(dir)?;
        let tree = match git.tree(dir)? {
            ren_git::Tree::Clean => Tree::Clean,
            ren_git::Tree::Modified => Tree::Modified,
        };
        Ok(Revision::Commit {
            commit: commit
                .as_str()
                .parse()
                .with_context(|| format!("{commit} is not a commit"))?,
            tree,
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
        let mut named = args.work.clone();
        for (commit, message) in git.log(dir, &head, 200)? {
            let Ok(commit) = commit.as_str().parse() else {
                continue;
            };
            for item in Landing::read(commit, &message).items {
                if !named.contains(&item) {
                    named.push(item);
                }
            }
        }
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
    /// The workspace this runs in, from what Ren tells its runs
    /// (`REN_WORKSPACE`, `REN_WORKSPACE_PORTS`, `REN_WORKSPACE_DATA`); else
    /// `dir` (or the current folder), with no ports.
    fn read(env: &HostEnv, dir: Option<PathBuf>) -> Result<Self> {
        let folder = env.absolute_path("REN_WORKSPACE");
        let data = env.absolute_path("REN_WORKSPACE_DATA");
        let ports = env
            .get("REN_WORKSPACE_PORTS")
            .map(Self::ports)
            .transpose()?;
        match (dir, folder, data, ports) {
            (None, Some(folder), Some(data), Some(ports)) => {
                let workspace = folder
                    .parent()
                    .and_then(|root| Workspace::of(root, &folder));
                Ok(Self {
                    place: Place {
                        workspace: folder,
                        data,
                        ports: Some(ports),
                    },
                    workspace,
                })
            }
            (dir, _, _, _) => {
                let dir = match dir {
                    Some(dir) => dir,
                    None => std::env::current_dir()?,
                };
                Ok(Self {
                    place: Place {
                        data: dir.join(".ren-data"),
                        workspace: dir,
                        ports: None,
                    },
                    workspace: None,
                })
            }
        }
    }

    fn ports(text: &str) -> Result<(u16, u16)> {
        let (first, last) = text
            .split_once('-')
            .with_context(|| format!("REN_WORKSPACE_PORTS {text:?} is not `first-last`"))?;
        Ok((first.trim().parse()?, last.trim().parse()?))
    }

    fn workspace(&self) -> Result<&Workspace> {
        self.workspace.as_ref().context(
            "a preview runs in a workspace Ren made, whose runs are told REN_WORKSPACE, REN_WORKSPACE_PORTS, and REN_WORKSPACE_DATA",
        )
    }
}

impl Steps {
    /// Runs `step` in `dir`, in the foreground; whether it succeeded.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place the CLI runs a project's declared steps"
    )]
    pub(super) fn run(step: &Expanded, dir: &Path) -> Result<bool> {
        let [program, args @ ..] = step.argv.as_slice() else {
            bail!("an empty command");
        };
        let status = std::process::Command::new(program)
            .args(args)
            .envs(step.env.iter().map(|(name, value)| (name, value)))
            .current_dir(dir)
            .stdin(Stdio::null())
            .status()
            .with_context(|| format!("cannot start {program}"))?;
        Ok(status.success())
    }

    /// Starts `step` as the transient user unit `unit`, in `dir`.
    pub(super) fn start(unit: &str, step: &Expanded, dir: &Path) -> Result<()> {
        let mut argv = vec![
            "systemd-run".to_owned(),
            "--user".to_owned(),
            "--collect".to_owned(),
            "--quiet".to_owned(),
            format!("--unit={unit}"),
            format!("--working-directory={}", dir.display()),
        ];
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
        if Self::run(&start, dir)? {
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
    /// returns its origin there.
    fn publish(port: u16) -> Result<PreviewUrl> {
        let target = format!("http://127.0.0.1:{port}");
        let argv = [
            "tailscale",
            "serve",
            "--bg",
            &format!("--https={port}"),
            &target,
        ]
        .map(str::to_owned)
        .to_vec();
        if !Self::run(
            &Expanded {
                argv,
                env: Vec::new(),
            },
            Path::new("/"),
        )? {
            bail!("tailscale serve did not publish port {port}");
        }
        let status = Self::output(&["tailscale", "status", "--json"])?;
        let status: serde_json::Value = serde_json::from_str(&status)?;
        let dns = status["Self"]["DNSName"]
            .as_str()
            .map(|name| name.trim_end_matches('.'))
            .filter(|name| !name.is_empty())
            .context("Tailscale reports no MagicDNS name for this node")?;
        let origin = format!("https://{dns}:{port}");
        origin
            .parse()
            .with_context(|| format!("{origin} is not an origin a work item keeps"))
    }

    fn unpublish(port: u16) -> Result<()> {
        let argv = ["tailscale", "serve", &format!("--https={port}"), "off"]
            .map(str::to_owned)
            .to_vec();
        Self::quiet(&argv).map(drop)
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
    fn a_workspace_is_the_place_its_runs_are_told() {
        let env = HostEnv::from_pairs([
            ("REN_WORKSPACE", "/data/worktrees/acme-web-2"),
            ("REN_WORKSPACE_PORTS", "20100-20199"),
            ("REN_WORKSPACE_DATA", "/data/worktrees/.data/acme-web-2"),
        ]);
        let place = ProjectPlace::read(&env, None).unwrap();
        assert_eq!(place.place.ports, Some((20_100, 20_199)));
        assert_eq!(
            place.workspace().unwrap().preview_units()[2],
            "ren-preview-acme-web-2-serve"
        );
        // A folder given, or no workspace, has no ports.
        let place = ProjectPlace::read(&env, Some(PathBuf::from("/src/acme"))).unwrap();
        assert_eq!(place.place.ports, None);
        assert_eq!(place.place.workspace, Path::new("/src/acme"));
        assert!(place.workspace().is_err());
    }
}
