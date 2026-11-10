//! Codex Responses over WebSocket: classifying whole messages for the channel
//! lifecycle.
//!
//! Each text message is one JSON object. Client messages are `response.create`
//! and `response.interrupt`; upstream messages are Responses events (see
//! [`super::responses`]) plus the interrupt answers. Classification reads only
//! the `type` and the fields the lifecycle needs; the message itself is
//! forwarded and recorded unchanged by the caller. Malformed and binary
//! messages are `Other`, never an error: they are still forwarded.

use super::{
    json::{Field, JsonDocument, JsonObject, Kind},
    responses::EventKind,
    text::WireText,
};
use crate::native::{MessageHints, NativeSessionId, NativeThreadId, NativeTurnId};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use uuid::Uuid;

/// How a message's payload is framed.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Payload {
    Text,
    Binary,
}

/// A WebSocket close status code (RFC 6455 section 7.4). Codes a peer sent are
/// forwarded as received; the socket layer rejects ones that may not be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseCode(u16);

impl CloseCode {
    pub const NORMAL: Self = Self(1000);
    pub const GOING_AWAY: Self = Self(1001);
    /// "Service restart": the client may reconnect.
    pub const SERVICE_RESTART: Self = Self(1012);
    /// "Bad gateway": the upstream failed.
    pub const BAD_GATEWAY: Self = Self(1014);

    pub fn new(code: u16) -> Self {
        Self(code)
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

/// A client message, as far as the lifecycle needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage<R> {
    /// `response.create`: opens a call.
    Create,
    /// `response.interrupt` for the named response, if it named one.
    Interrupt(Option<R>),
    /// Anything else, including binary and malformed messages.
    Other,
}

/// An upstream message, as far as the lifecycle needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpstreamMessage<R> {
    /// `response.created`, naming the response if it did.
    Created(Option<R>),
    /// A terminal event: `response.completed`, `.failed`, `.incomplete`, or `error`.
    Terminal,
    /// `response.interrupt.accepted` or `.failed` for the named response.
    InterruptAnswered {
        response: Option<R>,
        outcome: InterruptOutcome,
    },
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptOutcome {
    Accepted,
    Rejected,
}

/// Message types a client sends on a Responses WebSocket.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ClientKind {
    #[strum(to_string = "response.create")]
    Create,
    #[strum(to_string = "response.interrupt")]
    Interrupt,
}

/// A Responses response identifier (`resp_…`), as the provider sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseId(WireText);

impl ResponseId {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    fn read(field: Field<WireText>) -> Option<Self> {
        field.present().map(Self)
    }
}

/// A classified client message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    /// `response.create`, with the turn hints from its `client_metadata`.
    Create(MessageHints),
    Interrupt(Option<ResponseId>),
    Other,
}

impl ClientEvent {
    pub fn classify(payload: Payload, bytes: &[u8]) -> Self {
        let Some(document) = Self::document(payload, bytes) else {
            return Self::Other;
        };
        let Ok(object) = document.object() else {
            return Self::Other;
        };
        match object.label::<ClientKind>("type") {
            Field::Present(Kind::Known(ClientKind::Create)) => {
                Self::Create(Self::hints(object.object("client_metadata")))
            }
            Field::Present(Kind::Known(ClientKind::Interrupt)) => {
                Self::Interrupt(ResponseId::read(object.identifier("response_id")))
            }
            Field::Present(Kind::Unknown(_)) | Field::Absent | Field::Malformed => Self::Other,
        }
    }

    fn document(payload: Payload, bytes: &[u8]) -> Option<JsonDocument> {
        match payload {
            Payload::Text => JsonDocument::parse(bytes).ok(),
            Payload::Binary => None,
        }
    }

