//! The core alone: records in, operations out. No runtime, queue, or store.

use super::*;
use crate::batch::ObservationKind;
use crate::capture::{AttemptPhase, AttemptTerminal, BodyEnding};
use crate::id::{ChannelId, SequentialIds};
use crate::routing::{ConfigRevision, UpstreamId};
use crate::time::{Moment, UnixMillis};
use ren_harness::attribution::{Attribution, HintSource, SessionHint};
use ren_harness::tools::CallPurpose;
use ren_protocol::encoding::{ResponseEncoding, ResponseFormat};
use ren_protocol::native::{NativeSessionId, NativeTurnId};
use ren_protocol::route::{Method, Provider, Transport, Wire};
use std::time::Duration;

/// Builds one call's records with sequential identities and a moving clock.
struct Script {
    ids: SequentialIds,
    channel: ChannelId,
    call: CallId,
    tick: u64,
}

impl Script {
    fn new() -> Self {
        let ids = SequentialIds::default();
        let channel = ChannelId::generate(&ids);
        let call = CallId::generate(&ids);
        Self {
            ids,
            channel,
            call,
            tick: 0,
        }
    }

    fn stamp(&mut self) -> Stamp {
        self.tick += 1;
        Stamp {
            at: Moment {
                wall: UnixMillis::from_millis(1_000 + self.tick),
                monotonic: Monotonic::from_elapsed(Duration::from_millis(self.tick)),
            },
            channel: self.channel,
            call: self.call,
        }
    }

    fn attempt(&mut self, ordinal: u32) -> StartAttempt {
        StartAttempt {
            stamp: self.stamp(),
            attempt: AttemptId::generate(&self.ids),
            ordinal,
            upstream: UpstreamId::default_for(Provider::Anthropic),
            provider: Provider::Anthropic,
            changes: None,
            body: AttemptBody::Original,
        }
    }

    fn open(&mut self, first: StartAttempt) -> Record {
        Record::Open {
            open: Box::new(OpenCall {
                stamp: self.stamp(),
                transport: Transport::Http,
                provider: Provider::Anthropic,
                wire: Wire::AnthropicMessages,
                endpoint: Endpoint::Messages,
                method: Method::Post,
                config_revision: ConfigRevision::new("rev"),
                attribution: Attribution {
                    hint: Some(SessionHint {
                        source: HintSource::ClaudeCodeHeaders,
                        session: NativeSessionId::new(uuid::Uuid::from_u128(9)).unwrap(),
                        turn: NativeTurnId::new(uuid::Uuid::from_u128(10)),
                        thread: None,
                    }),
                    issues: Vec::new(),
                },
                caller: crate::caller::Caller::Unidentified,
                operator_revision: None,
                plan: None,
            }),
            attempt: Some(first),
            endpoint: Endpoint::Messages,
        }
    }

    fn head(&mut self, attempt: AttemptId, status: u16) -> Record {
        Record::Head(ResponseHead {
            stamp: self.stamp(),
            attempt,
            status,
            format: ResponseFormat::EventStream,
            encoding: ResponseEncoding::Identity,
        })
    }

    fn data(&mut self, track: Track, offset: u64, bytes: &[u8]) -> Record {
        self.tick += 1;
        Record::Data(Piece {
            call: self.call,
            track,
            offset,
            bytes: Bytes::copy_from_slice(bytes),
            enqueued: Monotonic::from_elapsed(Duration::from_millis(self.tick)),
            message: None,
        })
    }

    /// A response piece enqueued `ms` after the clock's origin.
    fn data_at(&mut self, offset: u64, bytes: &[u8], ms: u64) -> Record {
        Record::Data(Piece {
            call: self.call,
            track: Track::Response,
            offset,
            bytes: Bytes::copy_from_slice(bytes),
            enqueued: Monotonic::from_elapsed(Duration::from_millis(ms)),
            message: None,
        })
    }

    fn finish_attempt(&mut self, attempt: AttemptId, observed: u64) -> Record {
        Record::FinishAttempt {
            stamp: self.stamp(),
            attempt,
            end: TerminalRecord {
                terminal: AttemptTerminal::BodyComplete,
                cause: None,
                phase: Some(AttemptPhase::Response),
            },
            response: BodyEnd {
                observed_bytes: observed,
                ending: BodyEnding::Ended,
            },
            request: None,
        }
    }

    fn finish_call(&mut self, observed: u64) -> Record {
        Record::FinishCall {
            stamp: self.stamp(),
            request: BodyEnd {
                observed_bytes: observed,
                ending: BodyEnding::Ended,
            },
            rejection: None,
        }
    }
}

fn request(prompt: &str) -> Vec<u8> {
    serde_json::json!({ "messages": [{ "role": "user", "content": prompt }] })
        .to_string()
        .into_bytes()
}

