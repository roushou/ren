use crate::{
    PrefixLimit, Reader, Store, StoreError,
    query::{CallFilter, CallQuery, PageLimit, SearchText, StatusFilter, ToolFilter},
};
use ren_harness::attribution::{Attribution, HintSource, SessionHint};
use ren_protocol::{
    direction::Direction,
    encoding::{ResponseEncoding, ResponseFormat},
    inspect::{FormatSource, Inspector, ModelOutcome, Observation, ReportLimits},
    label::Stored,
    native::{NativeSessionId, NativeTurnId},
    route::{Endpoint, Method, Provider, Transport},
    text::WireText,
};
use ren_recorder::{batch::AttemptBody, routing::UpstreamId};
use ren_recorder::{
    batch::{
        Batch, BodyEnd, BodyKey, FinishAttempt, FinishCall, ObservationKind, Op, OpenCall,
        ResponseHead, Segment, Stamp, StartAttempt,
    },
    chunk::Chunker,
    inspection::InspectionRecord,
    retention::RetentionPolicy,
};
use ren_recorder::{
    capture::{
        AttemptPhase, AttemptTerminal, BodyEnding, CallState, TerminalCause, TerminalRecord,
    },
    id::{AttemptId, CallId, ChannelId, SequentialIds},
    routing::ConfigRevision,
    time::{Clock, ManualClock, UnixMillis},
};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

struct Fixture {
    dir: tempfile::TempDir,
    clock: ManualClock,
    ids: Arc<SequentialIds>,
    store: Store,
    reader: Reader,
}

/// What to record for one synthetic call.
struct CallSpec<'a> {
    provider: Provider,
    request: &'a [u8],
    response: &'a [u8],
    /// Bytes of the response observed but not stored, at the end.
    omitted: u64,
    session: Option<u128>,
    tool: Option<&'a str>,
    status: u16,
    usage: Option<ren_protocol::inspect::Usage>,
    endpoint: Endpoint,
    method: Method,
    source: HintSource,
}

impl Default for CallSpec<'_> {
    fn default() -> Self {
        Self {
            provider: Provider::Anthropic,
            request: b"{\"model\":\"claude\"}",
            response: b"data: {}\n\n",
            omitted: 0,
            session: None,
            tool: None,
            status: 200,
            usage: None,
            endpoint: Endpoint::Messages,
            method: Method::Post,
            source: HintSource::ClaudeCodeHeaders,
        }
    }
}

impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let clock = ManualClock::at(UnixMillis::from_millis(1_000_000));
        let ids = Arc::new(SequentialIds::default());
        let (store, recovery) = Store::open(dir.path(), &clock, ids.clone()).await.unwrap();
        assert_eq!(recovery.calls, 0);
        let reader = Reader::open(dir.path(), 2).await.unwrap();
        Self {
            dir,
            clock,
            ids,
            store,
            reader,
        }
    }

    fn stamp(&self, channel: ChannelId, call: CallId) -> Stamp {
        Stamp {
            at: self.clock.now(),
            channel,
            call,
        }
    }

    fn body(batch: &mut Batch, key: BodyKey, bytes: &[u8]) {
        let mut chunker = Chunker::at(0);
        let mut cuts = Vec::new();
        chunker.push(bytes, &mut cuts);
        chunker.flush(&mut cuts);
        let segments = cuts
            .into_iter()
            .map(|cut| Segment {
                offset: cut.offset,
                chunk: batch.add_chunk(cut.bytes),
            })
            .collect();
        batch.push(Op::Segments {
            body: key,
            segments,
        });
    }

    fn open(&self, spec: &CallSpec<'_>) -> (Batch, Stamp, AttemptId) {
        let channel = ChannelId::generate(self.ids.as_ref());
        let call = CallId::generate(self.ids.as_ref());
        let attempt = AttemptId::generate(self.ids.as_ref());
        let stamp = self.stamp(channel, call);
        let mut batch = Batch::default();
        batch.push(Op::OpenCall(OpenCall {
            stamp,
            transport: Transport::Http,
            provider: spec.provider,
            wire: spec.provider.native_api().wire(),
            endpoint: spec.endpoint,
            method: spec.method,
            config_revision: ConfigRevision::new("rev-1"),
            attribution: Attribution {
                hint: spec.session.map(|session| SessionHint {
                    source: spec.source,
                    session: NativeSessionId::new(Uuid::from_u128(session)).unwrap(),
                    turn: NativeTurnId::new(Uuid::from_u128(session + 1000)),
                    thread: None,
                }),
                issues: Vec::new(),
            },
            caller: ren_recorder::caller::Caller::Unidentified,
            operator_revision: None,
            plan: None,
        }));
        batch.push(Op::StartAttempt(StartAttempt {
            stamp,
            attempt,
            ordinal: 0,
            upstream: UpstreamId::default_for(spec.provider),
            provider: spec.provider,
            changes: None,
            body: AttemptBody::Original,
        }));
        (batch, stamp, attempt)
    }

    async fn record(&mut self, spec: CallSpec<'_>) -> CallId {
        let (mut batch, stamp, attempt) = self.open(&spec);
        Self::body(&mut batch, BodyKey::Request(stamp.call), spec.request);
        batch.push(Op::ResponseHead(ResponseHead {
            stamp,
            attempt,
            status: spec.status,
            format: ResponseFormat::EventStream,
            encoding: ResponseEncoding::Identity,
        }));
        Self::body(&mut batch, BodyKey::Response(attempt), spec.response);
        let mut inspector = Inspector::new(FormatSource::Declared, ReportLimits::default());
        inspector.observe(Observation::Model(WireText::new("claude-test").unwrap()));
        inspector.observe(Observation::Outcome(ModelOutcome::Completed));
        inspector.observe(Observation::TerminalEvent);
        if let Some(usage) = spec.usage {
            inspector.observe(Observation::Usage(usage));
        }
        if let Some(tool) = spec.tool {
            inspector.observe(Observation::ToolCall(ren_protocol::inspect::ToolCall {
                call_id: Some(WireText::new("toolu_1").unwrap()),
                item_id: None,
                kind: WireText::new("tool_use").unwrap(),
                name: Some(WireText::new(tool).unwrap()),
            }));
        }
        let report = inspector.finish();
        let observed = |bytes: &[u8]| u64::try_from(bytes.len()).unwrap();
        batch.push(Op::FinishAttempt(FinishAttempt {
            stamp,
            attempt,
            end: TerminalRecord {
                terminal: AttemptTerminal::BodyComplete,
                cause: None,
                phase: Some(AttemptPhase::Response),
            },
            response: BodyEnd {
                observed_bytes: observed(spec.response) + spec.omitted,
                ending: BodyEnding::Ended,
            },
            request: None,
            inspection: Some(Box::new(InspectionRecord::from(&report))),
        }));
        batch.push(Op::FinishCall(FinishCall {
            stamp,
            request: BodyEnd {
                observed_bytes: observed(spec.request),
                ending: BodyEnding::Ended,
            },
            rejection: None,
        }));
        self.store.commit(&batch).await.unwrap();
        self.clock.advance(Duration::from_millis(10));
        stamp.call
    }

    async fn export(&self, key: BodyKey) -> Result<Vec<u8>, StoreError> {
        let mut out = Vec::new();
        self.reader
            .export(key, |bytes| {
                out.extend_from_slice(bytes);
                Ok(())
            })
            .await
            .map(|_| out)
    }
}

