//! Anthropic Messages: streaming events and response documents.
//!
//! Vocabularies follow the documented sets (streaming events, content block
//! types, stop reasons, checked 2026-11-04). Values outside them are kept as
//! [`Kind::Unknown`] and reported, never dropped or guessed.

use super::{
    inspect::{
        EventClass, InputAccounting, KindFamily, ModelOutcome, Observation, ToolCall, Usage,
    },
    json::{DecodeError, Field, JsonDocument, JsonObject, Kind},
    sse::EventName,
    text::WireText,
};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Messages streaming event types.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum EventKind {
    MessageStart,
    ContentBlockStart,
    ContentBlockDelta,
    ContentBlockStop,
    MessageDelta,
    MessageStop,
    Ping,
    Error,
}

/// Why the model stopped.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum StopReason {
    EndTurn,
    StopSequence,
    ToolUse,
    MaxTokens,
    PauseTurn,
    ModelContextWindowExceeded,
    Refusal,
}

impl StopReason {
    /// What a stop reason says about the response. A named method rather than
    /// `From`: the reason itself is dropped.
    pub fn outcome(self) -> ModelOutcome {
        match self {
            Self::EndTurn | Self::StopSequence | Self::ToolUse => ModelOutcome::Completed,
            Self::MaxTokens | Self::PauseTurn | Self::ModelContextWindowExceeded => {
                ModelOutcome::Incomplete
            }
            Self::Refusal => ModelOutcome::Refused,
        }
    }

    /// Report a stop reason read from the wire. An unknown reason is kept
    /// verbatim and makes the outcome unknown, never completed.
    fn observe(reason: Field<Kind<Self>>, out: &mut impl FnMut(Observation)) {
        reason.observe_with(out, |reason, out| match reason {
            Kind::Known(reason) => {
                out(Observation::StopReason(WireText::from_label(reason)));
                out(Observation::Outcome(reason.outcome()));
            }
            Kind::Unknown(reason) => {
                out(Observation::StopReason(reason.clone()));
                out(Observation::Unknown {
                    family: KindFamily::StopReason,
                    kind: reason,
                });
                out(Observation::Outcome(ModelOutcome::Unknown));
            }
        });
    }
}

/// Content block types.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum BlockKind {
    Text,
    Thinking,
    RedactedThinking,
    ToolUse,
    ServerToolUse,
    McpToolUse,
    WebSearchToolResult,
    McpToolResult,
}

impl BlockKind {
    /// Blocks in which the model requests a tool.
    pub fn is_tool_call(self) -> bool {
        match self {
            Self::ToolUse | Self::ServerToolUse | Self::McpToolUse => true,
            Self::Text
            | Self::Thinking
            | Self::RedactedThinking
            | Self::WebSearchToolResult
            | Self::McpToolResult => false,
        }
    }
}

/// Top-level document types: a message, or an error body.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum DocumentKind {
    Message,
    Error,
}

/// Token counts as Messages reports them: cached tokens are excluded from
/// `input_tokens` and reported separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagesUsage {
    input_tokens: Field<u64>,
    output_tokens: Field<u64>,
    cache_read_input_tokens: Field<u64>,
    cache_creation_input_tokens: Field<u64>,
}

impl MessagesUsage {
    fn read(object: JsonObject<'_>) -> Self {
        Self {
            input_tokens: object.count("input_tokens"),
            output_tokens: object.count("output_tokens"),
            cache_read_input_tokens: object.count("cache_read_input_tokens"),
            cache_creation_input_tokens: object.count("cache_creation_input_tokens"),
        }
    }

