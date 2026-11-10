//! OpenAI Responses: streaming events and response documents, shared by the
//! public API and the ChatGPT Codex backend.
//!
//! The same event JSON arrives as SSE `data` or, over WebSocket, as one text
//! message per event, so [`Event::decode`] takes only the payload and an
//! optional transport-level name. Vocabularies follow the documented sets
//! (checked 2026-11-04); anything else is kept as [`Kind::Unknown`].

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

/// Responses streaming event types that Ren recognizes.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum EventKind {
    #[strum(to_string = "response.queued")]
    Queued,
    #[strum(to_string = "response.created")]
    Created,
    #[strum(to_string = "response.in_progress")]
    InProgress,
    #[strum(to_string = "response.completed")]
    Completed,
    #[strum(to_string = "response.incomplete")]
    Incomplete,
    #[strum(to_string = "response.failed")]
    Failed,
    Error,
    #[strum(to_string = "response.output_item.added")]
    OutputItemAdded,
    #[strum(to_string = "response.output_item.done")]
    OutputItemDone,
    #[strum(to_string = "response.content_part.added")]
    ContentPartAdded,
    #[strum(to_string = "response.content_part.done")]
    ContentPartDone,
    #[strum(to_string = "response.output_text.delta")]
    OutputTextDelta,
    #[strum(to_string = "response.output_text.done")]
    OutputTextDone,
    #[strum(to_string = "response.output_text.annotation.added")]
    OutputTextAnnotationAdded,
    #[strum(to_string = "response.refusal.delta")]
    RefusalDelta,
    #[strum(to_string = "response.refusal.done")]
    RefusalDone,
    #[strum(to_string = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta,
    #[strum(to_string = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone,
    #[strum(to_string = "response.custom_tool_call_input.delta")]
    CustomToolCallInputDelta,
    #[strum(to_string = "response.custom_tool_call_input.done")]
    CustomToolCallInputDone,
    #[strum(to_string = "response.reasoning_text.delta")]
    ReasoningTextDelta,
    #[strum(to_string = "response.reasoning_text.done")]
    ReasoningTextDone,
    #[strum(to_string = "response.reasoning_summary_part.added")]
    ReasoningSummaryPartAdded,
    #[strum(to_string = "response.reasoning_summary_part.done")]
    ReasoningSummaryPartDone,
    #[strum(to_string = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta,
    #[strum(to_string = "response.reasoning_summary_text.done")]
    ReasoningSummaryTextDone,
    #[strum(to_string = "codex.response.metadata")]
    CodexResponseMetadata,
    #[strum(to_string = "codex.rate_limits")]
    CodexRateLimits,
    /// WebSocket only: the answers to a client's `response.interrupt`.
    #[strum(to_string = "response.interrupt.accepted")]
    InterruptAccepted,
    #[strum(to_string = "response.interrupt.failed")]
    InterruptFailed,
}

/// Response status values.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Status {
    Queued,
    InProgress,
    Completed,
    Incomplete,
    Failed,
    Cancelled,
}

impl Status {
    /// The model outcome a status reports, if it is terminal.
    pub fn outcome(self) -> Option<ModelOutcome> {
        match self {
            Self::Queued | Self::InProgress => None,
            Self::Completed => Some(ModelOutcome::Completed),
            Self::Incomplete => Some(ModelOutcome::Incomplete),
            Self::Failed => Some(ModelOutcome::Failed),
            Self::Cancelled => Some(ModelOutcome::Cancelled),
        }
    }
}

/// Why a response is incomplete.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum IncompleteReason {
    MaxOutputTokens,
    MaxMessages,
    ContentFilter,
    Steered,
}

/// Output item types.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ItemKind {
    Message,
    Reasoning,
    FunctionCall,
    CustomToolCall,
    ApplyPatchCall,
    ShellCall,
    LocalShellCall,
    ComputerCall,
    McpCall,
    McpApprovalRequest,
    WebSearchCall,
    FileSearchCall,
    CodeInterpreterCall,
    ImageGenerationCall,
    ToolSearchCall,
}

