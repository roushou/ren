//! Inspection of one response body, fed incrementally as it is captured:
//! the inspection report and the response's activity (tool calls and reply
//! text), each frame decoded once and shown to both.
//!
//! The caller feeds contiguous captured bytes with [`CallInspector::feed`],
//! reports missing capture with [`CallInspector::gap`], and ends with
//! [`CallInspector::finish`]. Nothing is spliced across a gap, and HTTP body
//! completion stays independent of model completion.

use super::{EventClass, FormatSource, InspectionReport, Inspector, Limitation, ReportLimits};
use crate::activity::{ActivityLimits, ResponseActivity, ResponseReader};
use crate::{
    chat,
    encoding::{DecodeFailure, Decompressor, ResponseEncoding, ResponseFormat},
    messages, responses,
    route::Endpoint,
    sse::{EventName, SseFramer, SseLimits, SseOutput, StreamEnd},
};

/// The native protocol of an inspectable endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Anthropic Messages (`/v1/messages`).
    Messages,
    /// OpenAI Responses, public or Codex (`/responses`).
    Responses,
    /// OpenAI Chat Completions (`/v1/chat/completions`).
    ChatCompletions,
}

impl Protocol {
    /// The protocol an endpoint's responses use, if Ren inspects it.
    pub fn for_endpoint(endpoint: Endpoint) -> Option<Self> {
        match endpoint {
            Endpoint::Messages => Some(Self::Messages),
            Endpoint::Responses => Some(Self::Responses),
            Endpoint::ChatCompletions => Some(Self::ChatCompletions),
            Endpoint::CountTokens | Endpoint::Models | Endpoint::ResponsesCompact => None,
        }
    }

    fn observe_event(self, name: EventName<'_>, data: &[u8], inspector: &mut Inspector) {
        let class = match self {
            Self::Messages => match messages::Event::decode(name, data) {
                Ok(event) => event.observe(&mut |observation| inspector.observe(observation)),
                Err(_) => EventClass::Malformed,
            },
            Self::Responses => match responses::Event::decode(name, data) {
                Ok(event) => event.observe(&mut |observation| inspector.observe(observation)),
                Err(_) => EventClass::Malformed,
            },
            // Chunks carry no event names.
            Self::ChatCompletions => match chat::Event::decode(data) {
                Ok(event) => event.observe(&mut |observation| inspector.observe(observation)),
                Err(_) => EventClass::Malformed,
            },
        };
        inspector.count(class);
    }

    fn observe_document(self, bytes: &[u8], inspector: &mut Inspector) {
        let class = match self {
            Self::Messages => match messages::Document::decode(bytes) {
                Ok(document) => document.observe(&mut |observation| inspector.observe(observation)),
                Err(_) => EventClass::Malformed,
            },
            Self::Responses => match responses::Document::decode(bytes) {
                Ok(document) => document.observe(&mut |observation| inspector.observe(observation)),
                Err(_) => EventClass::Malformed,
            },
            Self::ChatCompletions => match chat::Document::decode(bytes) {
                Ok(document) => document.observe(&mut |observation| inspector.observe(observation)),
                Err(_) => EventClass::Malformed,
            },
        };
        inspector.count(class);
    }
}

/// How the HTTP body ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyEnd {
    /// End of body observed (EOF or complete framing).
    Complete,
    /// The body was cut off: disconnect, cancellation, or error.
    Unfinished,
}

/// Bounds for one call's inspection. These bound inspection work only; they
/// never affect forwarding or recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InspectionLimits {
    pub sse: SseLimits,
    /// Largest non-streaming JSON body that is parsed.
    pub max_document_bytes: usize,
    /// Largest decoded body, after decompression.
    pub max_decoded_bytes: u64,
    pub report: ReportLimits,
    pub activity: ActivityLimits,
}

impl Default for InspectionLimits {
    fn default() -> Self {
        Self {
            sse: SseLimits::default(),
            max_document_bytes: 16 * 1024 * 1024,
            max_decoded_bytes: 64 * 1024 * 1024,
            report: ReportLimits::default(),
            activity: ActivityLimits::default(),
        }
    }
}

