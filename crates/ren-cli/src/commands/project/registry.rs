//! The projects this host's daemon has registered (docs/plans/hierarchy.md):
//! `ren project add`, `list`, `archive`, and `unarchive`, and the project a
//! canary, promotion, or release command is for.
//!
//! A registration goes through the control API, which records it; the
//! registry is read from the store directly, beside the running daemon,
//! until the read API serves projects. A registration is recorded after it
//! is answered, so `add` waits for it to be stored and checks that it is
//! the project it registered. Adding a project already registered just so
//! does nothing.

use super::super::history::TargetArgs;
use crate::{commands::control::ControlTarget, context::Context};
use anyhow::{Context as _, Result, bail};
use clap::Args;
use ren_gateway::api::control::{
    projects::{ProjectAnswer, RegisterProjectRequest},
    request::ControlRequest,
};
use ren_git::{Git, Repository};
use ren_protocol::label::Stored;
use ren_recorder::project::{Factory, ProjectName, RepositoryPath};
use ren_store::view::ProjectView;
use std::{path::PathBuf, process::ExitCode, time::Duration};

#[derive(Args)]
pub(super) struct AddArgs {
    /// The repository; by default the current folder.
    #[arg(long)]
    dir: Option<PathBuf>,
    /// Its name; by default its folder's.
    #[arg(long)]
    name: Option<ProjectName>,
    /// How it is built and released: `contract` (its `.ren/project.toml`)
    /// or `ren` (Ren itself).
    #[arg(long, default_value = "contract")]
    factory: Factory,
    #[command(flatten)]
    target: TargetArgs,
}

#[derive(Args)]
pub(super) struct NameArgs {
    name: ProjectName,
    #[command(flatten)]
    target: TargetArgs,
}

/// Which registered project a command is for.
#[derive(Args)]
pub(super) struct ProjectArgs {
    /// The project's name; by default the one whose repository is the
    /// current folder.
    #[arg(long)]
    project: Option<ProjectName>,
}

/// The registered projects, as the store holds them.
pub(super) struct Registry;

impl Registry {
    pub(super) async fn add(ctx: &Context, args: AddArgs) -> Result<ExitCode> {
        let dir = match args.dir {
            Some(dir) => dir,
            None => std::env::current_dir()?,
        };
        let dir =
            std::fs::canonicalize(&dir).with_context(|| format!("no folder {}", dir.display()))?;
        match Git::repository(&dir) {
            Repository::At(_) => {}
            Repository::None | Repository::Unreadable => {
                bail!("{} is not a git repository", dir.display())
            }
        }
        let name = match args.name {
            Some(name) => name,
            None => ProjectName::of_folder(&dir).with_context(|| {
                format!(
                    "{}'s folder name is not a project name; pass --name",
                    dir.display()
                )
            })?,
        };
        let repository = RepositoryPath::of(&dir)?;
        // The same project registered again is already done, so a script
        // (install-vps) can register on every run.
        let same = Self::read(ctx).await?.into_iter().find(|project| {
            project.name == name
                && project.repository == repository
                && project.factory == Stored::Known(args.factory)
        });
        if let Some(same) = same {
            let archived = if same.archived_at.is_some() {
                ", archived"
            } else {
                ""
            };
            println!(
                "{name} is registered: {repository} ({}{archived})",
                args.factory
            );
            return Ok(ExitCode::SUCCESS);
        }
        let target = ControlTarget::select(&args.target, ctx)?;
        let answer = target
            .send(&ControlRequest::RegisterProject(RegisterProjectRequest {
                name: name.to_string(),
                repository: repository.to_string(),
                factory: args.factory.to_string(),
            }))
            .await?;
        if !answer.status.is_success() {
            return answer.report().map_err(Into::into);
        }
        let ProjectAnswer { id, .. } = serde_json::from_value(answer.body)
            .context("the daemon answered a registration with something else")?;
        let stored = Self::settled(ctx, &name).await?;
        if stored.id != id {
            bail!(
                "{name} was registered meanwhile as another project, at {}",
                stored.repository
            );
        }
        println!("Registered {name}: {repository} ({})", args.factory);
        let reader = ctx.reader().await?;
        let unowned = reader
            .unassigned_sessions()
            .await?
            .iter()
            .filter(|(_, workdir)| repository.holds(workdir))
            .count();
        reader.close().await;
        if unowned > 0 {
            println!(
                "{unowned} session(s) in it work for no project: `ren project adopt {name}` assigns them."
            );
        }
        Ok(ExitCode::SUCCESS)
    }

