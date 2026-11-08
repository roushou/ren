//! OpenAI Responses (public and Codex): the newest input items of a request,
//! and a response's output items, from a document or from streaming events.

use super::{
    ActivityLimitation, ActivityLimits, Fields, RequestActivity, ResponseActivity, ResultStatus,
    ToolInput, ToolResult, ToolUse,
};
use crate::label::Labeled;
use crate::responses::{EventKind, ItemKind};
use serde_json::{Map, Value};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Who wrote an input message.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum InputRole {
    User,
    Assistant,
    System,
    Developer,
}

/// Input items that carry a tool's output back to the model.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ToolOutputKind {
    #[strum(to_string = "function_call_output")]
    Function,
    #[strum(to_string = "custom_tool_call_output")]
    CustomTool,
    #[strum(to_string = "local_shell_call_output")]
    LocalShell,
    #[strum(to_string = "shell_call_output")]
    Shell,
    #[strum(to_string = "apply_patch_call_output")]
    ApplyPatch,
}

/// A Responses request body, or a WebSocket `response.create` message.
pub(super) struct Request<'a>(&'a Map<String, Value>);

impl<'a> Request<'a> {
    pub(super) fn new(body: &'a Map<String, Value>) -> Self {
        // Over WebSocket the request may be nested under `response`.
        Self(
            body.get("response")
                .and_then(Value::as_object)
                .unwrap_or(body),
        )
    }

    /// The input items after the model's last output item: the user's new
    /// message, or the outputs of the tools the model called. Over a
    /// WebSocket that continues a previous response, the input holds only
    /// these.
    pub(super) fn activity(self) -> RequestActivity {
        let mut activity = RequestActivity {
            max_output_tokens: self.0.get("max_output_tokens").and_then(Value::as_u64),
            ..RequestActivity::default()
        };
        let items = match self.0.get("input") {
            Some(Value::String(text)) => {
                activity.user_text.push(text.clone());
                return activity;
            }
            Some(Value::Array(items)) => items,
            Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_)) | None => {
                activity
                    .limitations
                    .push(ActivityLimitation::UnexpectedShape);
                return activity;
            }
        };
        let start = items
            .iter()
            .rposition(Self::from_model)
            .map_or(0, |index| index.saturating_add(1));
        for item in items
            .get(start..)
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_object)
        {
            let kind = Fields(item).str("type");
            if kind.and_then(ToolOutputKind::from_label).is_some() {
                let Some(id) = Fields(item).str("call_id") else {
                    activity
                        .limitations
                        .push(ActivityLimitation::UnexpectedShape);
                    continue;
                };
                activity.tool_results.push(ToolResult {
                    id: id.to_owned(),
                    status: ResultStatus::Unreported,
                    output: Fields(item).text("output"),
                });
            } else if kind.is_none_or(|kind| ItemKind::from_label(kind) == Some(ItemKind::Message))
                && Self::role(item) == Some(InputRole::User)
            {
                let text = Fields(item).text("content");
                if !text.is_empty() {
                    activity.user_text.push(text);
                }
            }
        }
        activity
    }

    fn role(item: &Map<String, Value>) -> Option<InputRole> {
        Fields(item).str("role").and_then(InputRole::from_label)
    }

    /// Whether the model produced this item: an assistant message, its
    /// reasoning, or a tool call.
    fn from_model(item: &Value) -> bool {
        let Some(item) = item.as_object() else {
            return false;
        };
        match Fields(item).str("type").and_then(ItemKind::from_label) {
            Some(ItemKind::Message) | None => Self::role(item) == Some(InputRole::Assistant),
            Some(kind) => kind == ItemKind::Reasoning || kind.is_tool_call(),
        }
    }
}

/// A Responses response: its output items, from a document, from
/// `response.output_item.done` events, or from the terminal response event.
/// Fed one event at a time.
#[derive(Debug)]
pub(super) struct Response {
    limits: ActivityLimits,
    done: Vec<Map<String, Value>>,
    terminal: Option<Map<String, Value>>,
}

impl Response {
    pub(super) fn new(limits: ActivityLimits) -> Self {
        Self {
            limits,
            done: Vec::new(),
            terminal: None,
        }
    }

    /// A complete response document: its output is the whole answer.
    pub(super) fn document(&mut self, document: Map<String, Value>) {
        self.done.clear();
        self.terminal = Some(document);
    }

    pub(super) fn event(&mut self, mut event: Map<String, Value>) {
        let kind = Fields(&event).str("type").and_then(EventKind::from_label);
        if kind == Some(EventKind::OutputItemDone) {
            if let Some(Value::Object(item)) = event.remove("item") {
                self.done.push(item);
            }
        } else if matches!(
            kind,
            Some(EventKind::Completed | EventKind::Incomplete | EventKind::Failed)
        ) && let Some(Value::Object(response)) = event.remove("response")
        {
            self.terminal = Some(response);
        }
    }

    /// Items arrive done one by one; the terminal event repeats them all
    /// when it carries output, and is the only source when no item event
    /// was captured.
    pub(super) fn finish(self) -> ResponseActivity {
        match (self.done.is_empty(), &self.terminal) {
            (true, Some(response)) => self.items(
                Fields(response)
                    .array("output")
                    .iter()
                    .filter_map(Value::as_object),
            ),
            (false, Some(_) | None) | (true, None) => self.items(self.done.iter()),
        }
    }

    fn items<'i>(&self, items: impl Iterator<Item = &'i Map<String, Value>>) -> ResponseActivity {
        let mut activity = ResponseActivity::default();
        let mut replies = Vec::new();
        for item in items {
            let Some(kind) = Fields(item).str("type").and_then(ItemKind::from_label) else {
                continue;
            };
            if kind == ItemKind::Message {
                replies.push(Fields(item).text("content"));
                continue;
            }
            if !kind.is_tool_call() || !activity.has_room(self.limits.max_tool_uses) {
                continue;
            }
            let input = match kind {
                ItemKind::FunctionCall | ItemKind::McpCall => Fields(item)
                    .str("arguments")
                    .map_or(ToolInput::Json(Value::Null), ToolInput::parse),
                ItemKind::CustomToolCall => {
                    ToolInput::Text(Fields(item).str("input").unwrap_or_default().to_owned())
                }
                ItemKind::LocalShellCall | ItemKind::ShellCall | ItemKind::WebSearchCall => {
                    ToolInput::Json(item.get("action").cloned().unwrap_or(Value::Null))
                }
                ItemKind::ApplyPatchCall => {
                    ToolInput::Json(item.get("operation").cloned().unwrap_or(Value::Null))
                }
                ItemKind::Message
                | ItemKind::Reasoning
                | ItemKind::ComputerCall
                | ItemKind::McpApprovalRequest
                | ItemKind::FileSearchCall
                | ItemKind::CodeInterpreterCall
                | ItemKind::ImageGenerationCall
                | ItemKind::ToolSearchCall => ToolInput::Json(Value::Object(item.clone())),
            };
            activity.tool_uses.push(ToolUse {
                id: Fields(item)
                    .str("call_id")
                    .or_else(|| Fields(item).str("id"))
                    .map(str::to_owned),
                kind: kind.label().to_owned(),
                name: Fields(item)
                    .str("name")
                    .unwrap_or_else(|| kind.label())
                    .to_owned(),
                input,
            });
        }
        activity.set_reply(replies);
        activity
    }
}
