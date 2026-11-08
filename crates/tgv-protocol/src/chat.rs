//! OpenAI Chat Completions: streamed chunks and completion documents, as
//! OpenAI, Anthropic's compatibility layer, DeepSeek, and local servers
//! speak them.
//!
//! Chunks carry no `type`; the `object` field names them. A stream ends with
//! `data: [DONE]`, which is this wire's end-of-response marker. Usage arrives
//! only when the request asked for it (`stream_options.include_usage`), in a
//! chunk whose `choices` is empty, so its absence says nothing. A tool call's
//! first delta carries its `id` and name; later deltas carry only its `index`
//! and argument fragments. Vocabularies follow the documented sets (checked
//! 2026-11-08); anything else is kept as [`Kind::Unknown`].

use super::{
    inspect::{
        EventClass, InputAccounting, KindFamily, ModelOutcome, Observation, ToolCall, Usage,
    },
    json::{DecodeError, Field, JsonDocument, JsonObject, Kind},
    text::WireText,
};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// The `object` field of a completion or a chunk.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ObjectKind {
    #[strum(to_string = "chat.completion")]
    Completion,
    #[strum(to_string = "chat.completion.chunk")]
    Chunk,
}

/// Why a choice stopped. `function_call` is OpenAI's deprecated form of
/// `tool_calls`; `insufficient_system_resource` is DeepSeek's.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    FunctionCall,
    InsufficientSystemResource,
}

impl FinishReason {
    /// What a finish reason says about the response. A named method rather
    /// than `From`: the reason itself is dropped.
    pub fn outcome(self) -> ModelOutcome {
        match self {
            Self::Stop | Self::ToolCalls | Self::FunctionCall => ModelOutcome::Completed,
            // Content was left out, as Responses reports it.
            Self::Length | Self::ContentFilter => ModelOutcome::Incomplete,
            Self::InsufficientSystemResource => ModelOutcome::Failed,
        }
    }

    /// Report a finish reason read from the wire. An unknown reason is kept
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

/// The `type` of a tool call.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ToolCallKind {
    Function,
    Custom,
}

impl ToolCallKind {
    /// The member holding the call's name and input.
    pub fn member(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Custom => "custom",
        }
    }

    /// The member's field holding the call's input: JSON arguments for a
    /// function, free-form text for a custom tool.
    pub fn input(self) -> &'static str {
        match self {
            Self::Function => "arguments",
            Self::Custom => "input",
        }
    }
}

/// Token counts as Chat Completions reports them: `prompt_tokens` includes
/// cached tokens. OpenAI breaks them out in `prompt_tokens_details`;
/// DeepSeek reports them as `prompt_cache_hit_tokens`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatUsage {
    prompt_tokens: Field<u64>,
    completion_tokens: Field<u64>,
    cached_tokens: Field<u64>,
    reasoning_tokens: Field<u64>,
}

impl ChatUsage {
    fn read(object: JsonObject<'_>) -> Self {
        let detail = |key: &str, field: &str| match object.object(key) {
            Field::Present(details) => details.count(field),
            Field::Absent => Field::Absent,
            Field::Malformed => Field::Malformed,
        };
        Self {
            prompt_tokens: object.count("prompt_tokens"),
            completion_tokens: object.count("completion_tokens"),
            cached_tokens: match detail("prompt_tokens_details", "cached_tokens") {
                Field::Absent => object.count("prompt_cache_hit_tokens"),
                present @ (Field::Present(_) | Field::Malformed) => present,
            },
            reasoning_tokens: detail("completion_tokens_details", "reasoning_tokens"),
        }
    }

    fn observe(self, out: &mut impl FnMut(Observation)) {
        let malformed = [
            &self.prompt_tokens,
            &self.completion_tokens,
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
            input_tokens: self.prompt_tokens.present(),
            output_tokens: self.completion_tokens.present(),
            cache_read_tokens: self.cached_tokens.present(),
            reasoning_tokens: self.reasoning_tokens.present(),
            ..Usage::new(InputAccounting::IncludesCache)
        }));
    }
}