    /// Codex sends `session_id`, `thread_id`, and `turn_id` as UUID strings.
    /// Present values that are not UUIDs make the hints malformed.
    fn hints(metadata: Field<JsonObject<'_>>) -> MessageHints {
        let metadata = match metadata {
            Field::Absent => return MessageHints::default(),
            Field::Malformed => return MessageHints::MALFORMED,
            Field::Present(metadata) => metadata,
        };
        let mut malformed = false;
        let mut uuid = |key: &str| match metadata.identifier(key) {
            Field::Absent => None,
            Field::Present(text) => {
                let parsed = Uuid::try_parse(text.as_str())
                    .ok()
                    .filter(|id| !id.is_nil());
                malformed |= parsed.is_none();
                parsed
            }
            Field::Malformed => {
                malformed = true;
                None
            }
        };
        let session = uuid("session_id").and_then(NativeSessionId::new);
        let thread = uuid("thread_id").and_then(NativeThreadId::new);
        let turn = uuid("turn_id").and_then(NativeTurnId::new);
        if malformed {
            MessageHints::MALFORMED
        } else {
            MessageHints::valid(session, thread, turn)
        }
    }

    /// What the channel lifecycle needs from this message.
    pub fn lifecycle(&self) -> ClientMessage<ResponseId> {
        match self {
            Self::Create(_) => ClientMessage::Create,
            Self::Interrupt(response) => ClientMessage::Interrupt(response.clone()),
            Self::Other => ClientMessage::Other,
        }
    }
}

