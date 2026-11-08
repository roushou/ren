//! OpenAI Chat Completions: the newest user input of a request, and a
//! response's reply and tool calls, from a document or from streamed chunks.

use super::{
    ActivityLimitation, ActivityLimits, Fields, RequestActivity, ResponseActivity, ResultStatus,
    ToolInput, ToolResult, ToolUse,
};
use crate::chat::ToolCallKind;
use crate::label::Labeled;
use serde_json::{Map, Value};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Who wrote a message. `function` is the deprecated form of `tool`.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ChatRole {
    System,
    Developer,
    User,
    Assistant,
    Tool,
    Function,
}

/// A Chat Completions request body.
pub(super) struct Request<'a>(&'a Map<String, Value>);

impl<'a> Request<'a> {
    pub(super) fn new(body: &'a Map<String, Value>) -> Self {
        Self(body)
    }

    /// The messages after the last assistant message: on the first call of
    /// a turn, the prompt; later, the results of the tools the previous
    /// response asked for.
    pub(super) fn activity(self) -> RequestActivity {
        let mut activity = RequestActivity {
            // `max_tokens` is the deprecated name, still widely sent.
            max_output_tokens: self
                .0
                .get("max_completion_tokens")
                .or_else(|| self.0.get("max_tokens"))
                .and_then(Value::as_u64),
            ..RequestActivity::default()
        };
        let Some(messages) = self.0.get("messages").and_then(Value::as_array) else {
            activity
                .limitations
                .push(ActivityLimitation::UnexpectedShape);
            return activity;
        };
        let role = |message: &Value| {
            message
                .get("role")
                .and_then(Value::as_str)
                .and_then(ChatRole::from_label)
        };
        let start = messages
            .iter()
            .rposition(|message| role(message) == Some(ChatRole::Assistant))
            .map_or(0, |index| index.saturating_add(1));
        for message in messages.get(start..).unwrap_or_default() {
            let Some(fields) = message.as_object().map(Fields) else {
                activity
                    .limitations
                    .push(ActivityLimitation::UnexpectedShape);
                continue;
            };
            match role(message) {
                Some(ChatRole::User) => {
                    let text = fields.text("content");
                    if !text.is_empty() {
                        activity.user_text.push(text);
                    }
                }
                Some(ChatRole::Tool) => {
                    let Some(id) = fields.str("tool_call_id") else {
                        activity
                            .limitations
                            .push(ActivityLimitation::UnexpectedShape);
                        continue;
                    };
                    activity.tool_results.push(ToolResult {
                        id: id.to_owned(),
                        status: ResultStatus::Unreported,
                        output: fields.text("content"),
                    });
                }
                // A deprecated function result names no call to answer.
                Some(
                    ChatRole::System
                    | ChatRole::Developer
                    | ChatRole::Assistant
                    | ChatRole::Function,
                ) => {}
                None => {
                    activity
                        .limitations
                        .push(ActivityLimitation::UnexpectedShape);
                }
            }
        }
        activity
    }
}

/// A tool call being read: whole from a document, or assembled from the
/// deltas that share its `index`.
#[derive(Debug, Default)]
struct Call {
    id: Option<String>,
    kind: Option<ToolCallKind>,
    name: Option<String>,
    /// Arguments (or a custom tool's input), concatenated.
    input: String,
}

impl Call {
    /// Take what one delta (or a whole call) adds.
    fn absorb(&mut self, call: &Map<String, Value>) {
        if let Some(id) = Fields(call).str("id") {
            self.id = Some(id.to_owned());
        }
        if let Some(kind) = Fields(call).str("type").and_then(ToolCallKind::from_label) {
            self.kind = Some(kind);
        }
        let kind = self.kind.unwrap_or(ToolCallKind::Function);
        let Some(member) = call.get(kind.member()).and_then(Value::as_object) else {
            return;
        };
        if let Some(name) = Fields(member).str("name") {
            self.name = Some(name.to_owned());
        }
        self.input
            .push_str(Fields(member).str(kind.input()).unwrap_or_default());
    }

