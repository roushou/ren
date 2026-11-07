//! Anthropic Messages: the newest user input of a request, and a response's
//! text and tool calls, from a document or from streaming events.

use super::{
    ActivityLimitation, ActivityLimits, ActivityReader, RequestActivity, ResponseActivity,
    ResultStatus, ToolInput, ToolResult, ToolUse,
};
use crate::label::Labeled;
use crate::messages::{BlockKind, EventKind};
use serde_json::{Map, Value};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Who wrote a message. Claude Code also sends `system` messages inside
/// `messages`: context, neither the user's words nor tool results.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

/// Content block types a request's user messages carry. Others (images,
/// documents) hold no text this reads.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum UserBlockKind {
    Text,
    ToolResult,
}

/// Streaming delta types that add to a block.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum DeltaKind {
    TextDelta,
    InputJsonDelta,
}

/// A Messages request body.
pub(super) struct Request<'a>(&'a Map<String, Value>);

impl<'a> Request<'a> {
    pub(super) fn new(body: &'a Map<String, Value>) -> Self {
        Self(body)
    }

    /// The user messages after the last assistant message: on the first
    /// call of a turn, the prompt; later, the results of the tools the
    /// previous response asked for.
    pub(super) fn activity(self) -> RequestActivity {
        let mut activity = RequestActivity::default();
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
                .and_then(MessageRole::from_label)
        };
        let start = messages
            .iter()
            .rposition(|message| role(message) == Some(MessageRole::Assistant))
            .map_or(0, |index| index.saturating_add(1));
        for message in messages.get(start..).unwrap_or_default() {
            match (role(message), message.get("content")) {
                (Some(MessageRole::User), Some(Value::String(text))) => {
                    activity.user_text.push(text.clone());
                }
                (Some(MessageRole::User), Some(Value::Array(blocks))) => {
                    for block in blocks.iter().filter_map(Value::as_object) {
                        Self::block(block, &mut activity);
                    }
                }
                (Some(MessageRole::User), _) | (None, _) => {
                    activity
                        .limitations
                        .push(ActivityLimitation::UnexpectedShape);
                }
                (Some(MessageRole::Assistant | MessageRole::System), _) => {}
            }
        }
        activity
    }

    fn block(block: &Map<String, Value>, activity: &mut RequestActivity) {
        match ActivityReader::str(block, "type").and_then(UserBlockKind::from_label) {
            Some(UserBlockKind::Text) => {
                if let Some(text) = ActivityReader::str(block, "text") {
                    activity.user_text.push(text.to_owned());
                }
            }
            Some(UserBlockKind::ToolResult) => {
                let Some(id) = ActivityReader::str(block, "tool_use_id") else {
                    activity
                        .limitations
                        .push(ActivityLimitation::UnexpectedShape);
                    return;
                };
                let status = match block.get("is_error").and_then(Value::as_bool) {
                    Some(true) => ResultStatus::Failed,
                    Some(false) | None => ResultStatus::Succeeded,
                };
                activity.tool_results.push(ToolResult {
                    id: id.to_owned(),
                    status,
                    output: ActivityReader::text(block.get("content")),
                });
            }
            None => {}
        }
    }
}

/// A content block being read.
#[derive(Debug)]
struct Block {
    kind: BlockKind,
    id: Option<String>,
    name: Option<String>,
    /// A document's or a start event's input.
    input: Option<Value>,
    /// Streamed input JSON, concatenated.
    partial_input: String,
    text: String,
    closed: bool,
}

/// A Messages response: a document's content, or the blocks its events
/// build.
#[derive(Debug)]
pub(super) struct Response {
    limits: ActivityLimits,
    /// By stream index.
    blocks: Vec<Option<Block>>,
    limitations: Vec<ActivityLimitation>,
}

impl Response {
    pub(super) fn new(limits: ActivityLimits) -> Self {
        Self {
            limits,
            blocks: Vec::new(),
            limitations: Vec::new(),
        }
    }

    /// A complete response document: its content blocks, all closed.
    pub(super) fn document(&mut self, document: Map<String, Value>) {
        self.blocks.clear();
        for (index, block) in ActivityReader::array(&document, "content")
            .iter()
            .filter_map(Value::as_object)
            .enumerate()
        {
            if let Some(mut block) = Self::start(block) {
                block.closed = true;
                self.put(index, block);
            }
        }
    }

