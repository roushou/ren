//! `cargo xtask preview`: this workspace's branch, built and run on a copy
//! of production's history, so its change can be judged on the page it was
//! marked on, beside production's (docs/plans/self-build.md, phase 5).
//!
//! Run in a workspace Ren made: its runs are told the workspace
//! (`REN_WORKSPACE`), its ports (`REN_WORKSPACE_PORTS`), and its data folder
//! (`REN_WORKSPACE_DATA`). The preview's daemon takes the slot's first port
//! and its UI the next, as `ren-preview-<workspace>-*` units; its build, data,
//! and configuration live in `<data>/preview`. Each sent work item the
//! branch's commits name (`Work-Item: <id>`), and each one given, records the
//! preview, so the Work view opens the marked page on it. Removing the
//! workspace stops it.

use crate::{
    file::{FileError, FileMode},
    install::publish::{Publish, PublishError},
    invocation::{Invocation, InvocationError},
    progress::Progress,
    replica::{Production, ProductionPlace, Replica, ReplicaError},
    tailscale::TailscaleCli,
    ui::{UiBuild, UiBuildError},
};
use clap::{Args, Subcommand};
use ren_gateway::api::{
    control::request::{PreviewWorkRequest, WorkChange},
    read::work::WorkPreviewBody,
};
use ren_git::{Git, GitError};
use ren_harness::host::HostEnv;
use ren_protocol::label::Stored;
use ren_recorder::{
    id::WorkItemId,
    work::{Landing, PreviewUrl, WorkStateKind},
};
use ren_release::Revision;
use ren_supervisor::Workspace;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// This workspace's branch, on a copy of production's history.
#[derive(Args)]
pub struct Preview {
    #[command(subcommand)]
    action: PreviewAction,
}

#[derive(Subcommand)]
enum PreviewAction {
    /// Build this branch, start it on a fresh copy of production's history,
    /// and record it on the work items it addresses.
    Up(UpArgs),
    /// Stop it, and take it off the work items it addresses.
    Down(DownArgs),
}

#[derive(Args)]
struct UpArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Publish it on the tailnet, on the slot's first two ports, with the
    /// install's saved Tailscale choice.
    #[arg(long)]
    publish: bool,
    /// Work items to record it on, besides those the branch's commits name.
    #[arg(long = "work", value_name = "ID")]
    work: Vec<WorkItemId>,
}

#[derive(Args)]
struct DownArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Remove its tailnet routes too.
    #[arg(long)]
    publish: bool,
    #[arg(long = "work", value_name = "ID")]
    work: Vec<WorkItemId>,
}

#[derive(Args, Clone, Copy)]
struct TargetArgs {
    /// The instance whose history it copies, and whose work items it
    /// records itself on.
    #[arg(long, value_enum, default_value_t = Production::Installed)]
    of: Production,
}