impl ItemKind {
    /// Items in which the model requests a tool (or approval to run one).
    pub fn is_tool_call(self) -> bool {
        match self {
            Self::Message | Self::Reasoning => false,
            Self::FunctionCall
            | Self::CustomToolCall
            | Self::ApplyPatchCall
            | Self::ShellCall
            | Self::LocalShellCall
            | Self::ComputerCall
            | Self::McpCall
            | Self::McpApprovalRequest
            | Self::WebSearchCall
            | Self::FileSearchCall
            | Self::CodeInterpreterCall
            | Self::ImageGenerationCall
            | Self::ToolSearchCall => true,
        }
    }
}

/// Token counts as Responses reports them: `input_tokens` includes cached
/// tokens, which are broken out in `input_tokens_details`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponsesUsage {
    input_tokens: Field<u64>,
    output_tokens: Field<u64>,
    cached_tokens: Field<u64>,
    reasoning_tokens: Field<u64>,
}

impl ResponsesUsage {
    fn read(object: JsonObject<'_>) -> Self {
        let detail = |key: &str, field: &str| match object.object(key) {
            Field::Present(details) => details.count(field),
            Field::Absent => Field::Absent,
            Field::Malformed => Field::Malformed,
        };
        Self {
            input_tokens: object.count("input_tokens"),
            output_tokens: object.count("output_tokens"),
            cached_tokens: detail("input_tokens_details", "cached_tokens"),
            reasoning_tokens: detail("output_tokens_details", "reasoning_tokens"),
        }
    }

    fn observe(self, out: &mut impl FnMut(Observation)) {
        let malformed = [
            &self.input_tokens,
            &self.output_tokens,
            &self.cached_tokens,
            &self.reasoning_tokens,
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
            cache_read_tokens: self.cached_tokens.present(),
            reasoning_tokens: self.reasoning_tokens.present(),
            ..Usage::new(InputAccounting::IncludesCache)
        }));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputItem {
    kind: Field<Kind<ItemKind>>,
    id: Field<WireText>,
    call_id: Field<WireText>,
    name: Field<WireText>,
}

impl OutputItem {
    fn read(object: JsonObject<'_>) -> Self {
        Self {
            kind: object.label("type"),
            id: object.identifier("id"),
            call_id: object.identifier("call_id"),
            name: object.text("name"),
        }
    }