fn stream(reply: &str) -> Vec<u8> {
    let event = |name: &str, data: serde_json::Value| format!("event: {name}\ndata: {data}\n\n");
    [
        event("message_start", serde_json::json!({ "type": "message_start", "message": { "model": "claude-test" } })),
        event("content_block_start", serde_json::json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } })),
        event("content_block_delta", serde_json::json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": reply } })),
        event("content_block_stop", serde_json::json!({ "type": "content_block_stop", "index": 0 })),
        event("message_delta", serde_json::json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" } })),
        event("message_stop", serde_json::json!({ "type": "message_stop" })),
    ]
    .concat()
    .into_bytes()
}

fn core() -> RecorderCore {
    RecorderCore::new(Arc::new(SecretScanner::new().unwrap()))
}

fn len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap()
}

/// The kinds of the lifecycle operations, in order.
fn lifecycle(batch: &Batch) -> Vec<ObservationKind> {
    batch
        .operations()
        .iter()
        .filter_map(|op| match op {
            Op::OpenCall(_) => Some(ObservationKind::CallOpened),
            Op::StartAttempt(_) => Some(ObservationKind::AttemptStarted),
            Op::ResponseHead(_) => Some(ObservationKind::ResponseHead),
            Op::FinishAttempt(_) => Some(ObservationKind::AttemptFinished),
            Op::FinishCall(_) => Some(ObservationKind::CallFinished),
            Op::Segments { .. }
            | Op::Message { .. }
            | Op::CloseChannel(_)
            | Op::Operator(_)
            | Op::Run(_)
            | Op::History(_)
            | Op::Digest { .. } => None,
        })
        .collect()
}

fn digest(batch: &Batch) -> Option<&CallDigest> {
    batch.operations().iter().find_map(|op| match op {
        Op::Digest { digest, .. } => Some(digest.as_ref()),
        Op::OpenCall(_)
        | Op::StartAttempt(_)
        | Op::ResponseHead(_)
        | Op::Segments { .. }
        | Op::Message { .. }
        | Op::FinishAttempt(_)
        | Op::FinishCall(_)
        | Op::CloseChannel(_)
        | Op::Operator(_)
        | Op::Run(_)
        | Op::History(_) => None,
    })
}

fn inspection(batch: &Batch) -> Option<&InspectionRecord> {
    batch.operations().iter().find_map(|op| match op {
        Op::FinishAttempt(finish) => finish.inspection.as_deref(),
        Op::OpenCall(_)
        | Op::StartAttempt(_)
        | Op::ResponseHead(_)
        | Op::Segments { .. }
        | Op::Message { .. }
        | Op::FinishCall(_)
        | Op::CloseChannel(_)
        | Op::Operator(_)
        | Op::Run(_)
        | Op::History(_)
        | Op::Digest { .. } => None,
    })
}

#[test]
fn a_call_becomes_its_lifecycle_chunks_inspection_and_digest() {
    let mut script = Script::new();
    let mut core = core();
    let first = script.attempt(0);
    let attempt = first.attempt;
    let (request, response) = (request("Fix the parser"), stream("Fixed."));
    core.accept(script.open(first));
    core.accept(script.data(Track::Request, 0, &request));
    core.accept(script.head(attempt, 200));
    // Pieces split mid-event, as the network delivers them.
    let mut offset = 0;
    for piece in response.chunks(7) {
        core.accept(script.data(Track::Response, offset, piece));
        offset += len(piece);
    }
    core.accept(script.finish_attempt(attempt, len(&response)));
    core.accept(script.finish_call(len(&request)));
    let batch = core.take_batch();
    assert_eq!(
        lifecycle(&batch),
        [
            ObservationKind::CallOpened,
            ObservationKind::AttemptStarted,
            ObservationKind::ResponseHead,
            ObservationKind::AttemptFinished,
            ObservationKind::CallFinished,
        ]
    );
    // Every captured byte is in a chunk.
    let chunked: usize = batch.chunks().iter().map(Bytes::len).sum();
    assert_eq!(chunked, request.len() + response.len());
    let inspected = inspection(&batch).unwrap();
    assert!(
        inspected.limitations.is_empty(),
        "{:?}",
        inspected.limitations
    );
    assert_eq!(inspected.model.as_deref(), Some("claude-test"));
    let digest = digest(&batch).unwrap();
    assert_eq!(digest.prompt.as_ref().unwrap().text, "Fix the parser");
    assert_eq!(digest.reply.as_ref().unwrap().text, "Fixed.");
    // Nothing is left behind for the next batch.
    assert!(core.take_batch().is_empty());
}

#[test]
fn claude_codes_quota_probe_is_digested_as_bookkeeping() {
    let mut script = Script::new();
    let mut core = core();
    let first = script.attempt(0);
    let attempt = first.attempt;
    let request = serde_json::json!({
        "model": "claude-test",
        "max_tokens": 1,
        "messages": [{ "role": "user", "content": "quota" }],
    })
    .to_string()
    .into_bytes();
    let refusal = br#"{"type":"error","error":{"type":"rate_limit_error","message":"Error"}}"#;
    core.accept(script.open(first));
    core.accept(script.data(Track::Request, 0, &request));
    core.accept(script.head(attempt, 429));
    core.accept(script.data(Track::Response, 0, refusal));
    core.accept(script.finish_attempt(attempt, len(refusal)));
    core.accept(script.finish_call(len(&request)));
    let batch = core.take_batch();
    let digest = digest(&batch).unwrap();
    assert_eq!(digest.purpose, CallPurpose::QuotaProbe);
    // The probe's message is Claude Code's, not the user's.
    assert_eq!(digest.prompt, None);
}

