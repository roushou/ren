//! `ren preview`: the factory's previews of Ren (docs/plans/self-build.md,
//! phase 9). `list` prints each preview in its slot and the commits asked
//! for, as a gateway's `/api/projects/{name}/previews` reports them; `ask`
//! and `withdraw` ask for a preview of a commit, or withdraw the ask,
//! through the control API, for the factory's next pass to start or stop.

use super::{control::ControlTarget, history::TargetArgs};
use crate::context::Context;
use anyhow::Result;
use clap::{Args, Subcommand};
use ren_gateway::api::{
    control::request::{ControlRequest, PreviewAskRequest},
    previews::{PreviewOfBody, PreviewStateBody, PreviewsBody},
};
use ren_gateway::preview::Reach;
use ren_git::Commit;
use ren_recorder::project::ProjectName;
use std::{fmt::Write as _, process::ExitCode};

/// Ren's previews: of each open pull request, and of each commit asked for.
#[derive(Args)]
pub struct PreviewCmd {
    #[command(subcommand)]
    action: PreviewAction,
}

#[derive(Subcommand)]
enum PreviewAction {
    /// Each preview in its slot, and the commits asked for.
    List(ListArgs),
    /// Ask for a preview of a commit; the factory's next pass starts it.
    Ask(AskArgs),
    /// Withdraw the ask; the factory's next pass stops its preview.
    Withdraw(AskArgs),
}

#[derive(Args)]
struct ListArgs {
    /// The project Ren is registered as.
    #[arg(long, default_value = "ren")]
    project: ProjectName,
    /// Print the API's JSON instead.
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    target: TargetArgs,
}

#[derive(Args)]
struct AskArgs {
    /// The commit, all 40 hex digits; it must be on GitHub, where the
    /// factory fetches it from.
    commit: Commit,
    #[command(flatten)]
    target: TargetArgs,
}

impl PreviewCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        match self.action {
            PreviewAction::List(args) => {
                let body = args.target.gateway(ctx)?.previews(&args.project).await?;
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&body)?);
                } else {
                    print!("{}", Self::render(&body));
                }
                Ok(ExitCode::SUCCESS)
            }
            PreviewAction::Ask(args) => {
                Self::send(
                    ctx,
                    &args.target,
                    ControlRequest::AskPreview(PreviewAskRequest {
                        commit: args.commit,
                    }),
                )
                .await
            }
            PreviewAction::Withdraw(args) => {
                Self::send(
                    ctx,
                    &args.target,
                    ControlRequest::WithdrawPreview(PreviewAskRequest {
                        commit: args.commit,
                    }),
                )
                .await
            }
        }
    }

    async fn send(ctx: &Context, target: &TargetArgs, request: ControlRequest) -> Result<ExitCode> {
        let answer = ControlTarget::select(target, ctx)?.send(&request).await?;
        Ok(answer.report()?)
    }

    /// The previews as lines of text.
    fn render(body: &PreviewsBody) -> String {
        let mut text = String::new();
        if body.previews.is_empty() {
            text.push_str("No preview runs.\n");
        }
        for preview in &body.previews {
            let of = match preview.of {
                PreviewOfBody::Pull { number } => format!("pull request #{number}"),
                PreviewOfBody::Commit => "asked for".to_owned(),
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
                "slot {}  {}  {of}  {state}",
                preview.slot,
                preview.commit.short()
            );
        }
        for commit in &body.requested {
            let _ = writeln!(text, "asked for {}", commit.short());
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::PreviewCmd;
    use ren_gateway::api::previews::{PreviewBody, PreviewOfBody, PreviewStateBody, PreviewsBody};
    use ren_gateway::preview::Reach;

    #[test]
    fn previews_read_as_lines() {
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
                    ui_port: 18_103,
                    published_ui_port: 9_453,
                },
            ],
            requested: vec![commit()],
        };
        assert_eq!(
            PreviewCmd::render(&body),
            "slot 0  222222222222  pull request #12  running, UI on loopback port 18101 and tailnet port 9451\n\
             slot 1  222222222222  asked for  failed: the build failed\n\
             asked for 222222222222\n"
        );
        assert_eq!(
            PreviewCmd::render(&PreviewsBody {
                previews: Vec::new(),
                requested: Vec::new()
            }),
            "No preview runs.\n"
        );
    }
}