    pub(super) fn event(&mut self, event: &Map<String, Value>) {
        let index = event
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok());
        match (
            ActivityReader::str(event, "type").and_then(EventKind::from_label),
            index,
        ) {
            (Some(EventKind::ContentBlockStart), Some(index)) => {
                if let Some(block) = event
                    .get("content_block")
                    .and_then(Value::as_object)
                    .and_then(Self::start)
                {
                    self.put(index, block);
                }
            }
            (Some(EventKind::ContentBlockDelta), Some(index)) => {
                let Some(delta) = event.get("delta").and_then(Value::as_object) else {
                    return;
                };
                let Some(Some(block)) = self.blocks.get_mut(index) else {
                    return;
                };
                match ActivityReader::str(delta, "type").and_then(DeltaKind::from_label) {
                    Some(DeltaKind::TextDelta) => {
                        block
                            .text
                            .push_str(ActivityReader::str(delta, "text").unwrap_or_default());
                    }
                    Some(DeltaKind::InputJsonDelta) => {
                        block.partial_input.push_str(
                            ActivityReader::str(delta, "partial_json").unwrap_or_default(),
                        );
                    }
                    None => {}
                }
            }
            (Some(EventKind::ContentBlockStop), Some(index)) => {
                if let Some(Some(block)) = self.blocks.get_mut(index) {
                    block.closed = true;
                }
            }
            (
                Some(
                    EventKind::ContentBlockStart
                    | EventKind::ContentBlockDelta
                    | EventKind::ContentBlockStop,
                ),
                None,
            )
            | (
                Some(
                    EventKind::MessageStart
                    | EventKind::MessageDelta
                    | EventKind::MessageStop
                    | EventKind::Ping
                    | EventKind::Error,
                ),
                _,
            )
            | (None, _) => {}
        }
    }

    /// A block as a document or a start event first shows it.
    fn start(block: &Map<String, Value>) -> Option<Block> {
        let kind = ActivityReader::str(block, "type").and_then(BlockKind::from_label)?;
        Some(Block {
            kind,
            id: ActivityReader::str(block, "id").map(str::to_owned),
            name: ActivityReader::str(block, "name").map(str::to_owned),
            input: block.get("input").cloned(),
            partial_input: String::new(),
            text: ActivityReader::str(block, "text")
                .unwrap_or_default()
                .to_owned(),
            closed: false,
        })
    }

    fn put(&mut self, index: usize, block: Block) {
        if index >= self.blocks.len() {
            self.blocks.resize_with(index.saturating_add(1), || None);
        }
        if let Some(slot) = self.blocks.get_mut(index) {
            *slot = Some(block);
        }
    }

    pub(super) fn finish(self) -> ResponseActivity {
        let mut activity = ResponseActivity {
            limitations: self.limitations,
            ..ResponseActivity::default()
        };
        let mut replies = Vec::new();
        for block in self.blocks.into_iter().flatten() {
            match block.kind {
                BlockKind::Text => replies.push(block.text),
                BlockKind::ToolUse | BlockKind::ServerToolUse | BlockKind::McpToolUse => {
                    if !activity.has_room(self.limits.max_tool_uses) {
                        continue;
                    }
                    if !block.closed {
                        activity
                            .limitations
                            .push(ActivityLimitation::UnfinishedToolCall);
                    }
                    let input = if block.partial_input.is_empty() {
                        block
                            .input
                            .map_or(ToolInput::Json(Value::Null), ToolInput::Json)
                    } else {
                        ActivityReader::input_from_text(&block.partial_input)
                    };
                    activity.tool_uses.push(ToolUse {
                        id: block.id,
                        kind: block.kind.label().to_owned(),
                        name: block.name.unwrap_or_default(),
                        input,
                    });
                }
                BlockKind::Thinking
                | BlockKind::RedactedThinking
                | BlockKind::WebSearchToolResult
                | BlockKind::McpToolResult => {}
            }
        }
        activity.set_reply(replies);
        activity
    }
}