impl UpstreamMessage<ResponseId> {
    /// Classify one upstream message for the channel lifecycle.
    pub fn classify(payload: Payload, bytes: &[u8]) -> Self {
        let Some(document) = ClientEvent::document(payload, bytes) else {
            return UpstreamMessage::Other;
        };
        let Ok(object) = document.object() else {
            return UpstreamMessage::Other;
        };
        let answered = |outcome| Self::InterruptAnswered {
            response: ResponseId::read(object.identifier("response_id")),
            outcome,
        };
        match object.label::<EventKind>("type") {
            Field::Present(Kind::Known(EventKind::InterruptAccepted)) => {
                answered(InterruptOutcome::Accepted)
            }
            Field::Present(Kind::Known(EventKind::InterruptFailed)) => {
                answered(InterruptOutcome::Rejected)
            }
            Field::Present(Kind::Known(EventKind::Created)) => Self::Created(ResponseId::read(
                object
                    .object("response")
                    .present()
                    .map_or(Field::Absent, |response| response.identifier("id")),
            )),
            Field::Present(Kind::Known(
                EventKind::Completed | EventKind::Incomplete | EventKind::Failed | EventKind::Error,
            )) => Self::Terminal,
            Field::Present(Kind::Known(
                EventKind::Queued
                | EventKind::InProgress
                | EventKind::OutputItemAdded
                | EventKind::OutputItemDone
                | EventKind::ContentPartAdded
                | EventKind::ContentPartDone
                | EventKind::OutputTextDelta
                | EventKind::OutputTextDone
                | EventKind::OutputTextAnnotationAdded
                | EventKind::RefusalDelta
                | EventKind::RefusalDone
                | EventKind::FunctionCallArgumentsDelta
                | EventKind::FunctionCallArgumentsDone
                | EventKind::CustomToolCallInputDelta
                | EventKind::CustomToolCallInputDone
                | EventKind::ReasoningTextDelta
                | EventKind::ReasoningTextDone
                | EventKind::ReasoningSummaryPartAdded
                | EventKind::ReasoningSummaryPartDone
                | EventKind::ReasoningSummaryTextDelta
                | EventKind::ReasoningSummaryTextDone
                | EventKind::CodexResponseMetadata
                | EventKind::CodexRateLimits,
            ))
            | Field::Present(Kind::Unknown(_))
            | Field::Absent
            | Field::Malformed => Self::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ClientEvent, ResponseId};
    use crate::{
        native::MessageHints,
        text::WireText,
        websocket::{ClientMessage, InterruptOutcome, Payload, UpstreamMessage},
    };

    fn id(value: &str) -> Option<ResponseId> {
        Some(ResponseId(WireText::identifier(value).unwrap()))
    }

    const SESSION: &str = "11111111-1111-4111-8111-111111111111";
    const TURN: &str = "33333333-3333-4333-8333-333333333333";

    #[test]
    fn client_messages_are_classified_by_type() {
        let create = format!(
            r#"{{"type":"response.create","model":"m","client_metadata":{{"session_id":"{SESSION}","turn_id":"{TURN}","extra":"kept"}}}}"#
        );
        let ClientEvent::Create(hints) = ClientEvent::classify(Payload::Text, create.as_bytes())
        else {
            panic!("not a create");
        };
        assert_eq!(hints.session().unwrap().to_string(), SESSION);
        assert_eq!(hints.turn().unwrap().to_string(), TURN);
        assert!(!hints.is_malformed());

        for (payload, bytes, expected) in [
            (
                Payload::Text,
                &br#"{"type":"response.create"}"#[..],
                ClientEvent::Create(MessageHints::default()),
            ),
            (
                Payload::Text,
                br#"{"type":"response.create","client_metadata":{"turn_id":"not-a-uuid"}}"#,
                ClientEvent::Create(MessageHints::MALFORMED),
            ),
            (
                Payload::Text,
                br#"{"type":"response.create","client_metadata":[]}"#,
                ClientEvent::Create(MessageHints::MALFORMED),
            ),
            (
                Payload::Text,
                br#"{"type":"response.interrupt","response_id":"resp_1","mode":"discard_partial_items"}"#,
                ClientEvent::Interrupt(id("resp_1")),
            ),
            (
                Payload::Text,
                br#"{"type":"response.interrupt"}"#,
                ClientEvent::Interrupt(None),
            ),
            (Payload::Text, br#"{"type":"session.update"}"#, ClientEvent::Other),
            (Payload::Text, b"not json", ClientEvent::Other),
            (Payload::Text, b"[]", ClientEvent::Other),
            (
                Payload::Binary,
                br#"{"type":"response.create"}"#,
                ClientEvent::Other,
            ),
        ] {
            assert_eq!(ClientEvent::classify(payload, bytes), expected);
        }
        assert_eq!(
            ClientEvent::Interrupt(id("resp_1")).lifecycle(),
            ClientMessage::Interrupt(id("resp_1"))
        );
    }

    #[test]
    fn upstream_messages_are_classified_for_the_lifecycle() {
        for (bytes, expected) in [
            (
                &br#"{"type":"response.created","response":{"id":"resp_1","status":"in_progress"}}"#[..],
                UpstreamMessage::Created(id("resp_1")),
            ),
            (
                br#"{"type":"response.created"}"#,
                UpstreamMessage::Created(None),
            ),
            (
                br#"{"type":"response.output_text.delta","delta":"hi"}"#,
                UpstreamMessage::Other,
            ),
            (br#"{"type":"codex.rate_limits"}"#, UpstreamMessage::Other),
            (br#"{"type":"response.completed","response":{}}"#, UpstreamMessage::Terminal),
            (br#"{"type":"response.incomplete"}"#, UpstreamMessage::Terminal),
            (br#"{"type":"response.failed"}"#, UpstreamMessage::Terminal),
            (br#"{"type":"error","error":{}}"#, UpstreamMessage::Terminal),
            (
                br#"{"type":"response.interrupt.accepted","response_id":"resp_1"}"#,
                UpstreamMessage::InterruptAnswered {
                    response: id("resp_1"),
                    outcome: InterruptOutcome::Accepted,
                },
            ),
            (
                br#"{"type":"response.interrupt.failed","response_id":"resp_1"}"#,
                UpstreamMessage::InterruptAnswered {
                    response: id("resp_1"),
                    outcome: InterruptOutcome::Rejected,
                },
            ),
            (br#"{"type":"something.new"}"#, UpstreamMessage::Other),
            (b"\xff", UpstreamMessage::Other),
        ] {
            assert_eq!(UpstreamMessage::classify(Payload::Text, bytes), expected);
        }
        assert_eq!(
            UpstreamMessage::classify(Payload::Binary, br#"{"type":"response.completed"}"#),
            UpstreamMessage::Other
        );
    }
}
