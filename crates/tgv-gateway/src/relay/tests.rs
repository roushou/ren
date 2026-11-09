use super::{AttemptCell, Dispatch, ObservedUpload};
use crate::{
    attempt::{HttpEvent, SendFailure, StreamFailure},
    upstream::UpstreamBody,
};
use axum::body::Body;
use bytes::Bytes;
use futures_util::{StreamExt as _, stream};
use http_body::{Body as _, Frame};
use http_body_util::{BodyExt, StreamBody};
use std::{io, sync::Arc};
use tgv_harness::attribution::Attribution;
use tgv_protocol::route::{Endpoint, Method, Provider, Transport};
use tgv_recorder::{AttemptStart, routing::UpstreamId};
use tgv_recorder::{
    CallStart, CaptureOptions, Recorder, RecorderSetup, RecorderTask,
    capture::{AttemptPhase, AttemptTerminal, BodyEnding, TerminalCause},
    id::{CallId, SequentialIds},
    retention::RetentionPolicy,
    routing::ConfigRevision,
    time::{ManualClock, UnixMillis},
};
use tgv_store::{Reader, Store};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

struct Harness {
    dir: tempfile::TempDir,
    recorder: Recorder,
    task: RecorderTask<Store>,
    admission: Arc<Semaphore>,
    abort: CancellationToken,
}

/// What the store recorded for the attempt.
#[derive(Debug, PartialEq, Eq)]
struct Recorded {
    terminal: AttemptTerminal,
    cause: Option<TerminalCause>,
    phase: Option<AttemptPhase>,
    request: (u64, BodyEnding),
    response: (u64, BodyEnding),
}

impl Harness {
    async fn open() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::at(UnixMillis::from_millis(1_800_000_000_000)));
        let ids = Arc::new(SequentialIds::default());
        let (store, recovery) = Store::open(dir.path(), clock.as_ref(), ids.clone())
            .await
            .unwrap();
        let setup = RecorderSetup {
            options: CaptureOptions::default(),
            retention: RetentionPolicy::default(),
            max_calls: 4,
            live_buffer: 16,
            scanner: std::sync::Arc::new(tgv_recorder::secrets::SecretScanner::new().unwrap()),
        };
        let (recorder, task) = Recorder::new(store, recovery.calls, setup, clock, ids).unwrap();
        Self {
            dir,
            recorder,
            task,
            admission: Arc::new(Semaphore::new(1)),
            abort: CancellationToken::new(),
        }
    }

    fn begin(&self) -> (Arc<AttemptCell>, CallId) {
        let tap = self.recorder.start(
            CallStart {
                transport: Transport::Http,
                api: Provider::Codex.native_api(),
                endpoint: Endpoint::Responses,
                method: Method::Post,
                config_revision: ConfigRevision::new("fixture"),
                attribution: Attribution::default(),
                caller: tgv_recorder::client::Caller::Unidentified,
                operator_revision: None,
                plan: None,
                attempts: 1,
            },
            AttemptStart::unchanged(UpstreamId::default_for(Provider::Codex), Provider::Codex),
        );
        let id = tap.call();
        let permit = self.admission.clone().try_acquire_owned().unwrap();
        (
            AttemptCell::new(
                tap,
                Some(permit),
                self.abort.clone(),
                CancellationToken::new(),
            ),
            id,
        )
    }

    fn free(&self) -> usize {
        self.admission.available_permits()
    }

    async fn record(self, id: CallId) -> Recorded {
        drop(self.recorder);
        self.task.run().await.unwrap();
        let reader = Reader::open(self.dir.path(), 1).await.unwrap();
        let detail = reader.call(id).await.unwrap();
        let body = |body: Option<tgv_store::view::BodyView>| {
            let body = body.unwrap();
            (
                body.observed_bytes.unwrap_or_default(),
                body.ending.known().unwrap(),
            )
        };
        let attempt = &detail.attempts[0];
        Recorded {
            terminal: attempt.terminal.as_ref().unwrap().known().unwrap(),
            cause: attempt.terminal_cause.as_ref().map(|c| c.known().unwrap()),
            phase: attempt.terminal_phase.as_ref().map(|p| p.known().unwrap()),
            request: body(detail.call.request),
            response: body(detail.call.response),
        }
    }
}

fn pending_body() -> Body {
    Body::from_stream(stream::pending::<Result<Bytes, io::Error>>())
}

fn response(body: UpstreamBody) -> http::Response<UpstreamBody> {
    http::Response::new(body)
}