fn large(len: usize, seed: u8) -> Vec<u8> {
    let mut state = u64::from(seed) + 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

#[tokio::test]
async fn a_recorded_call_reads_back_with_its_log_bodies_and_inspection() {
    let mut fixture = Fixture::new().await;
    let request = large(300_000, 1);
    let call = fixture
        .record(CallSpec {
            request: &request,
            session: Some(7),
            tool: Some("Bash"),
            ..CallSpec::default()
        })
        .await;
    let page = fixture.reader.calls(&CallQuery::default()).await.unwrap();
    assert_eq!(page.calls.len(), 1);
    assert!(page.next.is_none());
    let view = &page.calls[0];
    assert_eq!(view.id, call);
    assert_eq!(view.state, Stored::Known(CallState::Finished));
    assert_eq!(
        view.terminal,
        Some(Stored::Known(AttemptTerminal::BodyComplete))
    );
    assert_eq!(view.status, Some(200));
    assert_eq!(view.model.as_deref(), Some("claude-test"));
    assert_eq!(view.outcome, Some(Stored::Known(ModelOutcome::Completed)));
    assert_eq!(view.tool_names, ["Bash"]);
    assert!(view.capture_complete);
    assert!(view.session.is_some() && view.turn.is_some());
    let request_view = view.request.as_ref().unwrap();
    assert_eq!(request_view.observed_bytes, Some(300_000));
    assert_eq!(request_view.stored_bytes, 300_000);

    let detail = fixture.reader.call(call).await.unwrap();
    assert_eq!(detail.attempts.len(), 1);
    assert_eq!(
        detail.attempts[0].inspection_version,
        Some(InspectionRecord::VERSION)
    );
    assert!(detail.attempts[0].inspection.is_some());
    let kinds: Vec<_> = detail
        .observations
        .iter()
        .map(|o| o.kind.known().unwrap())
        .collect();
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
    assert!(
        detail
            .observations
            .windows(2)
            .all(|pair| pair[0].seq < pair[1].seq)
    );

    let key = fixture
        .reader
        .body_key(call, Direction::Request)
        .await
        .unwrap();
    assert_eq!(fixture.export(key).await.unwrap(), request);
    let prefix = fixture
        .reader
        .body_prefix(
            key,
            PrefixLimit {
                bytes: 1000,
                segments: 64,
            },
        )
        .await
        .unwrap();
    assert_eq!(prefix.bytes, request[..1000]);
    assert!(prefix.truncated && !prefix.gap);
    let sessions = fixture.reader.sessions(PageLimit::DEFAULT).await.unwrap();
    assert_eq!(
        (
            sessions.len(),
            sessions[0].call_count,
            sessions[0].turn_count
        ),
        (1, 1, 1)
    );
    let timeline = fixture
        .reader
        .session_calls(sessions[0].id, PageLimit::DEFAULT, None)
        .await
        .unwrap();
    assert_eq!(timeline.calls[0].id, call);
}

#[tokio::test]
async fn repeated_conversation_prefixes_are_stored_once() {
    let mut fixture = Fixture::new().await;
    // Each turn resends the whole conversation plus one new message.
    let mut conversation = Vec::new();
    let mut raw = 0;
    for turn in 0..8 {
        conversation.extend(large(60_000, turn));
        raw += conversation.len();
        fixture
            .record(CallSpec {
                request: &conversation,
                session: Some(1),
                ..CallSpec::default()
            })
            .await;
    }
    let storage = fixture.reader.storage().await.unwrap();
    let ratio = storage.referenced_bytes * 100 / storage.unique_bytes;
    println!(
        "dedup: referenced {} unique {} ratio {ratio}%",
        storage.referenced_bytes, storage.unique_bytes
    );
    assert!(storage.referenced_bytes >= u64::try_from(raw).unwrap());
    // 8 turns of a growing conversation: 2.16 MB sent, 480 KB distinct, plus
    // the chunk re-cut at each turn boundary. Measured: about 3.4.
    assert!(ratio > 300, "dedup ratio {ratio}%");
}

#[tokio::test]
async fn missing_bytes_are_never_exported_or_joined_across_a_gap() {
    let mut fixture = Fixture::new().await;
    let call = fixture
        .record(CallSpec {
            response: b"partial",
            omitted: 5,
            ..CallSpec::default()
        })
        .await;
    let view = fixture
        .reader
        .calls(&CallQuery::default())
        .await
        .unwrap()
        .calls
        .remove(0);
    assert!(!view.capture_complete);
    let key = fixture
        .reader
        .body_key(call, Direction::Response)
        .await
        .unwrap();
    assert!(matches!(
        fixture.export(key).await,
        Err(StoreError::IncompleteBody)
    ));
    let prefix = fixture
        .reader
        .body_prefix(
            key,
            PrefixLimit {
                bytes: 100,
                segments: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(prefix.bytes, b"partial");
    assert!(prefix.gap && !prefix.truncated);

    // A hole in the middle: bytes after it are not part of the prefix.
    let (mut batch, stamp, attempt) = fixture.open(&CallSpec::default());
    let first = batch.add_chunk(bytes::Bytes::from_static(b"abc"));
    let third = batch.add_chunk(bytes::Bytes::from_static(b"ghi"));
    batch.push(Op::Segments {
        body: BodyKey::Response(attempt),
        segments: vec![
            Segment {
                offset: 0,
                chunk: first,
            },
            Segment {
                offset: 6,
                chunk: third,
            },
        ],
    });
    batch.push(Op::FinishAttempt(FinishAttempt {
        stamp,
        attempt,
        end: TerminalRecord {
            terminal: AttemptTerminal::BodyComplete,
            cause: None,
            phase: Some(AttemptPhase::Response),
        },
        response: BodyEnd {
            observed_bytes: 9,
            ending: BodyEnding::Ended,
        },
        request: None,
        inspection: None,
    }));
    fixture.store.commit(&batch).await.unwrap();
    let key = BodyKey::Response(attempt);
    let prefix = fixture
        .reader
        .body_prefix(
            key,
            PrefixLimit {
                bytes: 100,
                segments: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!((prefix.bytes.as_slice(), prefix.gap), (&b"abc"[..], true));
    assert!(matches!(
        fixture.export(key).await,
        Err(StoreError::IncompleteBody)
    ));
}

#[tokio::test]
async fn a_restart_marks_open_calls_recovered_without_inventing_an_end() {
    let mut fixture = Fixture::new().await;
    let (batch, stamp, _) = fixture.open(&CallSpec::default());
    fixture.store.commit(&batch).await.unwrap();
    assert!(matches!(
        Store::open(fixture.dir.path(), &fixture.clock, fixture.ids.clone()).await,
        Err(StoreError::Locked)
    ));
    let Fixture {
        dir,
        clock,
        ids,
        store,
        reader,
    } = fixture;
    store.close().await.unwrap();
    reader.close().await;
    let (store, recovery) = Store::open(dir.path(), &clock, ids).await.unwrap();
    assert_eq!(recovery.calls, 1);
    let reader = Reader::open(dir.path(), 1).await.unwrap();
    let detail = reader.call(stamp.call).await.unwrap();
    assert_eq!(detail.call.state, Stored::Known(CallState::Recovered));
    assert_eq!(detail.call.terminal, None);
    assert_eq!(detail.call.ended_at, None);
    assert!(!detail.call.capture_complete);
    assert_eq!(
        detail.call.request.unwrap().ending,
        Stored::Known(BodyEnding::Unfinished)
    );
    assert_eq!(
        detail.observations.last().unwrap().kind,
        Stored::Known(ObservationKind::CallRecovered)
    );
    let page = reader
        .calls(&CallQuery {
            filter: CallFilter {
                status: Some(StatusFilter::Interrupted),
                ..CallFilter::default()
            },
            ..CallQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(page.calls.len(), 1);
    store.close().await.unwrap();
}

#[tokio::test]
async fn a_lifecycle_retry_without_data_keeps_the_terminal_and_reports_loss() {
    let mut fixture = Fixture::new().await;
    let (mut batch, stamp, attempt) = fixture.open(&CallSpec::default());
    Fixture::body(&mut batch, BodyKey::Request(stamp.call), b"lost request");
    batch.push(Op::FinishAttempt(FinishAttempt {
        stamp,
        attempt,
        end: TerminalRecord {
            terminal: AttemptTerminal::NotSent,
            cause: Some(TerminalCause::ConnectFailed),
            phase: Some(AttemptPhase::Upload),
        },
        response: BodyEnd {
            observed_bytes: 0,
            ending: BodyEnding::Ended,
        },
        request: None,
        inspection: None,
    }));
    batch.push(Op::FinishCall(FinishCall {
        stamp,
        request: BodyEnd {
            observed_bytes: 12,
            ending: BodyEnding::Ended,
        },
        rejection: None,
    }));
    fixture.store.commit(&batch.without_data()).await.unwrap();
    let detail = fixture.reader.call(stamp.call).await.unwrap();
    assert_eq!(detail.call.state, Stored::Known(CallState::Finished));
    assert!(!detail.call.capture_complete);
    let request = detail.call.request.unwrap();
    assert_eq!(
        (request.observed_bytes, request.stored_bytes),
        (Some(12), 0)
    );
}

#[tokio::test]
async fn retention_removes_oldest_calls_and_sweeps_only_unreferenced_chunks() {
    let mut fixture = Fixture::new().await;
    let shared = large(200_000, 9);
    let first = fixture
        .record(CallSpec {
            request: &shared,
            session: Some(3),
            ..CallSpec::default()
        })
        .await;
    let unique = large(200_000, 10);
    fixture
        .record(CallSpec {
            request: &unique,
            ..CallSpec::default()
        })
        .await;
    let last = fixture
        .record(CallSpec {
            request: &shared,
            ..CallSpec::default()
        })
        .await;
    let before = fixture.reader.storage().await.unwrap();
    let policy = RetentionPolicy {
        max_calls: 1,
        ..RetentionPolicy::default()
    };
    let pruned = fixture
        .store
        .prune(&policy, fixture.clock.now().wall)
        .await
        .unwrap();
    assert_eq!(pruned.calls, 2);
    assert!(pruned.chunks > 0);
    let after = fixture.reader.storage().await.unwrap();
    assert_eq!(after.calls, 1);
    assert!(after.chunks < before.chunks);
    // The shared chunks are still referenced by the retained call.
    let key = fixture
        .reader
        .body_key(last, Direction::Request)
        .await
        .unwrap();
    assert_eq!(fixture.export(key).await.unwrap(), shared);
    assert!(matches!(
        fixture.reader.call(first).await,
        Err(StoreError::NotFound)
    ));
    assert!(
        fixture
            .reader
            .sessions(PageLimit::DEFAULT)
            .await
            .unwrap()
            .is_empty()
    );

    // Age: everything older than the cutoff goes.
    fixture.clock.advance(Duration::from_secs(10));
    let policy = RetentionPolicy {
        max_age_seconds: 5,
        ..RetentionPolicy::default()
    };
    fixture
        .store
        .prune(&policy, fixture.clock.now().wall)
        .await
        .unwrap();
    let after = fixture.reader.storage().await.unwrap();
    assert_eq!((after.calls, after.chunks), (0, 0));
}

#[tokio::test]
async fn open_calls_are_never_pruned() {
    let mut fixture = Fixture::new().await;
    let (batch, stamp, _) = fixture.open(&CallSpec::default());
    fixture.store.commit(&batch).await.unwrap();
    let policy = RetentionPolicy {
        max_calls: 1,
        max_age_seconds: 1,
        ..RetentionPolicy::default()
    };
    fixture.clock.advance(Duration::from_secs(60));
    fixture
        .store
        .prune(&policy, fixture.clock.now().wall)
        .await
        .unwrap();
    assert!(fixture.reader.call(stamp.call).await.is_ok());
}

#[tokio::test]
async fn filters_and_cursors_page_through_calls() {
    let mut fixture = Fixture::new().await;
    for index in 0..5_u16 {
        fixture
            .record(CallSpec {
                provider: if index % 2 == 0 {
                    Provider::Anthropic
                } else {
                    Provider::Codex
                },
                tool: (index == 3).then_some("apply_patch"),
                status: if index == 4 { 529 } else { 200 },
                ..CallSpec::default()
            })
            .await;
    }
    let query = |filter: CallFilter| CallQuery {
        filter,
        ..CallQuery::default()
    };
    let count = |page: super::view::CallPage| page.calls.len();
    let reader = &fixture.reader;
    assert_eq!(
        count(
            reader
                .calls(&query(CallFilter {
                    provider: Some(Provider::Codex),
                    ..CallFilter::default()
                }))
                .await
                .unwrap()
        ),
        2
    );
    assert_eq!(
        count(
            reader
                .calls(&query(CallFilter {
                    tools: Some(ToolFilter::WithTools),
                    ..CallFilter::default()
                }))
                .await
                .unwrap()
        ),
        1
    );
    assert_eq!(
        count(
            reader
                .calls(&query(CallFilter {
                    tool: Some(SearchText::new("PATCH").unwrap()),
                    ..CallFilter::default()
                }))
                .await
                .unwrap()
        ),
        1
    );
    assert_eq!(
        count(
            reader
                .calls(&query(CallFilter {
                    text: Some(SearchText::new("apply").unwrap()),
                    ..CallFilter::default()
                }))
                .await
                .unwrap()
        ),
        1
    );
    assert_eq!(
        count(
            reader
                .calls(&query(CallFilter {
                    status: Some(StatusFilter::Errors),
                    ..CallFilter::default()
                }))
                .await
                .unwrap()
        ),
        1
    );
    assert_eq!(
        count(
            reader
                .calls(&query(CallFilter {
                    model: Some(SearchText::new("nope").unwrap()),
                    ..CallFilter::default()
                }))
                .await
                .unwrap()
        ),
        0
    );

    let mut seen = Vec::new();
    let mut after = None;
    loop {
        let page = reader
            .calls(&CallQuery {
                limit: PageLimit::new(2).unwrap(),
                after,
                ..CallQuery::default()
            })
            .await
            .unwrap();
        seen.extend(page.calls.iter().map(|call| call.id));
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    assert_eq!(seen.len(), 5);
    assert!(
        seen.windows(2).all(|pair| pair[0] > pair[1]),
        "newest first"
    );
}

#[tokio::test]
async fn the_last_operator_snapshot_is_returned_as_stored() {
    use crate::StoredOperatorState;
    use ren_recorder::{
        batch::{Actor, OperatorAction, Recorded},
        routing::OperatorRevision,
    };
    use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::at(UnixMillis::from_millis(1_000));
    let ids = Arc::new(SequentialIds::default());
    let (mut store, recovery) = Store::open(dir.path(), &clock, ids.clone()).await.unwrap();
    assert_eq!(recovery.operator, None);
    // The store never reads these documents; any JSON is kept as it is.
    let mut batch = Batch::default();
    for (revision, command, state) in [
        (
            1,
            r#"{"pause":{"retry_after":9}}"#,
            r#"{"revision":1,"paused":true}"#,
        ),
        (2, r#""resume""#, r#"{"revision":2,"paused":false}"#),
    ] {
        batch.push(Op::Operator(OperatorAction {
            at: clock.now(),
            revision: OperatorRevision::restore(revision),
            actor: Actor::Loopback,
            command: Recorded::of(&serde_json::from_str::<serde_json::Value>(command).unwrap())
                .unwrap(),
            state: Recorded::of(&serde_json::from_str::<serde_json::Value>(state).unwrap())
                .unwrap(),
        }));
    }
    store.commit(&batch).await.unwrap();
    // The log reads back oldest first, documents verbatim; a limit keeps the
    // newest.
    let reader = Reader::open(dir.path(), 1).await.unwrap();
    let actions = reader.operator_actions(PageLimit::DEFAULT).await.unwrap();
    assert_eq!(
        actions
            .iter()
            .map(|action| (
                action.revision,
                action.actor.as_str(),
                action.command.as_str()
            ))
            .collect::<Vec<_>>(),
        [
            (
                OperatorRevision::restore(1),
                r#"{"kind":"loopback"}"#,
                r#"{"pause":{"retry_after":9}}"#
            ),
            (
                OperatorRevision::restore(2),
                r#"{"kind":"loopback"}"#,
                r#""resume""#
            ),
        ]
    );
    assert!(
        actions
            .iter()
            .all(|action| action.at == UnixMillis::from_millis(1_000))
    );
    let newest = reader
        .operator_actions(PageLimit::new(1).unwrap())
        .await
        .unwrap();
    assert_eq!(newest.len(), 1);
    assert_eq!(newest[0].revision, OperatorRevision::restore(2));
    reader.close().await;
    store.close().await.unwrap();

    let (store, recovery) = Store::open(dir.path(), &clock, ids.clone()).await.unwrap();
    assert_eq!(
        recovery.operator,
        Some(StoredOperatorState {
            revision: OperatorRevision::restore(2),
            document: r#"{"revision":2,"paused":false}"#.into(),
        })
    );
    store.close().await.unwrap();

    // Even unreadable text comes back verbatim; the gateway decides.
    let options = SqliteConnectOptions::new().filename(dir.path().join("ren.sqlite3"));
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    sqlx::query("UPDATE operator_state SET revision = 5, state = 'not json'")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    let (store, recovery) = Store::open(dir.path(), &clock, ids).await.unwrap();
    assert_eq!(
        recovery.operator,
        Some(StoredOperatorState {
            revision: OperatorRevision::restore(5),
            document: "not json".into(),
        })
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn a_backup_is_a_readable_copy_that_copies_only_new_chunks() {
    use super::backup::Backup;
    let mut fixture = Fixture::new().await;
    let shared = large(200_000, 9);
    let first = fixture
        .record(CallSpec {
            request: &shared,
            ..CallSpec::default()
        })
        .await;
    let backup = Backup::new(fixture.dir.path(), fixture.store.sweep_gate());
    let report = backup.run().await.unwrap();
    assert_eq!(report.directory, fixture.dir.path().join("backup"));
    assert!(report.chunks_copied > 0);
    assert_eq!(report.chunks, report.chunks_copied);
    assert_eq!(report.chunks_removed, 0);
    // The backup is itself a data directory a reader can open.
    let copy = Reader::open(&report.directory, 1).await.unwrap();
    let key = copy.body_key(first, Direction::Request).await.unwrap();
    let mut body = Vec::new();
    copy.export(key, |bytes| {
        body.extend_from_slice(bytes);
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(body, shared);
    drop(copy);

    // Nothing new: nothing copied.
    let again = backup.run().await.unwrap();
    assert_eq!((again.chunks_copied, again.chunks), (0, report.chunks));
    // A new call copies only its own chunks.
    let unique = large(200_000, 10);
    let second = fixture
        .record(CallSpec {
            request: &unique,
            ..CallSpec::default()
        })
        .await;
    let third = backup.run().await.unwrap();
    assert!(third.chunks_copied > 0 && third.chunks_copied < third.chunks);
    // Chunks the history no longer references leave the backup too.
    let policy = RetentionPolicy {
        max_calls: 1,
        ..RetentionPolicy::default()
    };
    fixture
        .store
        .prune(&policy, fixture.clock.now().wall)
        .await
        .unwrap();
    let fourth = backup.run().await.unwrap();
    assert_eq!(fourth.chunks_copied, 0);
    assert!(fourth.chunks_removed > 0);
    // The backup holds exactly the chunks the history references.
    assert_eq!(
        fourth.chunks,
        fixture.reader.storage().await.unwrap().chunks
    );
    let copy = Reader::open(&fourth.directory, 1).await.unwrap();
    assert!(matches!(copy.call(first).await, Err(StoreError::NotFound)));
    assert!(copy.call(second).await.is_ok());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&fourth.directory), 0o700);
        assert_eq!(mode(&fourth.directory.join("ren.sqlite3")), 0o600);
        assert_eq!(mode(&fourth.directory.join("blobs.sqlite3")), 0o600);
    }
    assert!(!fourth.directory.join(".staging").exists());
}

#[tokio::test]
async fn a_held_sweep_gate_defers_chunk_removal() {
    let mut fixture = Fixture::new().await;
    for seed in [1, 2] {
        let request = large(100_000, seed);
        fixture
            .record(CallSpec {
                request: &request,
                ..CallSpec::default()
            })
            .await;
    }
    let policy = RetentionPolicy {
        max_calls: 1,
        ..RetentionPolicy::default()
    };
    let gate = fixture.store.sweep_gate();
    let hold = gate.hold().await;
    let pruned = fixture
        .store
        .prune(&policy, fixture.clock.now().wall)
        .await
        .unwrap();
    assert_eq!((pruned.calls, pruned.chunks), (1, 0));
    drop(hold);
    let pruned = fixture
        .store
        .prune(&policy, fixture.clock.now().wall)
        .await
        .unwrap();
    assert_eq!(pruned.calls, 0);
    assert!(pruned.chunks > 0);
}

#[tokio::test]
async fn sessions_total_their_errors_and_tokens_including_cached_input() {
    use ren_protocol::inspect::{InputAccounting, Usage};
    let mut fixture = Fixture::new().await;
    let usage = |accounting, input, cache_read, cache_creation, output| Usage {
        input_tokens: Some(input),
        cache_read_tokens: Some(cache_read),
        cache_creation_tokens: Some(cache_creation),
        output_tokens: Some(output),
        ..Usage::new(accounting)
    };
    fixture
        .record(CallSpec {
            session: Some(9),
            usage: Some(usage(InputAccounting::ExcludesCache, 10, 100, 5, 7)),
            ..CallSpec::default()
        })
        .await;
    fixture
        .record(CallSpec {
            session: Some(9),
            usage: Some(usage(InputAccounting::IncludesCache, 50, 40, 0, 3)),
            ..CallSpec::default()
        })
        .await;
    fixture
        .record(CallSpec {
            session: Some(9),
            status: 429,
            ..CallSpec::default()
        })
        .await;
    fixture
        .record(CallSpec {
            session: Some(10),
            ..CallSpec::default()
        })
        .await;
    let sessions = fixture.reader.sessions(PageLimit::DEFAULT).await.unwrap();
    let totals: Vec<_> = sessions
        .iter()
        .map(|s| (s.call_count, s.error_count, s.tokens.input, s.tokens.output))
        .collect();
    // Newest first. Cached input is added only where the provider left it out.
    assert_eq!(
        totals,
        [(1, 0, None, None), (3, 1, Some(115 + 50), Some(10))]
    );
}

#[tokio::test]
async fn usage_totals_by_hour_and_by_model_over_a_range() {
    use crate::query::TimeRange;
    use ren_protocol::inspect::{InputAccounting, Usage};
    use ren_recorder::time::UnixMillis;
    let mut fixture = Fixture::new().await;
    let usage = |input, cache_read, output| Usage {
        input_tokens: Some(input),
        cache_read_tokens: Some(cache_read),
        cache_creation_tokens: Some(0),
        output_tokens: Some(output),
        ..Usage::new(InputAccounting::ExcludesCache)
    };
    let start = fixture.clock.now().wall;
    fixture
        .record(CallSpec {
            usage: Some(usage(10, 90, 5)),
            ..CallSpec::default()
        })
        .await;
    fixture
        .record(CallSpec {
            status: 429,
            ..CallSpec::default()
        })
        .await;
    // Two hours later, a different provider.
    fixture.clock.advance(Duration::from_secs(2 * 3600));
    fixture
        .record(CallSpec {
            provider: Provider::Codex,
            usage: Some(usage(1, 0, 2)),
            ..CallSpec::default()
        })
        .await;

    let all = fixture.reader.usage(TimeRange::default()).await.unwrap();
    assert_eq!((all.totals.calls, all.totals.errors), (3, 1));
    assert_eq!(
        (
            all.totals.tokens.input,
            all.totals.tokens.output,
            all.totals.tokens.cache_read
        ),
        (Some(101), Some(7), Some(90))
    );
    // Hours with calls only, oldest first, each starting on the hour.
    assert_eq!(
        all.hours
            .iter()
            .map(|h| (h.usage.calls, h.start.as_millis() % 3_600_000))
            .collect::<Vec<_>>(),
        [(2, 0), (1, 0)]
    );
    // The fixture reports one model for every call; providers stay apart.
    assert_eq!(
        all.models
            .iter()
            .map(|m| (
                m.provider.label().to_owned(),
                m.model.clone(),
                m.usage.calls
            ))
            .collect::<Vec<_>>(),
        [
            ("anthropic".to_owned(), Some("claude-test".to_owned()), 2),
            ("codex".to_owned(), Some("claude-test".to_owned()), 1),
        ]
    );
    // A range keeps only the calls started in it; an empty one reports
    // nothing, with tokens "not reported" rather than zero.
    let early = TimeRange::new(
        Some(start),
        Some(UnixMillis::from_millis(start.as_millis() + 3_600_000)),
    )
    .unwrap();
    let first = fixture.reader.usage(early).await.unwrap();
    assert_eq!((first.totals.calls, first.hours.len()), (2, 1));
    let none = fixture
        .reader
        .usage(
            TimeRange::new(
                Some(UnixMillis::from_millis(1)),
                Some(UnixMillis::from_millis(2)),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!((none.totals.calls, none.totals.errors), (0, 0));
    assert_eq!(none.totals.tokens.input, None);
    assert!(none.hours.is_empty() && none.models.is_empty());
}

#[tokio::test]
async fn calls_narrow_by_session_client_and_kind() {
    use crate::query::{CallFilter, KindFilter};
    use ren_harness::Harness;
    let mut fixture = Fixture::new().await;
    let claude = fixture
        .record(CallSpec {
            session: Some(1),
            ..CallSpec::default()
        })
        .await;
    let count = fixture
        .record(CallSpec {
            session: Some(1),
            endpoint: Endpoint::CountTokens,
            ..CallSpec::default()
        })
        .await;
    let handshake = fixture
        .record(CallSpec {
            provider: Provider::Codex,
            session: Some(2),
            source: HintSource::CodexHeaders,
            endpoint: Endpoint::Responses,
            method: Method::Get,
            ..CallSpec::default()
        })
        .await;
    let codex = fixture
        .record(CallSpec {
            provider: Provider::Codex,
            session: Some(2),
            source: HintSource::CodexMessageMetadata,
            endpoint: Endpoint::Responses,
            method: Method::Message,
            ..CallSpec::default()
        })
        .await;
    let unattributed = fixture.record(CallSpec::default()).await;
    let ids = |filter: CallFilter| {
        let reader = fixture.reader.clone();
        async move {
            let mut ids: Vec<_> = reader
                .calls(&CallQuery {
                    filter,
                    ..CallQuery::default()
                })
                .await
                .unwrap()
                .calls
                .into_iter()
                .map(|call| call.id)
                .collect();
            ids.sort();
            ids
        }
    };
    let sorted = |mut ids: Vec<CallId>| {
        ids.sort();
        ids
    };
    let all = ids(CallFilter::default()).await;
    assert_eq!(all.len(), 5);
    let first_session = fixture
        .reader
        .call(claude)
        .await
        .unwrap()
        .call
        .session
        .unwrap();
    assert_eq!(
        ids(CallFilter {
            session: Some(first_session),
            ..CallFilter::default()
        })
        .await,
        sorted(vec![claude, count])
    );
    // Both Codex conventions count as Codex; an unattributed call is no client's.
    assert_eq!(
        ids(CallFilter {
            client: Some(Harness::Codex),
            ..CallFilter::default()
        })
        .await,
        sorted(vec![handshake, codex])
    );
    assert_eq!(
        ids(CallFilter {
            client: Some(Harness::ClaudeCode),
            ..CallFilter::default()
        })
        .await,
        sorted(vec![claude, count])
    );
    assert!(
        ids(CallFilter {
            client: Some(Harness::Pi),
            ..CallFilter::default()
        })
        .await
        .is_empty()
    );
    // Model calls exclude token counts and WebSocket handshakes; Other is the rest.
    assert_eq!(
        ids(CallFilter {
            kind: Some(KindFilter::Model),
            ..CallFilter::default()
        })
        .await,
        sorted(vec![claude, codex, unattributed])
    );
    assert_eq!(
        ids(CallFilter {
            kind: Some(KindFilter::Other),
            ..CallFilter::default()
        })
        .await,
        sorted(vec![count, handshake])
    );
    // Filters combine with AND.
    assert_eq!(
        ids(CallFilter {
            client: Some(Harness::Codex),
            kind: Some(KindFilter::Model),
            ..CallFilter::default()
        })
        .await,
        vec![codex]
    );
}

mod digests {
    use super::*;
    use crate::query::TimeRange;
    use crate::search::{ExposureQuery, SearchField, SearchQuery, SearchTerms};
    use ren_harness::{Harness, tools::InterpretedCall};
    use ren_protocol::activity::{
        Reading, RequestActivity, ResponseActivity, ResultStatus, ToolInput, ToolResult, ToolUse,
    };
    use ren_recorder::{
        digest::{CallDigest, DigestBudget},
        secrets::SecretScanner,
    };
    use serde_json::json;

    impl Fixture {
        /// Store the digest the recorder would derive from `reading`.
        async fn digest(&mut self, call: CallId, harness: Option<Harness>, reading: Reading) {
            let digest = CallDigest::seal(
                InterpretedCall::new(harness, reading),
                &SecretScanner::new().unwrap(),
                DigestBudget::STORED,
            );
            let mut batch = Batch::default();
            batch.push(Op::Digest {
                call,
                digest: Box::new(digest),
            });
            self.store.commit(&batch).await.unwrap();
        }
    }

    fn prompt(text: &str) -> RequestActivity {
        RequestActivity {
            user_text: vec![text.to_owned()],
            ..RequestActivity::default()
        }
    }

    fn query(text: &str) -> SearchQuery {
        SearchQuery {
            terms: SearchTerms::parse(text).unwrap(),
            session: None,
            client: None,
            field: None,
            time: TimeRange::default(),
            limit: PageLimit::new(20).unwrap(),
        }
    }

    #[tokio::test]
    async fn a_client_probe_is_bookkeeping_not_a_session_error() {
        use crate::query::{CallFilter, KindFilter};
        let mut fixture = Fixture::new().await;
        let probe = fixture
            .record(CallSpec {
                session: Some(1),
                status: 429,
                ..CallSpec::default()
            })
            .await;
        let work = fixture
            .record(CallSpec {
                session: Some(1),
                ..CallSpec::default()
            })
            .await;
        let reader = fixture.reader.clone();
        let kind = |kind| {
            let reader = reader.clone();
            async move {
                reader
                    .calls(&CallQuery {
                        filter: CallFilter {
                            kind: Some(kind),
                            ..CallFilter::default()
                        },
                        ..CallQuery::default()
                    })
                    .await
                    .unwrap()
                    .calls
                    .into_iter()
                    .map(|call| call.id)
                    .collect::<Vec<_>>()
            }
        };
        let errors = || {
            let reader = reader.clone();
            async move { reader.sessions(PageLimit::DEFAULT).await.unwrap()[0].error_count }
        };
        // Until its digest is derived, the probe is taken for work.
        assert_eq!(kind(KindFilter::Other).await, []);
        assert_eq!(errors().await, 1);
        fixture
            .digest(
                probe,
                Some(Harness::ClaudeCode),
                Reading {
                    request: RequestActivity {
                        max_output_tokens: Some(1),
                        ..prompt("quota")
                    },
                    response: ResponseActivity::default(),
                },
            )
            .await;
        fixture
            .digest(
                work,
                Some(Harness::ClaudeCode),
                Reading {
                    request: prompt("Fix the parser"),
                    response: ResponseActivity::default(),
                },
            )
            .await;
        assert_eq!(kind(KindFilter::Model).await, [work]);
        assert_eq!(kind(KindFilter::Other).await, [probe]);
        // Its refusal is still recorded, but is no error of the session's.
        assert_eq!(errors().await, 0);
    }

    #[tokio::test]
    async fn stored_digests_are_searched_filtered_and_listed() {
        let mut fixture = Fixture::new().await;
        let claude = fixture
            .record(CallSpec {
                session: Some(10),
                ..CallSpec::default()
            })
            .await;
        let codex = fixture
            .record(CallSpec {
                session: Some(20),
                source: HintSource::CodexHeaders,
                ..CallSpec::default()
            })
            .await;
        let undigested = fixture.record(CallSpec::default()).await;
        let key = format!("sk-ant-{}a1b2", "api03xY9z".repeat(5));
        fixture
            .digest(
                claude,
                Some(Harness::ClaudeCode),
                Reading {
                    request: prompt(&format!("Why does the migration fail with {key}?")),
                    response: ResponseActivity {
                        tool_uses: vec![ToolUse {
                            id: Some("t1".to_owned()),
                            kind: "tool_use".to_owned(),
                            name: "Bash".to_owned(),
                            input: ToolInput::Json(json!({ "command": "cargo xtask install-vps" })),
                        }],
                        ..ResponseActivity::default()
                    },
                },
            )
            .await;
        fixture
            .digest(
                codex,
                Some(Harness::Codex),
                Reading {
                    request: RequestActivity {
                        tool_results: vec![ToolResult {
                            id: "t1".to_owned(),
                            status: ResultStatus::Unreported,
                            output: "test migration ... FAILED".to_owned(),
                        }],
                        ..RequestActivity::default()
                    },
                    response: ResponseActivity::default(),
                },
            )
            .await;
        let reader = &fixture.reader;

        let page = reader.search(&query("migration")).await.unwrap();
        assert_eq!(page.indexed_calls, 2);
        let calls: Vec<_> = page.hits.iter().map(|hit| hit.call).collect();
        // Newest call first; the call without a digest is not searchable yet.
        assert_eq!(calls, [codex, claude]);
        assert_eq!(
            page.hits[1].snippet,
            "Why does the «migration» fail with «redacted: anthropic_key»?"
        );
        assert_eq!(page.hits[1].field, Stored::Known(SearchField::Prompt));
        assert_eq!(
            page.hits[1].client,
            Some(Stored::Known(Harness::ClaudeCode))
        );
        assert!(!page.hits.iter().any(|hit| hit.call == undigested));

        // Punctuation splits words: a hyphenated name is found as a phrase.
        let page = reader.search(&query(r#""install-vps""#)).await.unwrap();
        assert_eq!(page.hits.len(), 1);
        assert_eq!(page.hits[0].field, Stored::Known(SearchField::ToolInput));

        let mut filtered = query("migration");
        filtered.client = Some(Harness::Codex);
        assert_eq!(reader.search(&filtered).await.unwrap().hits[0].call, codex);
        filtered.client = None;
        filtered.field = Some(SearchField::Prompt);
        assert_eq!(reader.search(&filtered).await.unwrap().hits[0].call, claude);
        filtered.field = None;
        filtered.session = reader.call(codex).await.unwrap().call.session;
        let hits = reader.search(&filtered).await.unwrap().hits;
        assert_eq!(hits.iter().map(|hit| hit.call).collect::<Vec<_>>(), [codex]);
        filtered.session = None;
        let codex_start = reader.call(codex).await.unwrap().call.started_at;
        filtered.time = TimeRange::new(Some(codex_start), None).unwrap();
        assert_eq!(reader.search(&filtered).await.unwrap().hits.len(), 1);
        // An FTS5 operator in the text is searched for, never executed.
        assert!(
            reader
                .search(&query("NEAR("))
                .await
                .unwrap()
                .hits
                .is_empty()
        );

        let exposures = reader
            .exposures(&ExposureQuery {
                session: None,
                time: TimeRange::default(),
                limit: PageLimit::new(10).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(exposures.len(), 1);
        assert_eq!(exposures[0].call, claude);
        assert_eq!(exposures[0].hint, "sk-ant-…a1b2");
        assert_eq!(
            exposures[0].client,
            Some(Stored::Known(Harness::ClaudeCode))
        );
    }

    #[tokio::test]
    async fn turns_list_the_first_prompt_their_digests_hold() {
        let mut fixture = Fixture::new().await;
        // One Claude Code turn: a prompt, then a tool loop without one.
        let first = fixture
            .record(CallSpec {
                session: Some(5),
                ..CallSpec::default()
            })
            .await;
        let second = fixture
            .record(CallSpec {
                session: Some(5),
                ..CallSpec::default()
            })
            .await;
        fixture
            .digest(
                second,
                Some(Harness::ClaudeCode),
                Reading {
                    request: RequestActivity::default(),
                    response: ResponseActivity::default(),
                },
            )
            .await;
        let session = fixture
            .reader
            .call(first)
            .await
            .unwrap()
            .call
            .session
            .unwrap();
        let listed = fixture
            .reader
            .session_turns(session, PageLimit::new(10).unwrap())
            .await
            .unwrap();
        // Not derived yet: no prompt is claimed.
        assert_eq!(listed[0].prompt, None);
        fixture
            .digest(
                first,
                Some(Harness::ClaudeCode),
                Reading {
                    request: prompt(&"Fix the parser ".repeat(40)),
                    response: ResponseActivity::default(),
                },
            )
            .await;
        let listed = fixture
            .reader
            .session_turns(session, PageLimit::new(10).unwrap())
            .await
            .unwrap();
        let prompt = listed[0].prompt.as_ref().unwrap();
        assert_eq!(prompt.text.chars().count(), 300);
        // The user's words, trimmed.
        assert_eq!(prompt.total_chars(), 599);
        assert_eq!(listed[0].call_count, 2);
    }
}
