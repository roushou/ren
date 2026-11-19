//! `ren release`: where the release stands, as a gateway's `/api/release`
//! reports what the factory (`cargo xtask canary`) last found: `main`'s tip,
//! the build production runs and how far behind it is, the canary's build
//! and its checks, and what promotion decided.

use super::history::TargetArgs;
use crate::context::Context;
use anyhow::Result;
use clap::Args;
use ren_gateway::api::release::{ReleaseBody, ReleaseView};
use ren_protocol::label::Stored;
use ren_release::{Decision, Policy, Revision, Tree};
use std::{fmt::Write as _, process::ExitCode};

/// Show where the release stands: main, production, and the canary.
#[derive(Args)]
pub struct ReleaseCmd {
    /// Print the API's JSON instead.
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    target: TargetArgs,
}

impl ReleaseCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let body = self.target.gateway(ctx)?.release().await?;
        if self.json {
            println!("{}", serde_json::to_string_pretty(&body)?);
        } else {
            print!("{}", Self::render(&body));
        }
        Ok(ExitCode::SUCCESS)
    }

    /// The release as lines of text.
    pub(crate) fn render(body: &ReleaseBody) -> String {
        let Some(release) = &body.release else {
            return "No release yet: the factory (`cargo xtask canary`) has not run against this gateway.\n".to_owned();
        };
        let mut text = String::new();
        let _ = writeln!(text, "main        {}", release.main.short());
        let production = match &release.production {
            Revision::Commit {
                commit,
                tree: Tree::Clean,
            } => commit.short().to_owned(),
            Revision::Commit {
                commit,
                tree: Tree::Modified,
            } => format!("{}, modified", commit.short()),
            Revision::Unknown => "unknown".to_owned(),
        };
        let behind = match release.behind {
            Some(0) => "up to date with main".to_owned(),
            Some(1) => "1 commit behind main".to_owned(),
            Some(count) => format!("{count} commits behind main"),
            None => "not counted against main".to_owned(),
        };
        let _ = writeln!(text, "production  {production} ({behind})");
        match &release.canary {
            Some(canary) => {
                let checks: Vec<String> = canary
                    .checks
                    .iter()
                    .map(|check| {
                        let named = check
                            .name
                            .clone()
                            .unwrap_or_else(|| check.check.to_string());
                        match (&check.detail, check.passed) {
                            (_, true) => format!("{named} passed"),
                            (Some(detail), false) => format!("{named} failed: {detail}"),
                            (None, false) => format!("{named} failed"),
                        }
                    })
                    .collect();
                let verdict = if canary.passed { "passed" } else { "failed" };
                let _ = writeln!(
                    text,
                    "canary      {} {verdict} ({})",
                    canary.commit.short(),
                    checks.join(", ")
                );
            }
            None => {
                let _ = writeln!(text, "canary      none checked");
            }
        }
        let _ = writeln!(text, "promotion   {}", Self::promotion(release));
        text
    }

    /// The policy and what it decided, as a sentence.
    fn promotion(release: &ReleaseView) -> String {
        let policy = match release.policy {
            Policy::Manual => "manual".to_owned(),
            Policy::Automatic => "automatic".to_owned(),
            Policy::Window(window) => format!("in the window {window} UTC"),
        };
        let decision = match &release.decision {
            Stored::Known(Decision::Promote) => "production takes the canary's build now",
            Stored::Known(Decision::Current) => "production runs the canary's build",
            Stored::Known(Decision::Failed) => {
                "the canary's build failed; production keeps its own"
            }
            Stored::Known(Decision::Unchecked) => "no build has been checked",
            Stored::Known(Decision::ProductionAhead) => {
                "production has commits the canary's build lacks; it is not taken"
            }
            Stored::Known(Decision::Unconfirmed) => {
                "production runs a build the factory cannot place; promote it by hand"
            }
            Stored::Known(Decision::AwaitingOperator) => {
                "passed; promote it with `cargo xtask canary promote`"
            }
            Stored::Known(Decision::OutsideWindow) => {
                "passed; production takes it when the window opens"
            }
            Stored::Known(Decision::PromotionFailed) => {
                "production failed to take it and was rolled back; promote it by hand to try again"
            }
            Stored::Unknown(label) => label.as_str(),
        };
        format!("{policy}: {decision}")
    }
}

#[cfg(test)]
mod tests {
    use super::ReleaseCmd;
    use ren_gateway::api::release::ReleaseBody;

    #[test]
    fn a_release_reads_as_lines() {
        let body: ReleaseBody = serde_json::from_str(include_str!(
            "../../../../ui/apps/web/tests/fixtures/api/release.json"
        ))
        .unwrap();
        assert_eq!(
            ReleaseCmd::render(&body),
            "main        62e9e9e00000\n\
             production  07f4bb7c1e2a (1 commit behind main)\n\
             canary      62e9e9e00000 passed (doctor passed, reads passed, pages passed)\n\
             promotion   manual: passed; promote it with `cargo xtask canary promote`\n"
        );
        let none: ReleaseBody = serde_json::from_str(r#"{ "release": null }"#).unwrap();
        assert!(ReleaseCmd::render(&none).starts_with("No release yet"));
    }
}