#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    #[error(
        "{0} is not set: run `cargo xtask preview` in a workspace Ren made, whose runs are told it"
    )]
    NotAWorkspace(&'static str),
    #[error("{0} is not a workspace Ren made (`<worktrees>/<repo>-<slot>`)")]
    NotRens(String),
    #[error("the built ren reports no revision: {0:?}")]
    Revision(String),
    #[error("the preview's address is not one a work item keeps: {0}")]
    Url(String),
    #[error(transparent)]
    Replica(#[from] ReplicaError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Invocation(#[from] InvocationError),
    #[error(transparent)]
    File(#[from] FileError),
    #[error(transparent)]
    Ui(#[from] UiBuildError),
    #[error(transparent)]
    Publish(#[from] PublishError),
}

/// The workspace, as Ren made it: read from `REN_WORKSPACE` alone, and
/// checked to be a folder of Ren's worktrees named as a workspace. Its data
/// folder and ports are the ones Ren gives that slot, never taken from
/// other variables, so a preview only ever replaces its own data.
struct PreviewPlace {
    workspace: Workspace,
}

impl PreviewPlace {
    /// The workspace Ren told this run it works in.
    fn read(env: &HostEnv) -> Result<Self, PreviewError> {
        let folder = env
            .absolute_path("REN_WORKSPACE")
            .ok_or(PreviewError::NotAWorkspace("REN_WORKSPACE"))?;
        let workspace = folder
            .parent()
            .and_then(|root| Workspace::of(root, &folder))
            .ok_or_else(|| PreviewError::NotRens(folder.display().to_string()))?;
        Ok(Self { workspace })
    }

    fn name(&self) -> String {
        self.workspace
            .folder()
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_default()
    }

    fn replica(&self) -> Replica {
        Replica::preview(&self.name(), self.ports().0, &self.workspace.data())
    }

    fn root(&self) -> PathBuf {
        self.workspace.data().join("preview")
    }

    /// The slot's first two ports: its daemon's, then its UI's.
    fn ports(&self) -> (u16, u16) {
        let (first, _) = self.workspace.ports();
        (first, first.saturating_add(1))
    }
}

impl Preview {
    /// How far back the branch's history is read for trailers.
    const HISTORY: u32 = 200;

    pub fn run(self, env: &HostEnv) -> Result<(), PreviewError> {
        let workspace = PreviewPlace::read(env)?;
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        match self.action {
            PreviewAction::Up(args) => Self::up(env, &workspace, &repo, &args),
            PreviewAction::Down(args) => Self::down(env, &workspace, &repo, &args),
        }
    }

    fn up(
        env: &HostEnv,
        workspace: &PreviewPlace,
        repo: &Path,
        args: &UpArgs,
    ) -> Result<(), PreviewError> {
        let production = ProductionPlace::new(env, args.target.of)?;
        let replica = workspace.replica();
        let (ren, revision) = Self::build(env, workspace, repo)?;
        let backup = production.backup()?;
        replica.replace_data(&backup)?;
        let keys = replica.configure(&production)?;
        let publish = Self::tailscale(env, args.publish)?.map(|tailscale| {
            let (gateway, ui) = workspace.ports();
            (tailscale, gateway, ui)
        });
        let reached = replica.start(env, &ren, &workspace.root().join("ui"), &keys, publish)?;
        let ui: PreviewUrl = reached
            .ui
            .parse()
            .map_err(|_| PreviewError::Url(reached.ui.clone()))?;
        let items = Self::addressed(&production, repo, &args.work)?;
        for item in &items {
            production.move_work(
                *item,
                WorkChange::Preview(PreviewWorkRequest {
                    preview: Some(WorkPreviewBody {
                        ui: ui.clone(),
                        revision: revision.clone(),
                    }),
                }),
            )?;
        }
        println!("\nPreview of {revision}: {ui}");
        match items.len() {
            0 => println!(
                "No work item in a session is named yet: commit with a `Work-Item:` trailer, or pass --work."
            ),
            count => println!(
                "Recorded on {count} work item(s); the Work view opens it beside production."
            ),
        }
        Ok(())
    }

    fn down(
        env: &HostEnv,
        workspace: &PreviewPlace,
        repo: &Path,
        args: &DownArgs,
    ) -> Result<(), PreviewError> {
        let production = ProductionPlace::new(env, args.target.of)?;
        let replica = workspace.replica();
        replica.stop()?;
        if let Some(tailscale) = Self::tailscale(env, args.publish)? {
            replica.unpublish(tailscale, workspace.ports())?;
        }
        let listed = production.work_items()?.unwrap_or_default();
        for item in Self::addressed(&production, repo, &args.work)? {
            // Only this preview's, by the port its UI took.
            let previewed = listed.iter().any(|entry| {
                entry.id == item
                    && entry
                        .preview
                        .as_ref()
                        .is_some_and(|preview| preview.ui.port() == Some(workspace.ports().1))
            });
            if previewed {
                production.move_work(
                    item,
                    WorkChange::Preview(PreviewWorkRequest { preview: None }),
                )?;
            }
        }
        Progress("The preview is down").show();
        Ok(())
    }

    /// This checkout's `ren` and UI, built where the preview runs them from,
    /// and the revision the binary reports.
    fn build(
        env: &HostEnv,
        workspace: &PreviewPlace,
        repo: &Path,
    ) -> Result<(PathBuf, Revision), PreviewError> {
        let cargo = env
            .absolute_path("CARGO")
            .unwrap_or_else(|| PathBuf::from("cargo"));
        Progress("Building this branch for its preview").show();
        Invocation::new(&cargo)
            .args(["build", "--locked", "--package", "ren-rs", "--bin", "ren"])
            .arg("--manifest-path")
            .arg(repo.join("Cargo.toml"))
            .run()?;
        let root = workspace.root();
        fs::create_dir_all(root.join("bin")).map_err(FileError::at(&root))?;
        // Copied, so a later build never replaces the binary under the unit.
        let built = repo.join("target/debug/ren");
        let ren = root.join("bin/ren");
        let binary = fs::read(&built).map_err(FileError::at(&built))?;
        FileMode::Executable.write(&ren, &binary)?;
        let version = Invocation::new(&ren).arg("--version").capture()?;
        let revision = version
            .split_once('(')
            .and_then(|(_, rest)| rest.split_once(')'))
            .and_then(|(text, _)| text.parse::<Revision>().ok())
            .ok_or_else(|| PreviewError::Revision(version.trim().to_owned()))?;
        let next = root.join("ui.next");
        UiBuild::new(env, repo)?.build_into(&next)?;
        let ui = root.join("ui");
        if ui.exists() {
            fs::remove_dir_all(&ui).map_err(FileError::at(&ui))?;
        }
        fs::rename(&next, &ui).map_err(FileError::at(&ui))?;
        Ok((ren, revision))
    }

    /// The sent work items this branch addresses: those its recent commits
    /// name, and those given.
    fn addressed(
        production: &ProductionPlace,
        repo: &Path,
        given: &[WorkItemId],
    ) -> Result<Vec<WorkItemId>, PreviewError> {
        let Some(listed) = production.work_items()? else {
            return Ok(Vec::new());
        };
        let git = Git::inherited();
        let head = git.head_commit(repo)?;
        let mut named: Vec<WorkItemId> = given.to_vec();
        for (commit, message) in git.log(repo, &head, Self::HISTORY)? {
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

    /// The install's saved Tailscale choice, when publishing.
    fn tailscale(env: &HostEnv, publish: bool) -> Result<Option<TailscaleCli>, PreviewError> {
        if !publish {
            return Ok(None);
        }
        let home = env
            .absolute_path("HOME")
            .ok_or(PreviewError::NotAWorkspace("HOME"))?;
        let saved = Publish::default().resolve(&Publish::path(&home))?;
        Ok(Some(if saved.sudo_tailscale {
            TailscaleCli::Sudo
        } else {
            TailscaleCli::Direct
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{PreviewError, PreviewPlace};
    use ren_harness::host::HostEnv;
    use std::path::Path;

    #[test]
    fn a_preview_takes_its_workspace_from_where_ren_made_it() {
        // The data and ports variables are not read: the slot gives them.
        let env = HostEnv::from_pairs([
            ("REN_WORKSPACE", "/data/worktrees/ren-3"),
            ("REN_WORKSPACE_PORTS", "1-2"),
            ("REN_WORKSPACE_DATA", "/home/op"),
        ]);
        let place = PreviewPlace::read(&env).unwrap();
        assert_eq!(place.name(), "ren-3");
        assert_eq!(place.ports(), (20_200, 20_201));
        assert_eq!(
            place.root(),
            Path::new("/data/worktrees/.data/ren-3/preview")
        );
        for folder in ["/data/worktrees/notes", "/home/op"] {
            assert!(matches!(
                PreviewPlace::read(&HostEnv::from_pairs([("REN_WORKSPACE", folder)])),
                Err(PreviewError::NotRens(_))
            ));
        }
        assert!(matches!(
            PreviewPlace::read(&HostEnv::from_pairs([])),
            Err(PreviewError::NotAWorkspace("REN_WORKSPACE"))
        ));
    }
}