    fn tool_use(self) -> ToolUse {
        let kind = self.kind.unwrap_or(ToolCallKind::Function);
        let input = match (kind, self.input.is_empty()) {
            (ToolCallKind::Function, true) => ToolInput::Json(Value::Null),
            (ToolCallKind::Function, false) => ToolInput::parse(&self.input),
            (ToolCallKind::Custom, _) => ToolInput::Text(self.input),
        };
        ToolUse {
            id: self.id,
            kind: kind.label().to_owned(),
            name: self.name.unwrap_or_default(),
            input,
        }
    }
}

/// One choice being read.
#[derive(Debug, Default)]
struct Choice {
    text: String,
    /// By the call's `index` within the choice.
    calls: Vec<Option<Call>>,
    finished: bool,
}

impl Choice {
    fn call(&mut self, index: usize) -> Option<&mut Call> {
        if index >= self.calls.len() {
            self.calls.resize_with(index.saturating_add(1), || None);
        }
        self.calls
            .get_mut(index)
            .map(|slot| slot.get_or_insert_with(Call::default))
    }

    /// A document's `message` or a chunk's `delta`. Calls without an
    /// `index` (a document's) are placed in order.
    fn absorb(&mut self, body: &Map<String, Value>) {
        self.text
            .push_str(Fields(body).str("content").unwrap_or_default());
        for (position, call) in Fields(body)
            .array("tool_calls")
            .iter()
            .filter_map(Value::as_object)
            .enumerate()
        {
            let index = call
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
                .unwrap_or(position);
            if let Some(slot) = self.call(index) {
                slot.absorb(call);
            }
        }
    }
}

/// A Chat Completions response: a document's choices, or the choices its
/// chunks build, each by its `index`.
#[derive(Debug)]
pub(super) struct Response {
    limits: ActivityLimits,
    choices: Vec<Option<Choice>>,
}

impl Response {
    pub(super) fn new(limits: ActivityLimits) -> Self {
        Self {
            limits,
            choices: Vec::new(),
        }
    }

    /// A complete response document: its choices, all finished.
    pub(super) fn document(&mut self, document: &Map<String, Value>) {
        self.choices.clear();
        self.choices(document, "message", true);
    }

    pub(super) fn event(&mut self, chunk: &Map<String, Value>) {
        self.choices(chunk, "delta", false);
    }

    fn choices(&mut self, object: &Map<String, Value>, body: &str, whole: bool) {
        for (position, choice) in Fields(object)
            .array("choices")
            .iter()
            .filter_map(Value::as_object)
            .enumerate()
        {
            let index = choice
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
                .unwrap_or(position);
            if index >= self.choices.len() {
                self.choices.resize_with(index.saturating_add(1), || None);
            }
            let Some(slot) = self.choices.get_mut(index) else {
                continue;
            };
            let reading = slot.get_or_insert_with(Choice::default);
            if let Some(body) = choice.get(body).and_then(Value::as_object) {
                reading.absorb(body);
            }
            if whole
                || choice
                    .get("finish_reason")
                    .is_some_and(|reason| !reason.is_null())
            {
                reading.finished = true;
            }
        }
    }

    pub(super) fn finish(self) -> ResponseActivity {
        let mut activity = ResponseActivity::default();
        let mut replies = Vec::new();
        for choice in self.choices.into_iter().flatten() {
            replies.push(choice.text);
            for call in choice.calls.into_iter().flatten() {
                if !activity.has_room(self.limits.max_tool_uses) {
                    continue;
                }
                if !choice.finished
                    && !activity
                        .limitations
                        .contains(&ActivityLimitation::UnfinishedToolCall)
                {
                    activity
                        .limitations
                        .push(ActivityLimitation::UnfinishedToolCall);
                }
                activity.tool_uses.push(call.tool_use());
            }
        }
        activity.set_reply(replies);
        activity
    }
}