/// Inspects one response body.
#[derive(Debug)]
pub struct CallInspector {
    readers: Readers,
    state: State,
}

/// What one response's frames feed.
#[derive(Debug)]
struct Readers {
    inspector: Inspector,
    /// `None` when the body cannot be interpreted.
    activity: Option<ResponseReader>,
}

/// What a response showed: its inspection report and its activity.
#[derive(Debug, Clone, PartialEq)]
pub struct ResponseReading {
    pub inspection: InspectionReport,
    pub activity: ResponseActivity,
}

impl Readers {
    fn event(&mut self, protocol: Protocol, name: EventName<'_>, data: &[u8]) {
        protocol.observe_event(name, data, &mut self.inspector);
        if let Some(activity) = &mut self.activity {
            activity.event_bytes(data);
        }
    }

    fn document(&mut self, protocol: Protocol, bytes: &[u8]) {
        protocol.observe_document(bytes, &mut self.inspector);
        if let Some(activity) = &mut self.activity {
            activity.document_bytes(bytes);
        }
    }

    fn limit(&mut self, reason: Limitation) {
        self.inspector.limit(reason);
    }

    fn finish(self) -> ResponseReading {
        ResponseReading {
            inspection: self.inspector.finish(),
            activity: self
                .activity
                .map(ResponseReader::finish)
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug)]
enum State {
    /// The body cannot be interpreted; bytes are ignored.
    Unavailable {
        format: FormatSource,
        reason: Limitation,
    },
    Active(Box<Active>),
    /// A gap, limit, or decoding failure ended interpretation. The reason is
    /// already a limitation; later bytes are ignored.
    Stopped,
}

#[derive(Debug)]
struct Active {
    protocol: Protocol,
    limits: InspectionLimits,
    decoder: Decompressor,
    body: Body,
    decoded_bytes: u64,
}

#[derive(Debug)]
enum Body {
    Stream(SseFramer),
    Document(Vec<u8>),
    Messages(MessageBuffer),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    Stream,
    Document,
    Messages,
}

/// WebSocket messages recorded back to back. The caller announces each
/// message's length with [`CallInspector::message`] before feeding its bytes,
/// so boundaries come from the socket framing, never from guessing.
#[derive(Debug, Default)]
struct MessageBuffer {
    bytes: Vec<u8>,
    /// Bytes still to come of the current message.
    remaining: usize,
    current: Message,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Message {
    /// Between messages: bytes now have no announced message.
    #[default]
    None,
    Buffering,
    /// Too large to inspect; its bytes are skipped.
    Skipping,
}

impl CallInspector {
    pub fn new(
        protocol: Protocol,
        format: ResponseFormat,
        encoding: ResponseEncoding,
        limits: InspectionLimits,
    ) -> Self {
        let selected = match (format, protocol) {
            (
                ResponseFormat::EventStream,
                Protocol::Messages | Protocol::Responses | Protocol::ChatCompletions,
            ) => Some((BodyKind::Stream, FormatSource::Declared)),
            (
                ResponseFormat::Json,
                Protocol::Messages | Protocol::Responses | Protocol::ChatCompletions,
            ) => Some((BodyKind::Document, FormatSource::Declared)),
            // Codex can omit Content-Type on /responses; it streams SSE.
            (ResponseFormat::Unspecified, Protocol::Responses) => {
                Some((BodyKind::Stream, FormatSource::Inferred))
            }
            (ResponseFormat::WebSocketMessages, Protocol::Responses) => {
                Some((BodyKind::Messages, FormatSource::Declared))
            }
            (
                ResponseFormat::Unspecified | ResponseFormat::WebSocketMessages,
                Protocol::Messages | Protocol::ChatCompletions,
            )
            | (
                ResponseFormat::Other,
                Protocol::Messages | Protocol::Responses | Protocol::ChatCompletions,
            ) => None,
        };
        let Some((kind, source)) = selected else {
            return Self::unavailable(FormatSource::Declared, Limitation::UnsupportedFormat);
        };
        let Ok(decoder) = Decompressor::new(encoding) else {
            return Self::unavailable(source, Limitation::UnsupportedEncoding);
        };
        Self {
            readers: Readers {
                inspector: Inspector::new(source, limits.report),
                activity: Some(ResponseReader::new(protocol, limits.activity)),
            },
            state: State::Active(Box::new(Active {
                protocol,
                limits,
                decoder,
                body: match kind {
                    BodyKind::Stream => Body::Stream(SseFramer::new(limits.sse)),
                    BodyKind::Document => Body::Document(Vec::new()),
                    BodyKind::Messages => Body::Messages(MessageBuffer::default()),
                },
                decoded_bytes: 0,
            })),
        }
    }

