//! The recorder with the real store: what it records is what the store reads
//! back. Synthetic payloads; no network.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use std::sync::Arc;
use tgv_harness::attribution::Attribution;
use tgv_protocol::{
    direction::Direction,
    encoding::{ResponseEncoding, ResponseFormat},
    inspect::{Limitation, ModelOutcome},
    label::Stored,
    route::{Endpoint, Method, Provider, Transport},
};
use tgv_recorder::routing::UpstreamId;
use tgv_recorder::{
    AttemptStart, CallStart, CallTap, CaptureOptions, Recorder, RecorderMetrics, RecorderSetup,
    RecorderTask,
};
use tgv_recorder::{
    batch::ObservationKind,
    capture::{AttemptTerminal, BodyEnding, CallState, TerminalRecord},
    id::{CallId, SequentialIds},
    retention::RetentionPolicy,
    routing::ConfigRevision,
    time::{ManualClock, UnixMillis},
};
use tgv_store::{Reader, Store, query::CallQuery, view::CallDetail};

/// A terminal without cause or phase, as tests of the recorder need no more.
fn ended(terminal: AttemptTerminal) -> TerminalRecord {
    TerminalRecord {
        terminal,
        cause: None,
        phase: None,
    }
}

const PIECE: usize = CaptureOptions::PIECE_BYTES;
const MESSAGE_STREAM: &[u8] = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-test\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

struct Harness {
    dir: tempfile::TempDir,
    recorder: Recorder,
    task: RecorderTask<Store>,
}

impl Harness {
    async fn new(options: CaptureOptions, max_calls: usize) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::at(UnixMillis::from_millis(5_000)));
        let ids = Arc::new(SequentialIds::default());
        let (store, recovery) = Store::open(dir.path(), clock.as_ref(), ids.clone())
            .await
            .unwrap();
        let setup = RecorderSetup {
            options,
            retention: RetentionPolicy::default(),
            max_calls,
            live_buffer: 64,
        };
        let (recorder, task) = Recorder::new(store, recovery.calls, setup, clock, ids).unwrap();
        Self {
            dir,
            recorder,
            task,
        }
    }

    fn start(&self) -> Arc<CallTap> {
        self.recorder.start(
            CallStart {
                transport: Transport::Http,
                provider: Provider::Anthropic,
                endpoint: Endpoint::Messages,
                method: Method::Post,
                config_revision: ConfigRevision::new("rev"),
                attribution: Attribution::default(),
                operator_revision: None,
                plan: None,
                attempts: 1,
            },
            AttemptStart::unchanged(UpstreamId::default_for(Provider::Anthropic)),
        )
    }

    /// Drop every handle and run the task until it drains and closes the store.
    async fn drain(self) -> (tempfile::TempDir, RecorderMetrics, Reader) {
        let Self {
            dir,
            recorder,
            task,
        } = self;
        let meter = recorder.meter();
        let queue = recorder.metrics().queue;
        drop(recorder);
        task.run().await.unwrap();
        let metrics = meter.snapshot(queue);
        let reader = Reader::open(dir.path(), 1).await.unwrap();
        (dir, metrics, reader)
    }
}

async fn detail(reader: &Reader, call: CallId) -> CallDetail {
    reader.call(call).await.unwrap()
}

fn complete(tap: &CallTap, request: &[u8], response: &[u8]) {
    tap.data(Direction::Request, request);
    tap.end(Direction::Request);
    tap.response_head(200, ResponseFormat::EventStream, ResponseEncoding::Identity);
    tap.data(Direction::Response, response);
    tap.end(Direction::Response);
    tap.finish(ended(AttemptTerminal::BodyComplete));
}

#[tokio::test]
async fn a_streamed_call_is_recorded_inspected_and_stored_completely() {
    let harness = Harness::new(CaptureOptions::default(), 4).await;
    let tap = harness.start();
    let call = tap.call();
    tap.data(Direction::Request, b"{\"stream\":true}");
    tap.end(Direction::Request);
    tap.response_head(200, ResponseFormat::EventStream, ResponseEncoding::Identity);
    // Arbitrary fragments, including inside an event.
    for piece in MESSAGE_STREAM.chunks(7) {
        tap.data(Direction::Response, piece);
    }
    tap.end(Direction::Response);
    tap.finish(ended(AttemptTerminal::BodyComplete));
    drop(tap);
    let (_dir, metrics, reader) = harness.drain().await;
    assert_eq!(metrics.omissions.unrecorded_calls, 0);
    let detail = detail(&reader, call).await;
    assert_eq!(detail.call.state, Stored::Known(CallState::Finished));
    assert!(detail.call.capture_complete);
    assert_eq!(
        detail.call.outcome,
        Some(Stored::Known(ModelOutcome::Completed))
    );
    assert_eq!(detail.call.model.as_deref(), Some("claude-test"));
    assert_eq!(detail.call.stop_reason.as_deref(), Some("end_turn"));
    let inspection = detail.attempts[0].inspection.as_ref().unwrap();
    assert!(
        inspection.limitations.is_empty(),
        "{:?}",
        inspection.limitations
    );
}