    /// Assign `args.name` the sessions no project owns in its repository
    /// or its workspaces.
    pub(super) async fn adopt(ctx: &Context, args: NameArgs) -> Result<ExitCode> {
        let target = ControlTarget::select(&args.target, ctx)?;
        Ok(target
            .send(&ControlRequest::AdoptSessions(args.name))
            .await?
            .report()?)
    }

    pub(super) async fn list(ctx: &Context) -> Result<ExitCode> {
        let projects = Self::read(ctx).await?;
        if projects.is_empty() {
            println!("No project is registered: `ren project add` registers one.");
            return Ok(ExitCode::SUCCESS);
        }
        let width = projects
            .iter()
            .map(|project| project.name.as_str().len())
            .max()
            .unwrap_or(0);
        for project in &projects {
            let factory = project.factory.label();
            let archived = if project.archived_at.is_some() {
                "  archived"
            } else {
                ""
            };
            println!(
                "{:width$}  {factory:8}  {}{archived}",
                project.name.as_str(),
                project.repository
            );
        }
        Ok(ExitCode::SUCCESS)
    }

    pub(super) async fn archive(ctx: &Context, args: NameArgs) -> Result<ExitCode> {
        let target = ControlTarget::select(&args.target, ctx)?;
        Ok(target
            .send(&ControlRequest::ArchiveProject(args.name))
            .await?
            .report()?)
    }

    pub(super) async fn unarchive(ctx: &Context, args: NameArgs) -> Result<ExitCode> {
        let target = ControlTarget::select(&args.target, ctx)?;
        Ok(target
            .send(&ControlRequest::UnarchiveProject(args.name))
            .await?
            .report()?)
    }

    /// The project `args` names, or the one whose repository is the current
    /// folder. An archived project is not built.
    pub(super) async fn resolve(ctx: &Context, args: &ProjectArgs) -> Result<ProjectView> {
        let reader = ctx.reader().await?;
        let found = match &args.project {
            Some(name) => reader
                .project_named(name)
                .await?
                .with_context(|| format!("no project is named {name}: see `ren project list`"))?,
            None => {
                let here = std::fs::canonicalize(std::env::current_dir()?)?;
                let repository = RepositoryPath::of(&here)?;
                reader.project_at(&repository).await?.with_context(|| {
                    format!(
                        "{repository} is no registered project's repository: pass --project, or `ren project add`"
                    )
                })?
            }
        };
        reader.close().await;
        if found.archived_at.is_some() {
            bail!(
                "{} is archived: `ren project unarchive {}` first",
                found.name,
                found.name
            );
        }
        Ok(found)
    }

    async fn read(ctx: &Context) -> Result<Vec<ProjectView>> {
        let reader = ctx.reader().await?;
        let projects = reader.projects().await?;
        reader.close().await;
        Ok(projects)
    }

    /// The project named `name` once the daemon has stored it.
    async fn settled(ctx: &Context, name: &ProjectName) -> Result<ProjectView> {
        for _ in 0..100 {
            if let Some(project) = Self::read(ctx)
                .await?
                .into_iter()
                .find(|project| &project.name == name)
            {
                return Ok(project);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        bail!("{name} was accepted, but is not stored after five seconds")
    }
}