/// A tool call in a message, or the first delta of one in a chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatToolCall {
    kind: Field<Kind<ToolCallKind>>,
    id: Field<WireText>,
    name: Field<WireText>,
}

impl ChatToolCall {
    fn read(object: JsonObject<'_>) -> Self {
        let kind = object.label::<ToolCallKind>("type");
        // A delta may omit `type` after the first; the name is read from
        // the member its type names, or `function` when it has none.
        let member = match &kind {
            Field::Present(Kind::Known(kind)) => kind.member(),
            Field::Present(Kind::Unknown(_)) | Field::Absent | Field::Malformed => {
                ToolCallKind::Function.member()
            }
        };
        Self {
            kind,
            id: object.identifier("id"),
            name: match object.object(member) {
                Field::Present(member) => member.text("name"),
                Field::Absent => Field::Absent,
                Field::Malformed => Field::Malformed,
            },
        }
    }

    fn observe(self, out: &mut impl FnMut(Observation)) {
        let Self { kind, id, name } = self;
        // An argument fragment names no call: nothing to report.
        if matches!(id, Field::Absent) && matches!(name, Field::Absent) {
            return kind.observe_with(out, Self::unknown_kind);
        }
        if id.is_malformed() || name.is_malformed() {
            out(Observation::MalformedField);
        }
        let label = match kind {
            Field::Present(Kind::Known(kind)) => WireText::from_label(kind),
            Field::Present(Kind::Unknown(kind)) => {
                Self::unknown_kind(Kind::Unknown(kind.clone()), out);
                kind
            }
            Field::Absent => WireText::from_label(ToolCallKind::Function),
            Field::Malformed => {
                out(Observation::MalformedField);
                WireText::from_label(ToolCallKind::Function)
            }
        };
        out(Observation::ToolCall(ToolCall {
            call_id: id.present(),
            item_id: None,
            kind: label,
            name: name.present(),
        }));
    }

    fn unknown_kind(kind: Kind<ToolCallKind>, out: &mut impl FnMut(Observation)) {
        match kind {
            Kind::Known(_) => {}
            // Chat Completions tool calls are the counterpart of Responses
            // output items.
            Kind::Unknown(kind) => out(Observation::Unknown {
                family: KindFamily::OutputItem,
                kind,
            }),
        }
    }
}

/// One choice: a document's `message` or a chunk's `delta`, and how it
/// finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    tool_calls: Field<Vec<Field<ChatToolCall>>>,
    finish_reason: Field<Kind<FinishReason>>,
}

impl Choice {
    fn read(object: JsonObject<'_>, body: &str) -> Self {
        Self {
            tool_calls: match object.object(body) {
                Field::Present(body) => body.objects("tool_calls").map(|calls| {
                    calls
                        .into_iter()
                        .map(|call| call.map(ChatToolCall::read))
                        .collect()
                }),
                Field::Absent => Field::Absent,
                Field::Malformed => Field::Malformed,
            },
            finish_reason: object.label("finish_reason"),
        }
    }

    fn observe(self, out: &mut impl FnMut(Observation)) {
        self.tool_calls.observe_with(out, |calls, out| {
            for call in calls {
                call.observe_with(out, ChatToolCall::observe);
            }
        });
        FinishReason::observe(self.finish_reason, out);
    }
}

/// A completion or a chunk: the fields both share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    model: Field<WireText>,
    choices: Field<Vec<Field<Choice>>>,
    usage: Field<ChatUsage>,
}

impl Completion {
    /// `body` is `message` in a document and `delta` in a chunk.
    fn read(object: JsonObject<'_>, body: &str) -> Self {
        Self {
            model: object.text("model"),
            choices: object.objects("choices").map(|choices| {
                choices
                    .into_iter()
                    .map(|choice| choice.map(|choice| Choice::read(choice, body)))
                    .collect()
            }),
            usage: object.object("usage").map(ChatUsage::read),
        }
    }