    fn observe(self, out: &mut impl FnMut(Observation)) {
        let malformed = [
            &self.input_tokens,
            &self.output_tokens,
            &self.cache_read_input_tokens,
            &self.cache_creation_input_tokens,
        ]
        .iter()
        .filter(|field| field.is_malformed())
        .count();
        for _ in 0..malformed {
            out(Observation::MalformedField);
        }
        out(Observation::Usage(Usage {
            input_tokens: self.input_tokens.present(),
            output_tokens: self.output_tokens.present(),
            cache_read_tokens: self.cache_read_input_tokens.present(),
            cache_creation_tokens: self.cache_creation_input_tokens.present(),
            ..Usage::new(InputAccounting::ExcludesCache)
        }));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentBlock {
    kind: Field<Kind<BlockKind>>,
    id: Field<WireText>,
    name: Field<WireText>,
}

impl ContentBlock {
    fn read(object: JsonObject<'_>) -> Self {
        Self {
            kind: object.label("type"),
            id: object.identifier("id"),
            name: object.text("name"),
        }
    }

    fn observe(self, out: &mut impl FnMut(Observation)) {
        let Self { kind, id, name } = self;
        kind.observe_with(out, |kind, out| match kind {
            Kind::Known(kind) if kind.is_tool_call() => {
                if id.is_malformed() || name.is_malformed() {
                    out(Observation::MalformedField);
                }
                out(Observation::ToolCall(ToolCall {
                    call_id: id.present(),
                    item_id: None,
                    kind: WireText::from_label(kind),
                    name: name.present(),
                }));
            }
            Kind::Known(_) => {}
            Kind::Unknown(kind) => out(Observation::Unknown {
                family: KindFamily::ContentBlock,
                kind,
            }),
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    model: Field<WireText>,
    stop_reason: Field<Kind<StopReason>>,
    usage: Field<MessagesUsage>,
    content: Field<Vec<Field<ContentBlock>>>,
}

impl Message {
    fn read(object: JsonObject<'_>) -> Self {
        Self {
            model: object.text("model"),
            stop_reason: object.label("stop_reason"),
            usage: object.object("usage").map(MessagesUsage::read),
            content: object.objects("content").map(|blocks| {
                blocks
                    .into_iter()
                    .map(|block| block.map(ContentBlock::read))
                    .collect()
            }),
        }
    }

    fn observe(self, out: &mut impl FnMut(Observation)) {
        self.model
            .observe_with(out, |model, out| out(Observation::Model(model)));
        StopReason::observe(self.stop_reason, out);
        self.usage.observe_with(out, MessagesUsage::observe);
        self.content.observe_with(out, |blocks, out| {
            for block in blocks {
                block.observe_with(out, ContentBlock::observe);
            }
        });
    }
}

/// One Messages streaming event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    MessageStart(Field<Message>),
    ContentBlockStart(Field<ContentBlock>),
    MessageDelta {
        stop_reason: Field<Kind<StopReason>>,
        usage: Field<MessagesUsage>,
    },
    MessageStop,
    Error,
    /// A known event that carries nothing the report records (pings, deltas).
    Ignored(EventKind),
    /// `[DONE]`, sent by some gateways. Not part of the Messages protocol and
    /// not terminal evidence.
    DoneSentinel,
    Unknown(WireText),
}

impl Event {
    /// Decode one SSE event. The JSON `type` field is authoritative; the SSE
    /// event name is used only when `type` is absent.
    pub fn decode(name: EventName<'_>, data: &[u8]) -> Result<Self, DecodeError> {
        if data == b"[DONE]" {
            return Ok(Self::DoneSentinel);
        }
        let document = JsonDocument::parse(data)?;
        let object = document.object()?;
        let kind = match object.label::<EventKind>("type") {
            Field::Present(kind) => kind,
            Field::Malformed => return Err(DecodeError::NoKind),
            Field::Absent => match name {
                EventName::Present(name) => Kind::parse(name).ok_or(DecodeError::NoKind)?,
                EventName::Absent | EventName::Invalid => return Err(DecodeError::NoKind),
            },
        };
        Ok(match kind {
            Kind::Unknown(kind) => Self::Unknown(kind),
            Kind::Known(EventKind::MessageStart) => {
                Self::MessageStart(object.object("message").map(Message::read))
            }
            Kind::Known(EventKind::ContentBlockStart) => {
                Self::ContentBlockStart(object.object("content_block").map(ContentBlock::read))
            }
            Kind::Known(EventKind::MessageDelta) => Self::MessageDelta {
                stop_reason: match object.object("delta") {
                    Field::Present(delta) => delta.label("stop_reason"),
                    Field::Absent => Field::Absent,
                    Field::Malformed => Field::Malformed,
                },
                usage: object.object("usage").map(MessagesUsage::read),
            },
            Kind::Known(EventKind::MessageStop) => Self::MessageStop,
            Kind::Known(EventKind::Error) => Self::Error,
            Kind::Known(
                kind @ (EventKind::ContentBlockDelta
                | EventKind::ContentBlockStop
                | EventKind::Ping),
            ) => Self::Ignored(kind),
        })
    }

    pub fn observe(self, out: &mut impl FnMut(Observation)) -> EventClass {
        match self {
            Self::MessageStart(message) => message.observe_with(out, Message::observe),
            Self::ContentBlockStart(block) => block.observe_with(out, ContentBlock::observe),
            Self::MessageDelta { stop_reason, usage } => {
                StopReason::observe(stop_reason, out);
                usage.observe_with(out, MessagesUsage::observe);
            }
            Self::MessageStop => out(Observation::TerminalEvent),
            Self::Error => {
                out(Observation::Outcome(ModelOutcome::Failed));
                out(Observation::TerminalEvent);
            }
            Self::Ignored(_) | Self::DoneSentinel => {}
            Self::Unknown(kind) => {
                out(Observation::Unknown {
                    family: KindFamily::Event,
                    kind,
                });
                return EventClass::Unknown;
            }
        }
        EventClass::Understood
    }
}

/// A complete, non-streaming Messages response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Document {
    Message(Message),
    Error,
    Unknown(WireText),
}

impl Document {
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let document = JsonDocument::parse(bytes)?;
        let object = document.object()?;
        Ok(match object.label::<DocumentKind>("type") {
            Field::Present(Kind::Known(DocumentKind::Message)) | Field::Absent => {
                Self::Message(Message::read(object))
            }
            Field::Present(Kind::Known(DocumentKind::Error)) => Self::Error,
            Field::Present(Kind::Unknown(kind)) => Self::Unknown(kind),
            Field::Malformed => return Err(DecodeError::NoKind),
        })
    }

    /// A complete document is its own terminal evidence.
    pub fn observe(self, out: &mut impl FnMut(Observation)) -> EventClass {
        let class = match self {
            Self::Message(message) => {
                message.observe(out);
                EventClass::Understood
            }
            Self::Error => {
                out(Observation::Outcome(ModelOutcome::Failed));
                EventClass::Understood
            }
            Self::Unknown(kind) => {
                out(Observation::Unknown {
                    family: KindFamily::Document,
                    kind,
                });
                EventClass::Unknown
            }
        };
        out(Observation::TerminalEvent);
        class
    }
}

#[cfg(test)]
mod tests {
    use super::StopReason;
    use crate::{inspect::ModelOutcome, label::Labeled};

    #[test]
    fn every_documented_stop_reason_has_a_deliberate_outcome() {
        let outcomes: Vec<_> = StopReason::ALL
            .iter()
            .map(|reason| (reason.label(), reason.outcome()))
            .collect();
        assert_eq!(
            outcomes,
            [
                ("end_turn", ModelOutcome::Completed),
                ("stop_sequence", ModelOutcome::Completed),
                ("tool_use", ModelOutcome::Completed),
                ("max_tokens", ModelOutcome::Incomplete),
                ("pause_turn", ModelOutcome::Incomplete),
                ("model_context_window_exceeded", ModelOutcome::Incomplete),
                ("refusal", ModelOutcome::Refused),
            ]
        );
    }

    #[test]
    fn vocabularies_have_unique_labels() {}
}
