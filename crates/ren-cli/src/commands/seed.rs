//! `ren seed`: write invented history into this host's storage, through the
//! recorder and the store, as the daemon would record the calls. The
//! history is the seed's ([`crate::seed`]); this only writes it.

use crate::context::Context;
use crate::seed::{Ending, History};
use anyhow::{Context as _, Result};
use clap::Args;
use ren_daemon::host::{RandomIds, SystemClock};
use ren_gateway::deployment::SeedNumber;
use ren_protocol::route::{Method, Transport};
use ren_protocol::{direction::Direction, encoding::ResponseEncoding, limits::LimitHeaders};
use ren_recorder::{
    AttemptStart, CallStart, Head, Recorder, RecorderSetup,
    capture::{AttemptTerminal, TerminalRecord},
    routing::{ConfigRevision, UpstreamId},
    secrets::SecretScanner,
    time::{Clock as _, ManualClock, UnixMillis},
};
use ren_store::{Store, StoreError};
use std::{process::ExitCode, sync::Arc, time::Duration};

/// Write invented history into this host's storage: sessions from each
/// client, with turns and tool calls, several callers, failures, and long
/// prompts, placed over the last three days. For a preview; the daemon
/// must be stopped.
#[derive(Args)]
pub struct SeedCmd {
    /// The number the history is made from: the same number always
    /// writes the same history.
    #[arg(long, default_value = "1")]
    seed: SeedNumber,
}

impl SeedCmd {
    pub async fn run(self, ctx: &Context) -> Result<ExitCode> {
        let config = ctx.config()?;
        let data_dir = ctx.data_dir(&config)?;
        let history = History::invent(self.seed);
        let end = SystemClock::start().now().wall;
        let clock = Arc::new(ManualClock::at(end));
        let ids = Arc::new(RandomIds);
        let (store, recovery) = match Store::open(&data_dir, clock.as_ref(), ids.clone()).await {
            Err(StoreError::Locked) => anyhow::bail!(
                "a daemon is using {}; stop it before seeding",
                data_dir.display()
            ),
            opened => opened.context("cannot open the store")?,
        };
        let (recorder, task) = Recorder::new(
            store,
            recovery.calls,
            RecorderSetup {
                options: config.capture.clone(),
                retention: config.retention.clone(),
                max_calls: 64,
                live_buffer: 64,
                scanner: Arc::new(SecretScanner::new()?),
            },
            clock.clone(),
            ids,
        )?;
        let writing = async move {
            for call in history.calls() {
                let ago = u64::try_from(call.ago.as_millis()).unwrap_or(u64::MAX);
                clock.set_wall(UnixMillis::from_millis(end.as_millis().saturating_sub(ago)));
                Self::record(&recorder, &clock, call);
                // The queue drops what does not fit: let it empty first.
                while recorder.metrics().queue.items > 0 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
            history.calls().len()
        };
        let (written, ended) = tokio::join!(writing, task.run());
        ended.context("the recorder failed")?;
        println!("Seeded {written} calls into {}", data_dir.display());
        Ok(ExitCode::SUCCESS)
    }

    /// Records one call as the gateway's relay would: its request, the
    /// answer's head, the answer, and how it ended.
    fn record(recorder: &Recorder, clock: &ManualClock, call: &crate::seed::InventedCall) {
        let provider = call.api.provider();
        let mut tap = recorder.start(
            CallStart {
                transport: Transport::Http,
                api: call.api,
                endpoint: call.endpoint,
                method: Method::Post,
                config_revision: ConfigRevision::new("seeded"),
                attribution: ren_harness::Harness::attribution(call.harness, &call.headers),
                caller: call.caller.clone(),
                operator_revision: None,
                plan: None,
                attempts: 1,
            },
            AttemptStart::unchanged(UpstreamId::default_for(provider), provider),
        );
        tap.request_data(&call.request);
        tap.request_ended();
        let mut response = tap.head(Head {
            status: call.answer.status,
            format: call.answer.format,
            encoding: ResponseEncoding::Identity,
            limits: LimitHeaders::NONE,
        });
        clock.advance(call.took);
        response.data(Direction::Response, &call.answer.body);
        let terminal = match call.answer.ending {
            Ending::Complete => {
                response.ended(Direction::Response);
                AttemptTerminal::BodyComplete
            }
            Ending::Cut => AttemptTerminal::StreamFailed,
        };
        response.finish(TerminalRecord {
            terminal,
            cause: None,
            phase: None,
        });
    }
}