#[tokio::test]
async fn an_exhausted_byte_budget_omits_data_but_never_the_terminal() {
    // The task does not run until drain, so the budget stays held.
    let options = CaptureOptions {
        queue_bytes: PIECE,
        ..CaptureOptions::default()
    };
    let harness = Harness::new(options, 4).await;
    let tap = harness.start();
    let call = tap.call();
    tap.data(Direction::Request, &vec![7; PIECE * 3]);
    tap.end(Direction::Request);
    tap.response_head(200, ResponseFormat::EventStream, ResponseEncoding::Identity);
    tap.data(Direction::Response, MESSAGE_STREAM);
    tap.end(Direction::Response);
    tap.finish(ended(AttemptTerminal::BodyComplete));
    drop(tap);
    let (_dir, metrics, reader) = harness.drain().await;
    let omitted = u64::try_from(PIECE * 2 + MESSAGE_STREAM.len()).unwrap();
    assert_eq!(metrics.omissions.queue_bytes_bytes, omitted);
    let detail = detail(&reader, call).await;
    assert_eq!(detail.call.state, Stored::Known(CallState::Finished));
    assert_eq!(
        detail.call.terminal,
        Some(Stored::Known(AttemptTerminal::BodyComplete))
    );
    assert!(!detail.call.capture_complete);
    let request = detail.call.request.unwrap();
    assert_eq!(
        (request.observed_bytes, request.stored_bytes),
        (
            Some(u64::try_from(PIECE * 3).unwrap()),
            u64::try_from(PIECE).unwrap()
        )
    );
    let inspection = detail.attempts[0].inspection.as_ref().unwrap();
    assert!(
        inspection
            .limitations
            .contains(&Stored::Known(Limitation::CaptureGap))
    );
    assert_eq!(inspection.outcome, None);
}

#[tokio::test]
async fn exhausted_data_slots_are_counted_by_reason() {
    let options = CaptureOptions {
        queue_capacity: 4,
        ..CaptureOptions::default()
    };
    let harness = Harness::new(options, 1).await;
    let tap = harness.start();
    for _ in 0..6 {
        tap.data(Direction::Request, b"0123456789");
    }
    tap.finish(ended(AttemptTerminal::Cancelled));
    drop(tap);
    let (_dir, metrics, reader) = harness.drain().await;
    assert_eq!(metrics.omissions.queue_slots_bytes, 20);
    let page = reader.calls(&CallQuery::default()).await.unwrap();
    let call = &page.calls[0];
    assert_eq!(
        call.terminal,
        Some(Stored::Known(AttemptTerminal::Cancelled))
    );
    let request = call.request.as_ref().unwrap();
    assert_eq!(
        (request.observed_bytes, request.stored_bytes),
        (Some(60), 40)
    );
    assert_eq!(request.ending, Stored::Known(BodyEnding::Unfinished));
}

#[tokio::test]
async fn every_admitted_call_keeps_its_terminal_when_data_saturates_the_queue() {
    let options = CaptureOptions {
        queue_capacity: 4,
        queue_bytes: PIECE,
        ..CaptureOptions::default()
    };
    let harness = Harness::new(options, 8).await;
    let taps: Vec<_> = (0..8).map(|_| harness.start()).collect();
    for tap in &taps {
        tap.data(Direction::Request, &vec![1; PIECE]);
        tap.response_head(200, ResponseFormat::EventStream, ResponseEncoding::Identity);
        tap.data(Direction::Response, MESSAGE_STREAM);
    }
    let calls: Vec<_> = taps.iter().map(|tap| tap.call()).collect();
    for tap in &taps {
        tap.finish(ended(AttemptTerminal::BodyComplete));
    }
    drop(taps);
    let (_dir, metrics, reader) = harness.drain().await;
    assert_eq!(metrics.omissions.unrecorded_calls, 0);
    assert!(metrics.omissions.queue_bytes_bytes + metrics.omissions.queue_slots_bytes > 0);
    for call in calls {
        let detail = detail(&reader, call).await;
        assert_eq!(detail.call.state, Stored::Known(CallState::Finished));
        let kinds: Vec<_> = detail
            .observations
            .iter()
            .map(|o| o.kind.known().unwrap())
            .collect();
        assert_eq!(kinds.last(), Some(&ObservationKind::CallFinished));
    }
}