fn frames(frames: Vec<Result<Frame<Bytes>, StreamFailure>>) -> UpstreamBody {
    UpstreamBody::new(StreamBody::new(stream::iter(frames)))
}

#[tokio::test]
async fn the_response_end_releases_admission_and_stops_an_unfinished_upload() {
    let harness = Harness::open().await;
    let (cell, id) = harness.begin();
    let mut upload = ObservedUpload::wrap(pending_body(), cell.clone());
    let mut body = Dispatch::new(cell)
        .responded(response(UpstreamBody::full("abc")))
        .unwrap()
        .into_body();
    assert_eq!(harness.free(), 0);
    assert_eq!(body.size_hint().exact(), Some(3));
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        "abc"
    );
    // The end is the attempt's terminal: admission returns even though the
    // client still holds the stream, and the upload has nowhere to go.
    assert_eq!(harness.free(), 1);
    assert!(upload.frame().await.unwrap().is_err());
    assert!(body.is_end_stream());
    assert!(body.frame().await.is_none());
    drop((upload, body));
    let recorded = harness.record(id).await;
    assert_eq!(
        (recorded.terminal, recorded.phase),
        (AttemptTerminal::BodyComplete, Some(AttemptPhase::Response))
    );
    assert_eq!(recorded.request.1, BodyEnding::Unfinished);
    assert_eq!(recorded.response, (3, BodyEnding::Ended));
}

#[tokio::test]
async fn a_finished_upload_waits_for_dispatch_but_a_failed_one_ends_the_attempt() {
    // A complete upload owns nothing: the attempt waits for the head, and a
    // dispatch dropped without one is interrupted while awaiting it.
    let harness = Harness::open().await;
    let (cell, id) = harness.begin();
    let dispatch = Dispatch::new(cell.clone());
    let mut upload = ObservedUpload::wrap(Body::from("abc"), cell);
    assert!(upload.frame().await.unwrap().is_ok());
    assert!(upload.frame().await.is_none());
    assert_eq!(harness.free(), 0);
    drop((upload, dispatch));
    assert_eq!(harness.free(), 1);
    let recorded = harness.record(id).await;
    assert_eq!(
        (recorded.terminal, recorded.cause, recorded.phase),
        (
            AttemptTerminal::Interrupted,
            None,
            Some(AttemptPhase::AwaitingHead)
        )
    );
    // The gateway records the buffered request itself; the upload does not.
    assert_eq!(recorded.request, (0, BodyEnding::Unfinished));

    // A failed upload means the client left: that ends the attempt at once,
    // and the dispatch that follows reports the same end.
    let harness = Harness::open().await;
    let (cell, id) = harness.begin();
    let dispatch = Dispatch::new(cell.clone());
    let failing = Body::from_stream(stream::iter([Err::<Bytes, _>(io::Error::other("fixture"))]));
    let mut upload = ObservedUpload::wrap(failing, cell);
    assert!(upload.frame().await.unwrap().is_err());
    assert_eq!(harness.free(), 1);
    let end = dispatch.stopped(HttpEvent::SendFailed(SendFailure::SendFailed));
    assert_eq!(end.phase, AttemptPhase::Upload);
    drop(upload);
    let recorded = harness.record(id).await;
    assert_eq!(
        (recorded.terminal, recorded.cause, recorded.phase),
        (
            AttemptTerminal::Cancelled,
            Some(TerminalCause::Client),
            Some(AttemptPhase::Upload)
        )
    );
    assert_eq!(recorded.request.1, BodyEnding::Unfinished);
}

#[tokio::test]
async fn response_failures_aborts_and_drops_keep_their_evidence() {
    for (abort, consume, terminal, cause, observed) in [
        (
            false,
            true,
            AttemptTerminal::StreamFailed,
            Some(TerminalCause::BodyError),
            3,
        ),
        (
            true,
            true,
            AttemptTerminal::Cancelled,
            Some(TerminalCause::Shutdown),
            0,
        ),
        (false, false, AttemptTerminal::Interrupted, None, 0),
    ] {
        let harness = Harness::open().await;
        let (cell, id) = harness.begin();
        let body = frames(vec![
            Ok(Frame::data(Bytes::from_static(b"abc"))),
            Err(StreamFailure::BodyError),
        ]);
        let mut body = Dispatch::new(cell)
            .responded(response(body))
            .unwrap()
            .into_body();
        if abort {
            harness.abort.cancel();
        }
        // An abort requested is not yet an observed terminal.
        assert_eq!(harness.free(), 0);
        if consume {
            let mut error = false;
            while let Some(result) = body.frame().await {
                error |= result.is_err();
            }
            assert!(error);
        }
        drop(body);
        assert_eq!(harness.free(), 1);
        let recorded = harness.record(id).await;
        assert_eq!((recorded.terminal, recorded.cause), (terminal, cause));
        assert_eq!(recorded.response, (observed, BodyEnding::Unfinished));
    }
}

