//! The tap's states against the queue alone: what each sequence of
//! transitions sends, in order. No runtime, recorder task, or store.

use super::*;
use crate::{
    capture::AttemptTerminal,
    id::SequentialIds,
    metrics::Counters,
    options::CaptureOptions,
    recording::Record,
    time::{ManualClock, UnixMillis},
};
use ren_protocol::route::Provider;
use tokio::sync::mpsc;

/// A queue sized for `units` lifecycle units, and what was sent to it.
struct Queue {
    shared: Arc<Shared>,
    receiver: mpsc::Receiver<Item>,
    counters: Arc<Counters>,
}

/// One sent record, by what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Sent {
    /// The call opened, with its first attempt if it made one.
    Open {
        attempt: bool,
    },
    Attempt {
        ordinal: u32,
    },
    Head,
    Data {
        track: Track,
        bytes: usize,
    },
    FinishAttempt {
        end: AttemptTerminal,
        response: (u64, BodyEnding),
    },
    FinishCall {
        request: (u64, BodyEnding),
        rejected: bool,
    },
    Close(ChannelEnd),
}

impl Queue {
    fn new(units: usize) -> Self {
        let counters = Arc::new(Counters::default());
        let (shared, receiver) = Shared::new(
            CaptureOptions::default(),
            units,
            counters.clone(),
            Arc::new(ManualClock::at(UnixMillis::from_millis(1_000))),
            Arc::new(SequentialIds::default()),
        );
        Self {
            shared: Arc::new(shared),
            receiver,
            counters,
        }
    }

    fn start(&self, attempts: usize) -> AttemptTap {
        AttemptTap::start(
            self.shared.clone(),
            ChannelId::generate(self.shared.ids.as_ref()),
            call_start(attempts),
            attempt(),
        )
    }

    fn sent(&mut self) -> Vec<Sent> {
        let mut sent = Vec::new();
        while let Ok(item) = self.receiver.try_recv() {
            sent.push(match item.into_record().0 {
                Record::Open { attempt, .. } => Sent::Open {
                    attempt: attempt.is_some(),
                },
                Record::Attempt(attempt) => Sent::Attempt {
                    ordinal: attempt.ordinal,
                },
                Record::Head(_) => Sent::Head,
                Record::Data(piece) => Sent::Data {
                    track: piece.track,
                    bytes: piece.bytes.len(),
                },
                Record::FinishAttempt { end, response, .. } => Sent::FinishAttempt {
                    end: end.terminal,
                    response: (response.observed_bytes, response.ending),
                },
                Record::FinishCall {
                    request, rejection, ..
                } => Sent::FinishCall {
                    request: (request.observed_bytes, request.ending),
                    rejected: rejection.is_some(),
                },
                Record::Close(close) => Sent::Close(close.end),
                other @ (Record::Operator(_)
                | Record::Session(_)
                | Record::Work(_)
                | Record::Run(_)
                | Record::RunEntry(_)
                | Record::History(_)) => panic!("unexpected record {other:?}"),
            });
        }
        sent
    }

    fn unrecorded(&self) -> u64 {
        let queue = crate::metrics::QueueMetrics {
            items: 0,
            capacity: 0,
            bytes: 0,
            byte_capacity: 0,
        };
        self.counters.snapshot(queue).omissions.unrecorded_calls
    }
}

fn call_start(attempts: usize) -> CallStart {
    CallStart {
        transport: Transport::Http,
        api: Provider::Anthropic.native_api(),
        endpoint: Endpoint::Messages,
        method: Method::Post,
        config_revision: ConfigRevision::new("rev"),
        attribution: Attribution::default(),
        caller: Caller::Unidentified,
        operator_revision: None,
        plan: None,
        attempts,
    }
}

fn attempt() -> AttemptStart {
    AttemptStart::unchanged(
        UpstreamId::default_for(Provider::Anthropic),
        Provider::Anthropic,
    )
}

const HEAD: Head = Head {
    status: 200,
    format: ResponseFormat::EventStream,
    encoding: ResponseEncoding::Identity,
};

fn complete() -> TerminalRecord {
    TerminalRecord {
        terminal: AttemptTerminal::BodyComplete,
        cause: None,
        phase: None,
    }
}

fn body(observed: u64, ending: BodyEnding) -> (u64, BodyEnding) {
    (observed, ending)
}

#[test]
fn a_call_records_its_attempt_head_bodies_and_terminals_in_order() {
    let mut queue = Queue::new(1);
    let mut tap = queue.start(1);
    tap.request(&Bytes::from_static(b"{}"));
    let mut tap = tap.head(HEAD);
    tap.data(Direction::Response, b"data: x\n\n");
    tap.ended(Direction::Response);
    tap.finish(complete());
    assert_eq!(
        queue.sent(),
        vec![
            Sent::Open { attempt: true },
            Sent::Data {
                track: Track::Request,
                bytes: 2
            },
            Sent::Head,
            Sent::Data {
                track: Track::Response,
                bytes: 9
            },
            Sent::FinishAttempt {
                end: AttemptTerminal::BodyComplete,
                response: body(9, BodyEnding::Ended),
            },
            Sent::FinishCall {
                request: body(2, BodyEnding::Ended),
                rejected: false,
            },
        ]
    );
}

#[test]
fn a_dropped_attempt_records_its_attempt_then_its_call_as_dropped() {
    let mut queue = Queue::new(1);
    let mut tap = queue.start(1);
    tap.request_data(b"part");
    drop(tap);
    assert_eq!(
        queue.sent()[1..],
        [
            Sent::Data {
                track: Track::Request,
                bytes: 4
            },
            Sent::FinishAttempt {
                end: AttemptTerminal::Interrupted,
                response: body(0, BodyEnding::Unfinished),
            },
            Sent::FinishCall {
                request: body(4, BodyEnding::Unfinished),
                rejected: false,
            },
        ]
    );
}