#[tokio::test]
async fn calls_beyond_the_reservation_limit_are_forwarded_unrecorded_and_counted() {
    let harness = Harness::new(CaptureOptions::default(), 1).await;
    let first = harness.start();
    let second = harness.start();
    complete(&second, b"x", b"y");
    complete(&first, b"a", MESSAGE_STREAM);
    let recorded = first.call();
    drop((first, second));
    // After the first call's finish is dequeued its reservation is free again.
    let (_dir, metrics, reader) = harness.drain().await;
    assert_eq!(metrics.omissions.unrecorded_calls, 1);
    let page = reader.calls(&CallQuery::default()).await.unwrap();
    assert_eq!(page.calls.len(), 1);
    assert_eq!(page.calls[0].id, recorded);
}

#[tokio::test]
async fn a_dropped_tap_records_an_interrupted_attempt_with_unfinished_bodies() {
    let harness = Harness::new(CaptureOptions::default(), 2).await;
    let tap = harness.start();
    let call = tap.call();
    tap.data(Direction::Request, b"partial upload");
    drop(tap);
    let (_dir, _, reader) = harness.drain().await;
    let detail = detail(&reader, call).await;
    assert_eq!(
        detail.call.terminal,
        Some(Stored::Known(AttemptTerminal::Interrupted))
    );
    assert_eq!(
        detail.call.request.unwrap().ending,
        Stored::Known(BodyEnding::Unfinished)
    );
    assert!(detail.attempts[0].inspection.is_none());
}

#[tokio::test]
async fn the_body_limit_is_a_policy_omission() {
    let options = CaptureOptions {
        body_limit_bytes: 3,
        ..CaptureOptions::default()
    };
    let harness = Harness::new(options, 2).await;
    let tap = harness.start();
    complete(&tap, b"abcdef", b"");
    drop(tap);
    let (_dir, metrics, reader) = harness.drain().await;
    assert_eq!(metrics.omissions.policy_limit_bytes, 3);
    let call = reader
        .calls(&CallQuery::default())
        .await
        .unwrap()
        .calls
        .remove(0);
    let request = call.request.unwrap();
    assert_eq!((request.observed_bytes, request.stored_bytes), (Some(6), 3));
}

#[tokio::test]
async fn live_subscribers_see_committed_observations_in_order() {
    let harness = Harness::new(CaptureOptions::default(), 2).await;
    let mut live = harness.recorder.subscribe();
    let Harness {
        dir: _dir,
        recorder,
        task,
    } = harness;
    let tap = recorder.start(
        CallStart {
            transport: Transport::Http,
            provider: Provider::Codex,
            endpoint: Endpoint::Responses,
            method: Method::Post,
            config_revision: ConfigRevision::new("rev"),
            attribution: Attribution::default(),
            operator_revision: None,
            plan: None,
            attempts: 1,
        },
        AttemptStart::unchanged(UpstreamId::default_for(Provider::Codex)),
    );
    let observe = async move {
        complete(
            &tap,
            b"{}",
            b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n",
        );
        drop(tap);
        let mut kinds = Vec::new();
        while kinds.len() < 5 {
            let observation = live.recv().await.unwrap();
            kinds.push(observation.kind.known().unwrap());
        }
        drop(recorder);
        kinds
    };
    let (result, kinds) = tokio::join!(task.run(), observe);
    result.unwrap();
    assert_eq!(
        kinds,
        [
            ObservationKind::CallOpened,
            ObservationKind::AttemptStarted,
            ObservationKind::ResponseHead,
            ObservationKind::AttemptFinished,
            ObservationKind::CallFinished,
        ]
    );
}

#[tokio::test]
async fn a_failed_commit_keeps_lifecycle_records_and_reports_the_lost_bytes() {
    use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
    let harness = Harness::new(CaptureOptions::default(), 2).await;
    let mut sabotage = SqliteConnection::connect_with(
        &SqliteConnectOptions::new().filename(harness.dir.path().join("tgv.sqlite3")),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER reject_segments BEFORE INSERT ON segments BEGIN SELECT RAISE(FAIL, 'fixture failure'); END")
        .execute(&mut sabotage)
        .await
        .unwrap();
    sabotage.close().await.unwrap();
    let tap = harness.start();
    let call = tap.call();
    complete(&tap, b"request body", MESSAGE_STREAM);
    drop(tap);
    let (_dir, metrics, reader) = harness.drain().await;
    assert_eq!(metrics.storage_errors, 1);
    assert!(!metrics.healthy());
    assert!(metrics.omissions.storage_failure_bytes > 0);
    assert!(metrics.commits >= 1);
    let detail = detail(&reader, call).await;
    assert_eq!(detail.call.state, Stored::Known(CallState::Finished));
    assert!(!detail.call.capture_complete);
    assert_eq!(detail.call.request.unwrap().stored_bytes, 0);
    // Inspection describes the bytes the recorder saw, before storage failed.
    assert_eq!(
        detail.call.outcome,
        Some(Stored::Known(ModelOutcome::Completed))
    );
}