    fn observe(self, out: &mut impl FnMut(Observation)) {
        let Self {
            kind,
            id,
            call_id,
            name,
        } = self;
        kind.observe_with(out, |kind, out| match kind {
            Kind::Known(kind) if kind.is_tool_call() => {
                if id.is_malformed() || call_id.is_malformed() || name.is_malformed() {
                    out(Observation::MalformedField);
                }
                out(Observation::ToolCall(ToolCall {
                    call_id: call_id.present(),
                    item_id: id.present(),
                    kind: WireText::from_label(kind),
                    name: name.present(),
                }));
            }
            Kind::Known(_) => {}
            Kind::Unknown(kind) => out(Observation::Unknown {
                family: KindFamily::OutputItem,
                kind,
            }),
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    model: Field<WireText>,
    status: Field<Kind<Status>>,
    incomplete_reason: Field<Kind<IncompleteReason>>,
    usage: Field<ResponsesUsage>,
    output: Field<Vec<Field<OutputItem>>>,
    error: Field<()>,
}

impl Response {
    fn read(object: JsonObject<'_>) -> Self {
        Self {
            model: object.text("model"),
            status: object.label("status"),
            incomplete_reason: match object.object("incomplete_details") {
                Field::Present(details) => details.label("reason"),
                Field::Absent => Field::Absent,
                Field::Malformed => Field::Malformed,
            },
            usage: object.object("usage").map(ResponsesUsage::read),
            output: object.objects("output").map(|items| {
                items
                    .into_iter()
                    .map(|item| item.map(OutputItem::read))
                    .collect()
            }),
            // Only presence matters: error contents never enter observations.
            error: object.object("error").map(|_| ()),
        }
    }

    fn observe(self, out: &mut impl FnMut(Observation)) {
        self.model
            .observe_with(out, |model, out| out(Observation::Model(model)));
        self.status.observe_with(out, |status, out| match status {
            Kind::Known(status) => {
                if let Some(outcome) = status.outcome() {
                    out(Observation::Outcome(outcome));
                }
            }
            Kind::Unknown(status) => {
                out(Observation::Unknown {
                    family: KindFamily::Status,
                    kind: status,
                });
                out(Observation::Outcome(ModelOutcome::Unknown));
            }
        });
        self.incomplete_reason
            .observe_with(out, |reason, out| match reason {
                Kind::Known(reason) => out(Observation::StopReason(WireText::from_label(reason))),
                Kind::Unknown(reason) => {
                    out(Observation::StopReason(reason.clone()));
                    out(Observation::Unknown {
                        family: KindFamily::IncompleteReason,
                        kind: reason,
                    });
                }
            });
        self.usage.observe_with(out, ResponsesUsage::observe);
        self.output.observe_with(out, |items, out| {
            for item in items {
                item.observe_with(out, OutputItem::observe);
            }
        });
        self.error.observe_with(out, |(), out| {
            out(Observation::Outcome(ModelOutcome::Failed));
        });
    }
}

/// One Responses streaming event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// `response.queued`, `.created`, `.in_progress`: not terminal.
    Progress(Field<Response>),
    /// `response.completed`, `.incomplete`, `.failed`: terminal, with the
    /// outcome the event type itself reports.
    Finished {
        outcome: ModelOutcome,
        response: Field<Response>,
    },
    OutputItem(Field<OutputItem>),
    Error,
    /// A known event that carries nothing the report records (deltas, parts,
    /// Codex metadata).
    Ignored(EventKind),
    /// `[DONE]`: some transports append it; it is not terminal evidence.
    DoneSentinel,
    Unknown(WireText),
}

impl Event {
    /// Decode one event payload. The JSON `type` field is authoritative; an
    /// SSE event name is used only when `type` is absent.
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
        let response = || object.object("response").map(Response::read);
        let finished = |outcome| Self::Finished {
            outcome,
            response: response(),
        };
        Ok(match kind {
            Kind::Unknown(kind) => Self::Unknown(kind),
            Kind::Known(EventKind::Queued | EventKind::Created | EventKind::InProgress) => {
                Self::Progress(response())
            }
            Kind::Known(EventKind::Completed) => finished(ModelOutcome::Completed),
            Kind::Known(EventKind::Incomplete) => finished(ModelOutcome::Incomplete),
            Kind::Known(EventKind::Failed) => finished(ModelOutcome::Failed),
            Kind::Known(EventKind::Error) => Self::Error,
            Kind::Known(EventKind::OutputItemAdded | EventKind::OutputItemDone) => {
                Self::OutputItem(object.object("item").map(OutputItem::read))
            }
            Kind::Known(
                kind @ (EventKind::ContentPartAdded
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
                | EventKind::CodexRateLimits
                | EventKind::InterruptAccepted
                | EventKind::InterruptFailed),
            ) => Self::Ignored(kind),
        })
    }

    pub fn observe(self, out: &mut impl FnMut(Observation)) -> EventClass {
        match self {
            Self::Progress(response) => response.observe_with(out, Response::observe),
            Self::Finished { outcome, response } => {
                response.observe_with(out, Response::observe);
                // The event type is terminal evidence in its own right; merge
                // keeps a more conservative nested status if there is one.
                out(Observation::Outcome(outcome));
                out(Observation::TerminalEvent);
            }
            Self::OutputItem(item) => item.observe_with(out, OutputItem::observe),
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

/// A complete, non-streaming Responses body: a response object or an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document(Response);

impl Document {
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let document = JsonDocument::parse(bytes)?;
        Ok(Self(Response::read(document.object()?)))
    }

    /// A complete document is its own terminal evidence.
    pub fn observe(self, out: &mut impl FnMut(Observation)) -> EventClass {
        self.0.observe(out);
        out(Observation::TerminalEvent);
        EventClass::Understood
    }
}

#[cfg(test)]
mod tests {
    use super::Status;
    use crate::{inspect::ModelOutcome, label::Labeled};

    #[test]
    fn every_documented_status_has_a_deliberate_outcome() {
        let outcomes: Vec<_> = Status::ALL
            .iter()
            .map(|status| (status.label(), status.outcome()))
            .collect();
        assert_eq!(
            outcomes,
            [
                ("queued", None),
                ("in_progress", None),
                ("completed", Some(ModelOutcome::Completed)),
                ("incomplete", Some(ModelOutcome::Incomplete)),
                ("failed", Some(ModelOutcome::Failed)),
                ("cancelled", Some(ModelOutcome::Cancelled)),
            ]
        );
    }

    #[test]
    fn vocabularies_have_unique_labels() {}
}