    fn unavailable(format: FormatSource, reason: Limitation) -> Self {
        Self {
            readers: Readers {
                inspector: Inspector::new(format, ReportLimits::default()),
                activity: None,
            },
            state: State::Unavailable { format, reason },
        }
    }

    /// A WebSocket message of `len` bytes starts with the next fed byte. Any
    /// unfinished previous message was cut short and is not interpreted.
    pub fn message(&mut self, len: u64) {
        let State::Active(active) = &mut self.state else {
            return;
        };
        let Body::Messages(buffer) = &mut active.body else {
            return;
        };
        if buffer.current != Message::None {
            self.readers.limit(Limitation::IncompleteEvent);
        }
        buffer.bytes.clear();
        match usize::try_from(len) {
            Ok(len) if len <= active.limits.sse.max_event_bytes => {
                buffer.remaining = len;
                buffer.current = Message::Buffering;
                if len == 0 {
                    buffer.current = Message::None;
                    self.readers.event(active.protocol, EventName::Absent, &[]);
                }
            }
            Ok(_) | Err(_) => {
                buffer.remaining = usize::try_from(len).unwrap_or(usize::MAX);
                buffer.current = Message::Skipping;
                self.readers.limit(Limitation::EventTooLarge);
            }
        }
    }

    /// The next contiguous captured bytes.
    pub fn feed(&mut self, bytes: &[u8]) {
        let State::Active(active) = &mut self.state else {
            return;
        };
        let readers = &mut self.readers;
        let Active {
            protocol,
            limits,
            decoder,
            body,
            decoded_bytes,
        } = &mut **active;
        let mut stop = None;
        let decoded = decoder.decode(bytes, |decoded| {
            if stop.is_none() {
                stop = body
                    .absorb(*protocol, limits, decoded_bytes, decoded, readers)
                    .err();
            }
        });
        let stop = stop.or(match decoded {
            Ok(()) => None,
            Err(DecodeFailure::Corrupt | DecodeFailure::Truncated | DecodeFailure::Unsupported) => {
                Some(Limitation::DecodingError)
            }
        });
        if let Some(reason) = stop {
            self.readers.limit(reason);
            self.state = State::Stopped;
        }
    }

    /// Captured bytes are missing at this point. Nothing after a gap is
    /// interpreted, because fragments cannot be spliced across it.
    pub fn gap(&mut self) {
        match self.state {
            State::Unavailable { .. } => {}
            State::Active(_) | State::Stopped => {
                self.readers.limit(Limitation::CaptureGap);
                self.state = State::Stopped;
            }
        }
    }

    /// The response's activity so far, while it streams: the reply and the
    /// tool calls read to now. `None` when the body cannot be interpreted.
    /// A whole JSON document is read only at its end.
    /// Whether the response so far ends inside the model's thinking
    /// ([`ResponseReader::thinking`]).
    pub fn thinking(&self) -> bool {
        self.readers
            .activity
            .as_ref()
            .is_some_and(ResponseReader::thinking)
    }

    pub fn so_far(&self) -> Option<ResponseActivity> {
        self.readers.activity.as_ref().map(ResponseReader::so_far)
    }

