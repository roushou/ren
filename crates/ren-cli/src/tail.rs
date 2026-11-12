//! `ren tail`'s pure core: the daemon's `/api/live` server-sent events,
//! decoded into typed events whatever their fragmentation, and the line a
//! call event prints. It performs no I/O.

use ren_protocol::{
    label::Stored,
    sse::{EventName, SseFramer, SseLimits, SseOutput},
};
use ren_recorder::{
    Observation, batch::ObservationKind, id::CallId, progress::Progress, time::UnixMillis,
};
use serde::Deserialize;
use std::fmt::Write as _;

/// What a live feed sends: committed observations, or those and progress
/// updates on streaming calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveUpdates {
    Observations,
    /// Also the reply so far on each streaming call, its tools, and whether
    /// it is thinking: never stored, never replayed, and skipped when the
    /// feed falls behind.
    WithProgress,
}

/// One live feed event.
#[derive(Debug, PartialEq, Eq)]
pub enum LiveEvent {
    /// A committed observation.
    Observation(Observation),
    /// An update on a streaming call, sent only with
    /// [`LiveUpdates::WithProgress`].
    Progress(Progress),
    /// The requested position was removed by retention; resume without it.
    Gone { oldest_retained: u64 },
    /// The feed fell behind and is catching up from storage.
    Lagged { resume_from: u64 },
    /// The daemon could not read its store; the feed ends.
    Failed(String),
    /// An event this CLI does not know, or one it could not decode. Kept
    /// so it is reported rather than silently dropped.
    Unrecognized { name: String, detail: String },
}

/// Decodes the feed's bytes, split however the network splits them.
pub struct LiveDecoder {
    framer: SseFramer,
}

#[derive(Deserialize)]
struct Gone {
    oldest_retained: u64,
}

#[derive(Deserialize)]
struct Lagged {
    resume_from: u64,
}

impl Default for LiveDecoder {
    fn default() -> Self {
        Self {
            framer: SseFramer::new(SseLimits::default()),
        }
    }
}

impl LiveDecoder {
    /// Appends the events completed by `bytes` to `events`, in order.
    pub fn feed(&mut self, bytes: &[u8], events: &mut Vec<LiveEvent>) {
        self.framer.feed(bytes, |output| {
            events.push(match output {
                SseOutput::Event { name, data } => Self::event(name, data),
                SseOutput::Discarded(discard) => LiveEvent::Unrecognized {
                    name: "discarded".to_owned(),
                    detail: format!("{discard:?}"),
                },
            });
        });
    }

    fn event(name: EventName<'_>, data: &[u8]) -> LiveEvent {
        let text = String::from_utf8_lossy(data);
        let unrecognized = |name: &str| LiveEvent::Unrecognized {
            name: name.to_owned(),
            detail: text.clone().into_owned(),
        };
        match name {
            EventName::Present("observation") => serde_json::from_slice(data)
                .map(LiveEvent::Observation)
                .unwrap_or_else(|_| unrecognized("observation")),
            EventName::Present("progress") => serde_json::from_slice(data)
                .map(LiveEvent::Progress)
                .unwrap_or_else(|_| unrecognized("progress")),
            EventName::Present("gone") => serde_json::from_slice::<Gone>(data)
                .map(|gone| LiveEvent::Gone {
                    oldest_retained: gone.oldest_retained,
                })
                .unwrap_or_else(|_| unrecognized("gone")),
            EventName::Present("lagged") => serde_json::from_slice::<Lagged>(data)
                .map(|lagged| LiveEvent::Lagged {
                    resume_from: lagged.resume_from,
                })
                .unwrap_or_else(|_| unrecognized("lagged")),
            EventName::Present("error") => LiveEvent::Failed(text.into_owned()),
            EventName::Present(other) => unrecognized(other),
            EventName::Absent => unrecognized("message"),
            EventName::Invalid => unrecognized("invalid"),
        }
    }
}

/// What a human-readable tail prints for an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    /// A call started: print it once its view is read.
    Started(CallId),
    /// A call ended, or was recovered after a restart.
    Ended(CallId, Ending),
    /// An operator changed the operator state.
    Operator,
    /// Attempt-level, channel, and run records: only `--json` prints them.
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    Finished,
    Recovered,
}

