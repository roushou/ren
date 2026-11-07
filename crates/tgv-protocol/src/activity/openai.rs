//! OpenAI Responses (public and Codex): the newest input items of a request,
//! and a response's output items, from a document or from streaming events.

use super::{
    ActivityLimitation, ActivityLimits, ActivityReader, RequestActivity, ResponseActivity,
    ResultStatus, ToolInput, ToolResult, ToolUse,
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
        let mut activity = RequestActivity::default();
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
            let kind = ActivityReader::str(item, "type");
            if kind.and_then(ToolOutputKind::from_label).is_some() {
                let Some(id) = ActivityReader::str(item, "call_id") else {
                    activity
                        .limitations
                        .push(ActivityLimitation::UnexpectedShape);
                    continue;
                };
                activity.tool_results.push(ToolResult {
                    id: id.to_owned(),
                    status: ResultStatus::Unreported,
                    output: ActivityReader::text(item.get("output")),
                });
            } else if kind.is_none_or(|kind| ItemKind::from_label(kind) == Some(ItemKind::Message))
                && Self::role(item) == Some(InputRole::User)
            {
                let text = ActivityReader::text(item.get("content"));
                if !text.is_empty() {
                    activity.user_text.push(text);
                }
            }
        }
        activity
    }

    fn role(item: &Map<String, Value>) -> Option<InputRole> {
        ActivityReader::str(item, "role").and_then(InputRole::from_label)
    }

    /// Whether the model produced this item: an assistant message, its
    /// reasoning, or a tool call.
    fn from_model(item: &Value) -> bool {
        let Some(item) = item.as_object() else {
            return false;
        };
        match ActivityReader::str(item, "type").and_then(ItemKind::from_label) {
            Some(ItemKind::Message) | None => Self::role(item) == Some(InputRole::Assistant),
            Some(kind) => kind == ItemKind::Reasoning || kind.is_tool_call(),
        }
    }
}

/// A Responses response: its output items, from a document, from
/// `response.output_item.done` events, or from the terminal response event.
pub(super) struct Response {
    limits: ActivityLimits,
}

impl Response {
    pub(super) fn new(limits: ActivityLimits) -> Self {
        Self { limits }
    }

    pub(super) fn document(&self, document: &Map<String, Value>) -> ResponseActivity {
        self.items(
            ActivityReader::array(document, "output")
                .iter()
                .filter_map(Value::as_object),
        )
    }

    pub(super) fn events(
        &self,
        events: impl Iterator<Item = Map<String, Value>>,
    ) -> ResponseActivity {
        let mut done = Vec::new();
        let mut terminal = None;
        for event in events {
            let kind = ActivityReader::str(&event, "type").and_then(EventKind::from_label);
            if kind == Some(EventKind::OutputItemDone) {
                if let Some(item) = event.get("item").and_then(Value::as_object) {
                    done.push(item.clone());
                }
            } else if matches!(
                kind,
                Some(EventKind::Completed | EventKind::Incomplete | EventKind::Failed)
            ) {
                terminal = event.get("response").and_then(Value::as_object).cloned();
            }
        }
        // Items arrive done one by one; the terminal event repeats them all
        // when it carries output, and is the only source when no item event
        // was captured.
        match (done.is_empty(), terminal) {
            (true, Some(response)) => self.document(&response),
            (false, Some(_) | None) | (true, None) => self.items(done.iter()),
        }
    }

    fn items<'i>(&self, items: impl Iterator<Item = &'i Map<String, Value>>) -> ResponseActivity {
        let mut activity = ResponseActivity::default();
        let mut replies = Vec::new();
        for item in items {
            let Some(kind) = ActivityReader::str(item, "type").and_then(ItemKind::from_label)
            else {
                continue;
            };
            if kind == ItemKind::Message {
                replies.push(ActivityReader::text(item.get("content")));
                continue;
            }
            if !kind.is_tool_call() || !activity.has_room(self.limits.max_tool_uses) {
                continue;
            }
            let input = match kind {
                ItemKind::FunctionCall | ItemKind::McpCall => {
                    ActivityReader::str(item, "arguments").map_or(
                        ToolInput::Json(Value::Null),
                        ActivityReader::input_from_text,
                    )
                }
                ItemKind::CustomToolCall => ToolInput::Text(
                    ActivityReader::str(item, "input")
                        .unwrap_or_default()
                        .to_owned(),
                ),
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
                id: ActivityReader::str(item, "call_id")
                    .or_else(|| ActivityReader::str(item, "id"))
                    .map(str::to_owned),
                kind: kind.label().to_owned(),
                name: ActivityReader::str(item, "name")
                    .unwrap_or_else(|| kind.label())
                    .to_owned(),
                input,
            });
        }
        activity.set_reply(replies);
        activity
    }
}