#[test]
fn a_gap_in_the_response_is_inspected_as_one() {
    let mut script = Script::new();
    let mut core = core();
    let first = script.attempt(0);
    let attempt = first.attempt;
    let response = stream("Fixed.");
    core.accept(script.open(first));
    core.accept(script.head(attempt, 200));
    let (head, tail) = response.split_at(40);
    core.accept(script.data(Track::Response, 0, head));
    // The bytes in between were omitted before reaching the recorder.
    core.accept(script.data(Track::Response, len(head) + 5, &tail[5..]));
    core.accept(script.finish_attempt(attempt, len(&response)));
    core.accept(script.finish_call(0));
    let batch = core.take_batch();
    let limitations: Vec<String> = inspection(&batch)
        .unwrap()
        .limitations
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        limitations
            .iter()
            .any(|limitation| limitation == "capture_gap"),
        "{limitations:?}"
    );
}

#[test]
fn the_final_attempt_is_the_one_a_digest_reads() {
    let mut script = Script::new();
    let mut core = core();
    let first = script.attempt(0);
    let refused = first.attempt;
    let request = request("Try again");
    core.accept(script.open(first));
    core.accept(script.data(Track::Request, 0, &request));
    core.accept(script.head(refused, 429));
    let limited = stream("Rate limited.");
    core.accept(script.data(Track::Response, 0, &limited));
    core.accept(script.finish_attempt(refused, len(&limited)));
    let second = script.attempt(1);
    let answered = second.attempt;
    core.accept(Record::Attempt(second));
    core.accept(script.head(answered, 200));
    let response = stream("Done on the fallback.");
    core.accept(script.data(Track::Response, 0, &response));
    core.accept(script.finish_attempt(answered, len(&response)));
    core.accept(script.finish_call(len(&request)));
    let batch = core.take_batch();
    assert_eq!(
        digest(&batch).unwrap().reply.as_ref().unwrap().text,
        "Done on the fallback."
    );
}

#[test]
fn data_for_an_unknown_call_adds_nothing() {
    let mut script = Script::new();
    let mut core = core();
    core.accept(script.data(Track::Request, 0, b"orphan"));
    assert!(core.take_batch().is_empty());
}

#[test]
fn a_commit_cuts_streaming_responses_but_keeps_the_call_open() {
    let mut script = Script::new();
    let mut core = core();
    let first = script.attempt(0);
    let attempt = first.attempt;
    core.accept(script.open(first));
    core.accept(script.head(attempt, 200));
    core.accept(script.data(Track::Response, 0, b"event: ping\ndata: {}\n\n"));
    // A batch taken mid-stream carries the response so far, so live views
    // advance; the call continues in the next batch.
    let mid = core.take_batch();
    assert_eq!(mid.chunks().iter().map(Bytes::len).sum::<usize>(), 22);
    core.accept(script.finish_attempt(attempt, 22));
    core.accept(script.finish_call(0));
    let end = core.take_batch();
    assert_eq!(
        lifecycle(&end),
        [
            ObservationKind::AttemptFinished,
            ObservationKind::CallFinished
        ]
    );
    assert!(digest(&end).is_some());
}

#[test]
fn a_streaming_reply_sends_spaced_updates_that_grow_into_it() {
    let mut script = Script::new();
    let mut core = core();
    let first = script.attempt(0);
    let attempt = first.attempt;
    let reply = "One. Two. Three. Four. Five.";
    // The reply arrives in three text deltas, 100 ms and then 300 ms apart.
    let event = |name: &str, data: serde_json::Value| format!("event: {name}\ndata: {data}\n\n");
    let delta = |text: &str| {
        event(
            "content_block_delta",
            serde_json::json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": text } }),
        )
    };
    let pieces = [
        event(
            "message_start",
            serde_json::json!({ "type": "message_start", "message": { "model": "claude-test" } }),
        ) + &event(
            "content_block_start",
            serde_json::json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
        ) + &delta("One. Two. "),
        delta("Three. "),
        delta("Four. Five."),
    ];
    core.accept(script.open(first));
    core.accept(script.head(attempt, 200));
    let mut offset = 0;
    for (piece, ms) in pieces.iter().zip([0, 100, 400]) {
        core.accept(script.data_at(offset, piece.as_bytes(), ms));
        offset += len(piece.as_bytes());
    }
    let updates = core.take_progress();
    // The second piece came too soon after the first; the third did not.
    let replies: Vec<&str> = updates.iter().map(|update| update.reply.as_str()).collect();
    assert_eq!(replies, ["One. Two. ", reply]);
    assert!(
        updates
            .iter()
            .all(|update| update.call == script.call && !update.cut)
    );
    // Taken once; a commit sends them.
    assert!(core.take_progress().is_empty());
}
