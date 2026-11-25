//! `ren deploy` and `ren deployments`: chosen deployments of Ren
//! (docs/plans/deployments.md, phase 4). `ren deploy <commit>` chooses a
//! commit to deploy, on a copy of production or on the history `--seed`
//! invents, recorded on each `--work` item once it runs; `ren deploy stop`
//! withdraws the choice. Both go through the control API, for the deployer
//! to start or stop: at once when it listens (`[deployer] socket`), else at
//! its next pass. `ren deployments` prints each deployment in its slot and
//! the commits chosen, as a gateway's `/api/projects/{name}/previews`
//! reports them.

use super::{control::ControlTarget, history::TargetArgs};
use crate::context::Context;
use anyhow::{Context as _, Result};
use clap::{Args, Subcommand};
use ren_gateway::api::{
    control::request::{ControlRequest, PreviewAskRequest, PreviewWithdrawRequest},
    previews::{PreviewOfBody, PreviewStateBody, PreviewsBody},
};
use ren_gateway::deployment::{Reach, SeedNumber};
use ren_git::{Commit, Git, Revision};
use ren_recorder::{id::WorkItemId, project::ProjectName};
use std::{fmt::Write as _, process::ExitCode};

/// Deploy a commit of Ren: on a copy of production's history, or on an
/// invented one; `ren deployments` shows where it stands.
#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct DeployCmd {
    #[command(subcommand)]
    action: Option<DeployAction>,
    #[command(flatten)]
    choose: ChooseArgs,
}

#[derive(Subcommand)]
enum DeployAction {
    /// Withdraw the choice of a commit; the deployer stops its deployment.
    Stop(StopArgs),
}

#[derive(Args)]
struct ChooseArgs {
    /// The commit: a revision of this folder's repository (`HEAD`), or all
    /// 40 hex digits of one on GitHub.
    commit: Option<String>,
    /// Seed it with the history this number invents (`ren seed`), not a
    /// copy of production's.
    #[arg(long, value_name = "NUMBER")]
    seed: Option<SeedNumber>,
    /// A work item whose change it previews: it is recorded on the item
    /// once it runs. Repeat for more.
    #[arg(long = "work", value_name = "ITEM")]
    work: Vec<WorkItemId>,
    #[command(flatten)]
    target: TargetArgs,
}

#[derive(Args)]
struct StopArgs {
    /// The commit, as `ren deploy` took it.
    commit: String,
    #[command(flatten)]
    target: TargetArgs,
}

/// Each deployment of Ren in its slot, and the commits chosen.
#[derive(Args)]
pub struct DeploymentsCmd {
    /// The project Ren is registered as.
    #[arg(long, default_value = "ren")]
    project: ProjectName,
    /// Print the API's JSON instead.
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    target: TargetArgs,
}

impl DeployCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        match self.action {
            Some(DeployAction::Stop(args)) => {
                let commit = Self::commit(&args.commit)?;
                Self::send(
                    ctx,
                    &args.target,
                    ControlRequest::WithdrawPreview(PreviewWithdrawRequest { commit }),
                )
                .await
            }
            None => {
                let args = self.choose;
                let text = args
                    .commit
                    .context("name the commit to deploy (`ren deploy HEAD`)")?;
                let commit = Self::commit(&text)?;
                Self::send(
                    ctx,
                    &args.target,
                    ControlRequest::AskPreview(PreviewAskRequest {
                        commit,
                        seed: args.seed,
                        work: args.work,
                    }),
                )
                .await
            }
        }
    }

    /// The commit `text` names: all 40 hex digits as they are, or a
    /// revision this folder's repository resolves.
    fn commit(text: &str) -> Result<Commit> {
        if let Ok(commit) = text.parse() {
            return Ok(commit);
        }
        let revision: Revision = text.parse()?;
        let here = std::env::current_dir()?;
        Ok(Git::inherited().commit_of(&here, &revision)?)
    }

    async fn send(ctx: &Context, target: &TargetArgs, request: ControlRequest) -> Result<ExitCode> {
        let answer = ControlTarget::select(target, ctx)?.send(&request).await?;
        Ok(answer.report()?)
    }
}

impl DeploymentsCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let body = self.target.gateway(ctx)?.previews(&self.project).await?;
        if self.json {
            println!("{}", serde_json::to_string_pretty(&body)?);
        } else {
            print!("{}", Self::render(&body));
        }
        Ok(ExitCode::SUCCESS)
    }

    /// The deployments as lines of text.
    fn render(body: &PreviewsBody) -> String {
        let mut text = String::new();
        if body.previews.is_empty() {
            text.push_str("No deployment runs.\n");
        }
        for preview in &body.previews {
            let of = match preview.of {
                PreviewOfBody::Pull { number } => format!("pull request #{number}"),
                PreviewOfBody::Commit => "chosen".to_owned(),
            };
            let data = match preview.seed {
                None => String::new(),
                Some(number) => format!(" on history {number}"),
            };
            let state = match &preview.state {
                PreviewStateBody::Building => "building".to_owned(),
                PreviewStateBody::Running { reach } => match reach {
                    Reach::Loopback => format!("running, UI on loopback port {}", preview.ui_port),
                    Reach::Tailnet => format!(
                        "running, UI on loopback port {} and tailnet port {}",
                        preview.ui_port, preview.published_ui_port
                    ),
                },
                PreviewStateBody::Failed { detail } => format!("failed: {detail}"),
            };
            let _ = writeln!(
                text,
                "slot {}  {}  {of}{data}  {state}",
                preview.slot,
                preview.commit.short()
            );
        }
        for commit in &body.requested {
            let _ = writeln!(text, "chosen {}", commit.short());
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::DeploymentsCmd;
    use ren_gateway::api::previews::{PreviewBody, PreviewOfBody, PreviewStateBody, PreviewsBody};
    use ren_gateway::deployment::{Reach, SeedNumber};

    #[test]
    fn deployments_read_as_lines() {
        let commit = || "2222222222222222222222222222222222222222".parse().unwrap();
        let body = PreviewsBody {
            previews: vec![
                PreviewBody {
                    slot: 0,
                    of: PreviewOfBody::Pull { number: 12 },
                    commit: commit(),
                    state: PreviewStateBody::Running {
                        reach: Reach::Tailnet,
                    },
                    seed: None,
                    work: Vec::new(),
                    ui_port: 18_101,
                    published_ui_port: 9_451,
                },
                PreviewBody {
                    slot: 1,
                    of: PreviewOfBody::Commit,
                    commit: commit(),
                    state: PreviewStateBody::Failed {
                        detail: "the build failed".into(),
                    },
                    seed: Some(SeedNumber::new(7)),
                    work: Vec::new(),
                    ui_port: 18_103,
                    published_ui_port: 9_453,
                },
            ],
            requested: vec![commit()],
        };
        assert_eq!(
            DeploymentsCmd::render(&body),
            "slot 0  222222222222  pull request #12  running, UI on loopback port 18101 and tailnet port 9451\n\
             slot 1  222222222222  chosen on history 7  failed: the build failed\n\
             chosen 222222222222\n"
        );
        assert_eq!(
            DeploymentsCmd::render(&PreviewsBody {
                previews: Vec::new(),
                requested: Vec::new()
            }),
            "No deployment runs.\n"
        );
    }
}