    fn observe(self, out: &mut impl FnMut(Observation)) {
        self.model
            .observe_with(out, |model, out| out(Observation::Model(model)));
        self.choices.observe_with(out, |choices, out| {
            for choice in choices {
                choice.observe_with(out, Choice::observe);
            }
        });
        self.usage.observe_with(out, ChatUsage::observe);
    }

    /// What `object` says a payload is. A payload without it is read as
    /// `assumed`: some servers leave it out.
    fn kind(object: JsonObject<'_>, assumed: ObjectKind) -> Result<Kind<ObjectKind>, DecodeError> {
        match object.label::<ObjectKind>("object") {
            Field::Present(kind) => Ok(kind),
            Field::Absent => Ok(Kind::Known(assumed)),
            Field::Malformed => Err(DecodeError::NoKind),
        }
    }
}

/// One streamed Chat Completions event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Chunk(Completion),
    /// `data: {"error": …}`: the stream failed. Its contents never enter
    /// observations.
    Error,
    /// `data: [DONE]`: the end of the response.
    Done,
    /// A whole completion where a chunk was expected; read the same way.
    Completion(Completion),
    Unknown(WireText),
}

impl Event {
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data == b"[DONE]" {
            return Ok(Self::Done);
        }
        let document = JsonDocument::parse(data)?;
        let object = document.object()?;
        if !matches!(object.object("error"), Field::Absent) {
            return Ok(Self::Error);
        }
        Ok(match Completion::kind(object, ObjectKind::Chunk)? {
            Kind::Known(ObjectKind::Chunk) => Self::Chunk(Completion::read(object, "delta")),
            Kind::Known(ObjectKind::Completion) => {
                Self::Completion(Completion::read(object, "message"))
            }
            Kind::Unknown(kind) => Self::Unknown(kind),
        })
    }

    pub fn observe(self, out: &mut impl FnMut(Observation)) -> EventClass {
        match self {
            Self::Chunk(chunk) | Self::Completion(chunk) => chunk.observe(out),
            Self::Error => {
                out(Observation::Outcome(ModelOutcome::Failed));
                out(Observation::TerminalEvent);
            }
            Self::Done => out(Observation::TerminalEvent),
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

/// A complete, non-streaming Chat Completions body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Document {
    Completion(Completion),
    Error,
    Unknown(WireText),
}

impl Document {
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let document = JsonDocument::parse(bytes)?;
        let object = document.object()?;
        if !matches!(object.object("error"), Field::Absent) {
            return Ok(Self::Error);
        }
        Ok(match Completion::kind(object, ObjectKind::Completion)? {
            Kind::Known(ObjectKind::Completion) => {
                Self::Completion(Completion::read(object, "message"))
            }
            // A chunk is not a document.
            Kind::Known(ObjectKind::Chunk) => {
                Self::Unknown(WireText::from_label(ObjectKind::Chunk))
            }
            Kind::Unknown(kind) => Self::Unknown(kind),
        })
    }

    /// A complete document is its own terminal evidence.
    pub fn observe(self, out: &mut impl FnMut(Observation)) -> EventClass {
        let class = match self {
            Self::Completion(completion) => {
                completion.observe(out);
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
    use super::FinishReason;
    use crate::{inspect::ModelOutcome, label::Labeled};

    #[test]
    fn every_documented_finish_reason_has_a_deliberate_outcome() {
        let outcomes: Vec<_> = FinishReason::ALL
            .iter()
            .map(|reason| (reason.label(), reason.outcome()))
            .collect();
        assert_eq!(
            outcomes,
            [
                ("stop", ModelOutcome::Completed),
                ("length", ModelOutcome::Incomplete),
                ("tool_calls", ModelOutcome::Completed),
                ("content_filter", ModelOutcome::Incomplete),
                ("function_call", ModelOutcome::Completed),
                ("insufficient_system_resource", ModelOutcome::Failed),
            ]
        );
    }
}