#[tokio::test]
async fn empty_bodies_complete_without_a_poll() {
    let harness = Harness::open().await;
    let (cell, id) = harness.begin();
    let upload = ObservedUpload::wrap(Body::empty(), cell.clone());
    let body = Dispatch::new(cell)
        .responded(response(UpstreamBody::empty()))
        .unwrap();
    assert_eq!(harness.free(), 1);
    drop((upload, body));
    let recorded = harness.record(id).await;
    assert_eq!(recorded.terminal, AttemptTerminal::BodyComplete);
    assert_eq!(recorded.request, (0, BodyEnding::Unfinished));
    assert_eq!(recorded.response, (0, BodyEnding::Ended));
}

#[tokio::test]
async fn a_data_length_does_not_end_a_body_before_its_trailers() {
    let harness = Harness::open().await;
    let (cell, id) = harness.begin();
    let mut trailers = http::HeaderMap::new();
    trailers.insert("x-fixture-trailer", "done".parse().unwrap());
    let upstream = http::Response::builder()
        .header("content-length", "3")
        .body(frames(vec![
            Ok(Frame::data(Bytes::from_static(b"abc"))),
            Ok(Frame::trailers(trailers.clone())),
        ]))
        .unwrap();
    let mut body = Dispatch::new(cell).responded(upstream).unwrap().into_body();
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        "abc"
    );
    assert_eq!(harness.free(), 0);
    assert!(!body.is_end_stream());
    assert_eq!(
        body.frame()
            .await
            .unwrap()
            .unwrap()
            .into_trailers()
            .unwrap(),
        trailers
    );
    assert!(body.frame().await.is_none());
    assert_eq!(harness.free(), 1);
    drop(body);
    let recorded = harness.record(id).await;
    assert_eq!(recorded.terminal, AttemptTerminal::BodyComplete);
    assert_eq!(recorded.response, (3, BodyEnding::Ended));
}

#[tokio::test]
async fn a_body_let_go_after_its_declared_length_is_complete_and_short_of_it_interrupted() {
    for (sent, terminal, cause, ending) in [
        (
            &b"abc"[..],
            AttemptTerminal::BodyComplete,
            Some(TerminalCause::DeclaredLength),
            BodyEnding::Ended,
        ),
        (
            &b"ab"[..],
            AttemptTerminal::Interrupted,
            None,
            BodyEnding::Unfinished,
        ),
    ] {
        let harness = Harness::open().await;
        let (cell, id) = harness.begin();
        // The data arrives; the stream's end does not (yet).
        let body = UpstreamBody::new(StreamBody::new(
            stream::iter([Ok(Frame::data(Bytes::copy_from_slice(sent)))]).chain(stream::pending()),
        ));
        let upstream = http::Response::builder()
            .header("content-length", "3")
            .body(body)
            .unwrap();
        let mut body = Dispatch::new(cell).responded(upstream).unwrap().into_body();
        assert_eq!(
            body.frame().await.unwrap().unwrap().into_data().unwrap(),
            sent
        );
        // Reaching the length while the body is still read ends nothing.
        assert!(!body.is_end_stream());
        assert_eq!(harness.free(), 0);
        drop(body);
        assert_eq!(harness.free(), 1);
        let recorded = harness.record(id).await;
        assert_eq!((recorded.terminal, recorded.cause), (terminal, cause));
        assert_eq!(
            recorded.response,
            (u64::try_from(sent.len()).unwrap(), ending)
        );
    }
}

#[test]
fn only_one_decimal_content_length_is_a_declared_length() {
    let length = |values: &[&str]| {
        let mut headers = http::HeaderMap::new();
        for value in values {
            headers.append("content-length", value.parse().unwrap());
        }
        super::ObservedResponse::declared_length(&headers)
    };
    assert_eq!(length(&["22"]), Some(22));
    assert_eq!(length(&[]), None);
    assert_eq!(length(&["22", "22"]), None);
    assert_eq!(length(&["-1"]), None);
    assert_eq!(length(&["0x16"]), None);
}