impl Shown {
    /// An observation of an unknown kind is shown with its raw label.
    pub fn of(observation: &Observation) -> Result<Self, &str> {
        let Stored::Known(kind) = &observation.kind else {
            return Err(observation.kind.label());
        };
        Ok(match (kind, observation.call) {
            (ObservationKind::CallOpened, Some(call)) => Self::Started(call),
            (ObservationKind::CallFinished, Some(call)) => Self::Ended(call, Ending::Finished),
            (ObservationKind::CallRecovered, Some(call)) => Self::Ended(call, Ending::Recovered),
            (ObservationKind::OperatorAction | ObservationKind::HistoryEdited, _) => Self::Operator,
            (
                ObservationKind::CallOpened
                | ObservationKind::CallFinished
                | ObservationKind::CallRecovered,
                None,
            )
            | (
                ObservationKind::AttemptStarted
                | ObservationKind::ResponseHead
                | ObservationKind::AttemptFinished
                | ObservationKind::ChannelClosed
                | ObservationKind::RunStarted
                | ObservationKind::RunOpened
                | ObservationKind::RunConfigured
                | ObservationKind::RunEnded
                | ObservationKind::RunRecovered,
                _,
            ) => Self::Hidden,
        })
    }
}

/// The parts of `GET /api/calls/{id}` a tail line shows. Labels are only
/// displayed, so they stay text.
#[derive(Debug, Deserialize)]
pub struct CallSummary {
    id: CallId,
    started_at: UnixMillis,
    ended_at: Option<UnixMillis>,
    provider: String,
    endpoint: String,
    model: Option<String>,
    session: Option<String>,
    status: Option<u16>,
    outcome: Option<String>,
    rejection: Option<String>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

/// `HH:MM:SS` in UTC.
pub struct ClockTime(pub UnixMillis);

impl std::fmt::Display for ClockTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let seconds = self.0.as_millis() / 1000 % 86_400;
        write!(
            f,
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    }
}

impl CallSummary {
    /// The first 8 characters of an identifier.
    fn short(id: &str) -> &str {
        id.get(..8).unwrap_or(id)
    }

    /// `14:30:45  started   2903a122  anthropic messages  session dadfe1c5`
    pub fn started(&self, at: UnixMillis) -> String {
        let mut line = format!(
            "{}  started   {}  {} {}",
            ClockTime(at),
            Self::short(&self.id.to_string()),
            self.provider,
            self.endpoint
        );
        if let Some(session) = &self.session {
            let _ = write!(line, "  session {}", Self::short(session));
        }
        line
    }