#[test]
fn a_dropped_response_is_interrupted_never_complete() {
    let mut queue = Queue::new(1);
    let mut tap = queue.start(1).head(HEAD);
    tap.data(Direction::Response, b"partial");
    drop(tap);
    let sent = queue.sent();
    assert_eq!(
        sent[sent.len() - 2..],
        [
            Sent::FinishAttempt {
                end: AttemptTerminal::Interrupted,
                response: body(7, BodyEnding::Unfinished),
            },
            Sent::FinishCall {
                request: body(0, BodyEnding::Unfinished),
                rejected: false,
            },
        ]
    );
}

#[test]
fn a_fallback_attempt_continues_the_call_its_first_attempt_handed_back() {
    let mut queue = Queue::new(2);
    let refused = queue.start(2).head(HEAD);
    let call: CallTap = refused.end(complete());
    let second = call.attempt(attempt()).head(HEAD);
    second.finish(complete());
    let kinds: Vec<_> = queue
        .sent()
        .into_iter()
        .map(|sent| match sent {
            Sent::FinishAttempt { .. } => "finish attempt",
            Sent::FinishCall { .. } => "finish call",
            Sent::Open { .. } => "open",
            Sent::Attempt { ordinal: 1 } => "attempt 1",
            Sent::Head => "head",
            other @ (Sent::Attempt { .. } | Sent::Data { .. } | Sent::Close(_)) => {
                panic!("unexpected {other:?}")
            }
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "open",
            "head",
            "finish attempt",
            "attempt 1",
            "head",
            "finish attempt",
            "finish call"
        ]
    );
    assert_eq!(queue.unrecorded(), 0);
}

#[test]
fn an_attempt_beyond_the_plan_is_forwarded_unrecorded_and_counted() {
    let mut queue = Queue::new(2);
    let call = queue.start(1).end(complete());
    let mut extra = call.attempt(attempt()).head(HEAD);
    extra.data(Direction::Response, b"not recorded");
    extra.finish(complete());
    // Only the planned attempt is in the record: the extra one's start,
    // head, bytes, and terminal are not.
    assert_eq!(
        queue.sent(),
        [
            Sent::Open { attempt: true },
            Sent::FinishAttempt {
                end: AttemptTerminal::BodyComplete,
                response: body(0, BodyEnding::Unfinished),
            },
            Sent::FinishCall {
                request: body(0, BodyEnding::Unfinished),
                rejected: false,
            },
        ]
    );
    assert_eq!(queue.unrecorded(), 1);
}

#[test]
fn a_call_holds_one_unit_per_planned_attempt() {
    let mut queue = Queue::new(2);
    let planned_twice = queue.start(2);
    let unrecorded = queue.start(1);
    assert_eq!(queue.unrecorded(), 1, "both units are the first call's");
    drop(unrecorded);
    drop(planned_twice);
    let sent = queue.sent();
    assert_eq!(
        sent.iter()
            .filter(|sent| matches!(sent, Sent::Open { .. }))
            .count(),
        1
    );
}

#[test]
fn a_refused_call_records_its_request_and_rejection() {
    let mut queue = Queue::new(1);
    let mut tap = CallTap::refused(
        queue.shared.clone(),
        ChannelId::generate(queue.shared.ids.as_ref()),
        call_start(0),
    );
    tap.request(&Bytes::from_static(b"{}"));
    tap.reject(CallRejection::Busy);
    assert_eq!(
        queue.sent(),
        [
            Sent::Open { attempt: false },
            Sent::Data {
                track: Track::Request,
                bytes: 2
            },
            Sent::FinishCall {
                request: body(2, BodyEnding::Ended),
                rejected: true,
            },
        ]
    );
}

#[test]
fn a_changed_request_is_recorded_when_its_attempt_starts() {
    let mut queue = Queue::new(1);
    let tap = AttemptTap::start(
        queue.shared.clone(),
        ChannelId::generate(queue.shared.ids.as_ref()),
        call_start(1),
        AttemptStart {
            body: AttemptRequest::Changed(Bytes::from_static(b"{\"x\":1}")),
            ..attempt()
        },
    );
    drop(tap);
    assert_eq!(
        queue.sent()[1],
        Sent::Data {
            track: Track::AttemptRequest,
            bytes: 7
        }
    );
}

#[test]
fn a_changed_request_shows_only_its_length_in_debug() {
    let body = AttemptRequest::Changed(Bytes::from_static(b"secret-token"));
    assert_eq!(format!("{body:?}"), "Changed(12 bytes)");
}

#[test]
fn a_dropped_channel_records_its_end_as_dropped_and_a_closed_one_once() {
    let mut queue = Queue::new(2);
    let dropped = ChannelTap::open(
        queue.shared.clone(),
        ChannelId::generate(queue.shared.ids.as_ref()),
    );
    drop(dropped);
    let mut closed = ChannelTap::open(
        queue.shared.clone(),
        ChannelId::generate(queue.shared.ids.as_ref()),
    );
    closed.unattributed(b"ping");
    closed.close(ChannelEnd::ClientClosed, Some(1000));
    assert_eq!(
        queue.sent(),
        [
            Sent::Close(ChannelEnd::Dropped),
            Sent::Close(ChannelEnd::ClientClosed)
        ]
    );
}
