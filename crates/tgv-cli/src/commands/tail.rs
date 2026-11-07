//! `tgv tail`: follow the targeted daemon's live feed. Calls are printed as
//! they start and finish; `--json` prints every observation instead.
//!
//! After the feed ends (a daemon restart) or the connection drops, it
//! reconnects from the last observation it printed, so nothing in between
//! is skipped. It stops on interrupt, or when the first connection fails.
use crate::commands::history::TargetArgs;
use crate::context::Context;
use crate::management::Management;
use crate::style::{Styling, Tone};
use crate::tail::{CallSummary, ClockTime, LiveDecoder, LiveEvent, Shown};
use anyhow::Result;
use clap::Args;
use std::{process::ExitCode, time::Duration};
use tgv_recorder::Observation;
use tgv_store::read::ReadRequest;

/// Follow calls as they start and finish (`--json`: every observation).
#[derive(Args)]
pub struct TailCmd {
    /// Print every observation as one JSON object per line.
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    target: TargetArgs,
}

/// How a connection to the feed ended.
enum Ended {
    /// The daemon closed the feed, or the connection dropped.
    Closed,
    /// The position was pruned; resume from now.
    Gone,
}

impl TailCmd {
    const RECONNECT: Duration = Duration::from_secs(2);

    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let gateway = self.target.gateway(ctx)?;
        let stderr = ctx.terminal.stderr;
        let mut after = None;
        let mut response = gateway.live(after).await?;
        eprintln!(
            "{}",
            stderr.paint(
                Tone::Muted,
                &format!("Following {} (Ctrl-C to stop)", gateway.root())
            )
        );
        loop {
            let ended = self
                .follow(&gateway, &mut response, &mut after, stderr)
                .await?;
            let reason = match ended {
                Ended::Closed => "the feed ended; reconnecting",
                Ended::Gone => {
                    after = None;
                    "retention removed the last position; continuing from now"
                }
            };
            eprintln!("{}", stderr.paint(Tone::Warn, reason));
            response = loop {
                tokio::time::sleep(Self::RECONNECT).await;
                match gateway.live(after).await {
                    Ok(response) => break response,
                    Err(error) => {
                        eprintln!(
                            "{}",
                            stderr.paint(Tone::Muted, &format!("{error}; retrying"))
                        );
                    }
                }
            };
        }
    }

    /// Prints events until this connection ends, advancing `after`.
    async fn follow(
        &self,
        gateway: &Management,
        response: &mut reqwest::Response,
        after: &mut Option<u64>,
        stderr: Styling,
    ) -> Result<Ended> {
        let mut decoder = LiveDecoder::default();
        let mut events = Vec::new();
        loop {
            let chunk = match gateway.chunk(response).await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => return Ok(Ended::Closed),
                Err(error) => {
                    eprintln!("{}", stderr.paint(Tone::Muted, &error.to_string()));
                    return Ok(Ended::Closed);
                }
            };
            decoder.feed(&chunk, &mut events);
            for event in events.drain(..) {
                match event {
                    LiveEvent::Observation(observation) => {
                        *after = Some(observation.seq);
                        self.observation(gateway, &observation).await?;
                    }
                    LiveEvent::Lagged { resume_from } => eprintln!(
                        "{}",
                        stderr.paint(
                            Tone::Warn,
                            &format!("fell behind; catching up from {resume_from}")
                        )
                    ),
                    LiveEvent::Gone { .. } => return Ok(Ended::Gone),
                    LiveEvent::Failed(reason) => {
                        eprintln!(
                            "{}",
                            stderr.paint(Tone::Bad, &format!("feed failed: {reason}"))
                        );
                        return Ok(Ended::Closed);
                    }
                    LiveEvent::Unrecognized { name, detail } => eprintln!(
                        "{}",
                        stderr.paint(Tone::Muted, &format!("unrecognized event {name}: {detail}"))
                    ),
                }
            }
        }
    }

    async fn observation(&self, gateway: &Management, observation: &Observation) -> Result<()> {
        if self.json {
            println!("{}", serde_json::to_string(observation)?);
            return Ok(());
        }
        let at = observation.at;
        let line = match Shown::of(observation) {
            Ok(Shown::Hidden) => return Ok(()),
            Ok(Shown::Operator) => format!("{}  operator  state changed", ClockTime(at)),
            Ok(Shown::Started(call)) => match Self::summary(gateway, call).await {
                Some(summary) => summary.started(at),
                None => format!("{}  started   {call}", ClockTime(at)),
            },
            Ok(Shown::Ended(call, ending)) => match Self::summary(gateway, call).await {
                Some(summary) => summary.ended(at, ending),
                None => format!("{}  ended     {call}", ClockTime(at)),
            },
            Err(kind) => format!("{}  {kind}", ClockTime(at)),
        };
        println!("{line}");
        Ok(())
    }

    /// The call's view, if it can be read; a tail line never fails on it.
    async fn summary(gateway: &Management, call: tgv_recorder::id::CallId) -> Option<CallSummary> {
        let detail = gateway.serve(&ReadRequest::Call(call)).await.ok()?;
        serde_json::from_value(detail.get("call")?.clone()).ok()
    }
}