    /// `14:30:47  finished  2903a122  anthropic claude-opus-5-5  completed 200  in 1234 out 56  1.9s`
    pub fn ended(&self, at: UnixMillis, ending: Ending) -> String {
        let verb = match ending {
            Ending::Finished => "finished ",
            Ending::Recovered => "recovered",
        };
        let mut line = format!(
            "{}  {verb} {}  {} {}",
            ClockTime(at),
            Self::short(&self.id.to_string()),
            self.provider,
            self.model.as_deref().unwrap_or(&self.endpoint),
        );
        match (&self.rejection, &self.outcome) {
            (Some(rejection), _) => {
                let _ = write!(line, "  rejected {rejection}");
            }
            (None, Some(outcome)) => {
                let _ = write!(line, "  {outcome}");
            }
            (None, None) => line.push_str("  no outcome"),
        }
        if let Some(status) = self.status {
            let _ = write!(line, " {status}");
        }
        if let Some(usage) = &self.usage {
            let count = |n: Option<u64>| n.map_or_else(|| "?".to_owned(), |n| n.to_string());
            let _ = write!(
                line,
                "  in {} out {}",
                count(usage.input_tokens),
                count(usage.output_tokens)
            );
        }
        if let Some(ended) = self.ended_at {
            let millis = ended
                .as_millis()
                .saturating_sub(self.started_at.as_millis());
            let _ = write!(line, "  {}.{}s", millis / 1000, millis % 1000 / 100);
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STREAM: &str = concat!(
        ": keep-alive\n\n",
        "event: observation\nid: 7\ndata: {\"seq\":7,\"at\":1793974238191,\"kind\":\"call_opened\",\"channel\":\"4cebe924-e644-4e00-9d22-b6bc6e62a420\",\"call\":\"2903a122-cd5b-4e63-b54f-2d95732c609e\",\"attempt\":null}\n\n",
        "event: observation\r\nid: 8\r\ndata: {\"seq\":8,\"at\":1793974238193,\"kind\":\"call_teleported\",\"channel\":\"4cebe924-e644-4e00-9d22-b6bc6e62a420\",\"call\":null,\"attempt\":null}\r\n\r\n",
        "event: progress\ndata: {\"call\":\"2903a122-cd5b-4e63-b54f-2d95732c609e\",\"reply\":\"Looking at \\u00e9tat\",\"cut\":true,\"tools\":[\"Read\"],\"thinking\":false}\n\n",
        "event: progress\ndata: {\"call\":\"not-a-call\"}\n\n",
        "event: lagged\ndata: {\"resume_from\":8}\n\n",
        "event: gone\ndata: {\"oldest_retained\":40}\n\n",
        "event: error\ndata: store unavailable\n\n",
        "event: surprise\ndata: {}\n\n",
    );

    fn decode(chunks: &[&[u8]]) -> Vec<LiveEvent> {
        let mut decoder = LiveDecoder::default();
        let mut events = Vec::new();
        for chunk in chunks {
            decoder.feed(chunk, &mut events);
        }
        events
    }

    #[test]
    fn decodes_every_event_kind_and_keeps_unknown_ones() {
        let events = decode(&[STREAM.as_bytes()]);
        assert_eq!(events.len(), 8);
        let LiveEvent::Observation(opened) = &events[0] else {
            panic!("{:?}", events[0]);
        };
        assert_eq!(opened.seq, 7);
        assert_eq!(
            Shown::of(opened),
            Ok(Shown::Started(
                "2903a122-cd5b-4e63-b54f-2d95732c609e".parse().unwrap()
            ))
        );
        let LiveEvent::Observation(unknown) = &events[1] else {
            panic!("{:?}", events[1]);
        };
        assert_eq!(Shown::of(unknown), Err("call_teleported"));
        assert_eq!(
            events[2],
            LiveEvent::Progress(Progress {
                call: "2903a122-cd5b-4e63-b54f-2d95732c609e".parse().unwrap(),
                reply: "Looking at état".to_owned(),
                cut: true,
                tools: vec!["Read".to_owned()],
                thinking: false,
            })
        );
        assert_eq!(
            events[3],
            LiveEvent::Unrecognized {
                name: "progress".to_owned(),
                detail: "{\"call\":\"not-a-call\"}".to_owned()
            }
        );
        assert_eq!(events[4], LiveEvent::Lagged { resume_from: 8 });
        assert_eq!(
            events[5],
            LiveEvent::Gone {
                oldest_retained: 40
            }
        );
        assert_eq!(events[6], LiveEvent::Failed("store unavailable".to_owned()));
        assert_eq!(
            events[7],
            LiveEvent::Unrecognized {
                name: "surprise".to_owned(),
                detail: "{}".to_owned()
            }
        );
    }

    #[test]
    fn fragmentation_never_changes_the_events() {
        let whole = decode(&[STREAM.as_bytes()]);
        let bytes = STREAM.as_bytes();
        for split in 0..=bytes.len() {
            let (a, b) = bytes.split_at(split);
            assert_eq!(decode(&[a, b]), whole, "split at {split}");
        }
        let single: Vec<&[u8]> = bytes.chunks(1).collect();
        assert_eq!(decode(&single), whole);
    }

    #[test]
    fn call_lines_show_what_happened() {
        let finished: CallSummary = serde_json::from_value(serde_json::json!({
            "id": "2903a122-cd5b-4e63-b54f-2d95732c609e",
            "started_at": 1793974238191_u64, "ended_at": 1793974240123_u64,
            "provider": "anthropic", "endpoint": "messages", "model": "claude-opus-5-5",
            "session": "dadfe1c5-d985-4bf8-9169-ae5da923ca89", "status": 200,
            "outcome": "completed", "rejection": null,
            "usage": { "input_tokens": 1234, "output_tokens": 56 },
            "state": "finished", "unrelated": [1, 2, 3]
        }))
        .unwrap();
        let at = UnixMillis::from_millis(1793974240123);
        assert_eq!(
            finished.started(UnixMillis::from_millis(1793974238191)),
            "14:10:38  started   2903a122  anthropic messages  session dadfe1c5"
        );
        assert_eq!(
            finished.ended(at, Ending::Finished),
            "14:10:40  finished  2903a122  anthropic claude-opus-5-5  completed 200  in 1234 out 56  1.9s"
        );
        let refused: CallSummary = serde_json::from_value(serde_json::json!({
            "id": "2903a122-cd5b-4e63-b54f-2d95732c609e",
            "started_at": 0, "ended_at": null, "provider": "codex", "endpoint": "responses",
            "model": null, "session": null, "status": 503, "outcome": null,
            "rejection": "paused", "usage": null
        }))
        .unwrap();
        assert_eq!(
            refused.ended(UnixMillis::from_millis(0), Ending::Recovered),
            "00:00:00  recovered 2903a122  codex responses  rejected paused 503"
        );
    }
}