    pub fn finish(self, end: BodyEnd) -> ResponseReading {
        let Self { mut readers, state } = self;
        match state {
            State::Unavailable { format, reason } => {
                return ResponseReading {
                    inspection: Inspector::unavailable(format, reason),
                    activity: ResponseActivity::default(),
                };
            }
            State::Stopped => {}
            State::Active(active) => {
                let Active {
                    protocol,
                    limits,
                    decoder,
                    mut body,
                    mut decoded_bytes,
                } = *active;
                let mut stop = None;
                let decoded = decoder.finish(|decoded| {
                    if stop.is_none() {
                        stop = body
                            .absorb(protocol, &limits, &mut decoded_bytes, decoded, &mut readers)
                            .err();
                    }
                });
                let decoded = match (decoded, end) {
                    (Ok(()), BodyEnd::Complete | BodyEnd::Unfinished) => Ok(()),
                    // A cut-off body is already reported as unfinished.
                    (Err(DecodeFailure::Truncated), BodyEnd::Unfinished) => Err(None),
                    (Err(DecodeFailure::Truncated), BodyEnd::Complete)
                    | (
                        Err(DecodeFailure::Corrupt | DecodeFailure::Unsupported),
                        BodyEnd::Complete | BodyEnd::Unfinished,
                    ) => Err(Some(Limitation::DecodingError)),
                };
                match (stop, decoded) {
                    (Some(reason), _) | (None, Err(Some(reason))) => readers.limit(reason),
                    (None, Err(None)) => {}
                    (None, Ok(())) => body.finish(protocol, end, &mut readers),
                }
            }
        }
        if end == BodyEnd::Unfinished {
            readers.limit(Limitation::BodyUnfinished);
        }
        readers.finish()
    }
}

impl Body {
    /// Take decoded bytes, enforcing the decoded and document limits.
    fn absorb(
        &mut self,
        protocol: Protocol,
        limits: &InspectionLimits,
        decoded_bytes: &mut u64,
        bytes: &[u8],
        readers: &mut Readers,
    ) -> Result<(), Limitation> {
        *decoded_bytes =
            decoded_bytes.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        if *decoded_bytes > limits.max_decoded_bytes {
            return Err(Limitation::DecodedTooLarge);
        }
        match self {
            Self::Stream(framer) => {
                framer.feed(bytes, |output| match output {
                    SseOutput::Event { name, data } => readers.event(protocol, name, data),
                    SseOutput::Discarded(_) => readers.limit(Limitation::EventTooLarge),
                });
                Ok(())
            }
            Self::Document(buffer) => {
                if buffer.len().saturating_add(bytes.len()) > limits.max_document_bytes {
                    buffer.clear();
                    Err(Limitation::DocumentTooLarge)
                } else {
                    buffer.extend_from_slice(bytes);
                    Ok(())
                }
            }
            Self::Messages(buffer) => buffer.absorb(protocol, bytes, readers),
        }
    }

    fn finish(self, protocol: Protocol, end: BodyEnd, readers: &mut Readers) {
        match (self, end) {
            (Self::Stream(framer), BodyEnd::Complete | BodyEnd::Unfinished) => {
                if framer.end() == StreamEnd::PartialEvent {
                    readers.limit(Limitation::IncompleteEvent);
                }
            }
            (Self::Document(buffer), BodyEnd::Complete) => {
                readers.document(protocol, &buffer);
            }
            // A truncated document is not parsed: a prefix could still be
            // valid JSON and would invent facts.
            (Self::Document(_), BodyEnd::Unfinished) => {}
            (Self::Messages(buffer), BodyEnd::Complete | BodyEnd::Unfinished) => {
                if buffer.current != Message::None {
                    readers.limit(Limitation::IncompleteEvent);
                }
            }
        }
    }
}

impl MessageBuffer {
    fn absorb(
        &mut self,
        protocol: Protocol,
        mut bytes: &[u8],
        readers: &mut Readers,
    ) -> Result<(), Limitation> {
        while !bytes.is_empty() {
            let (part, rest) = bytes.split_at(self.remaining.min(bytes.len()));
            bytes = rest;
            self.remaining = self.remaining.saturating_sub(part.len());
            match self.current {
                // Bytes outside an announced message: the framing is lost.
                Message::None => return Err(Limitation::IncompleteEvent),
                Message::Skipping => {}
                Message::Buffering => self.bytes.extend_from_slice(part),
            }
            if self.remaining == 0 {
                if self.current == Message::Buffering {
                    readers.event(protocol, EventName::Absent, &self.bytes);
                }
                self.bytes.clear();
                self.current = Message::None;
            }
        }
        Ok(())
    }
}
