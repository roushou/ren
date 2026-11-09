//! What the fake provider answers, as pure data: a request's wire format and
//! body in, a scripted reply out. No I/O; the server around it only writes
//! the reply's pieces, waiting between them as told.
//!
//! The requested model picks the behaviour, so a seed scenario (or a person
//! typing a model name into a real client) chooses what happens. Any other
//! model gets a short text reply that quotes the newest user text.

use serde_json::{Value, json};
use std::time::Duration;

/// The wire format a request arrived in, from its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    /// Anthropic Messages (`…/v1/messages`), also DeepSeek's.
    Messages,
    /// Anthropic's token counter (`…/v1/messages/count_tokens`).
    CountTokens,
    /// OpenAI Responses (`…/responses`, `…/v1/responses`), also Codex's.
    Responses,
    /// OpenAI Chat Completions (`…/chat/completions`).
    ChatCompletions,
    /// A model listing (`…/models`).
    Models,
}

impl Wire {
    /// The wire format for a request path, or `None` for a path no provider
    /// TGV forwards to has.
    pub fn for_path(path: &str) -> Option<Self> {
        let path = path.trim_end_matches('/');
        if path.ends_with("/messages/count_tokens") {
            Some(Self::CountTokens)
        } else if path.ends_with("/messages") {
            Some(Self::Messages)
        } else if path.ends_with("/responses") || path.ends_with("/responses/compact") {
            Some(Self::Responses)
        } else if path.ends_with("/chat/completions") {
            Some(Self::ChatCompletions)
        } else if path.ends_with("/models") {
            Some(Self::Models)
        } else {
            None
        }
    }
}

/// What the requested model asks the fake to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    /// A short text reply quoting the request.
    Text,
    /// A tool call, then the end of the turn.
    Tools,
    /// A reply of a few thousand words.
    Long,
    /// An upstream error (`500`).
    Error,
    /// A rate limit (`429` with `retry-after`).
    RateLimited,
    /// Text streamed slowly, a piece every half second.
    Slow,
    /// A stream that stops before its terminal event.
    Cut,
}

impl Behaviour {
    /// The behaviour a model name selects: `fake-tools`, `fake-long`,
    /// `fake-error`, `fake-429`, `fake-slow`, `fake-cut`; anything else is
    /// plain text.
    pub fn for_model(model: &str) -> Self {
        match model.strip_prefix("fake-") {
            Some("tools") => Self::Tools,
            Some("long") => Self::Long,
            Some("error") => Self::Error,
            Some("429") => Self::RateLimited,
            Some("slow") => Self::Slow,
            Some("cut") => Self::Cut,
            Some(_) | None => Self::Text,
        }
    }
}

/// One piece of a reply body, written after waiting `after`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub after: Duration,
    pub bytes: String,
}

/// A scripted reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub headers: Vec<(&'static str, String)>,
    pub pieces: Vec<Piece>,
}

/// One request, as the script reads it.
pub struct Request<'a> {
    pub wire: Wire,
    pub body: &'a [u8],
    /// A number unique to this request, for response identifiers.
    pub serial: u64,
}

impl Reply {
    const SSE: &'static str = "text/event-stream";
    const JSON: &'static str = "application/json";

    pub fn to(request: &Request<'_>) -> Self {
        let body: Value = serde_json::from_slice(request.body).unwrap_or(Value::Null);
        let model = body["model"].as_str().unwrap_or("fake-model").to_owned();
        let stream = body["stream"].as_bool().unwrap_or(false);
        // A rough, deterministic count: four bytes a token.
        let input_tokens = u64::try_from(request.body.len() / 4).unwrap_or(u64::MAX);
        let turn = Turn {
            model,
            serial: request.serial,
            input_tokens,
            prompt: Prompt::newest(request.wire, &body),
        };
        match request.wire {
            Wire::Models => Self::json(200, &turn.models()),
            Wire::CountTokens => Self::json(200, &json!({ "input_tokens": input_tokens })),
            Wire::Messages | Wire::Responses | Wire::ChatCompletions => {
                match Behaviour::for_model(&turn.model) {
                    Behaviour::Error => Self::error(request.wire, 500, "fake upstream error"),
                    Behaviour::RateLimited => {
                        let mut reply =
                            Self::error(request.wire, 429, "fake rate limit; try again later");
                        reply.headers.push(("retry-after", "5".to_owned()));
                        reply
                    }
                    behaviour @ (Behaviour::Text
                    | Behaviour::Tools
                    | Behaviour::Long
                    | Behaviour::Slow
                    | Behaviour::Cut) => {
                        let output = Output::of(behaviour, &turn);
                        match (request.wire, stream) {
                            (Wire::Messages, true) => turn.messages_stream(&output, behaviour),
                            (Wire::Messages, false) => Self::json(200, &turn.message(&output)),
                            // Codex always streams Responses.
                            (Wire::Responses, _) => turn.responses_stream(&output, behaviour),
                            (Wire::ChatCompletions, true) => turn.chat_stream(&output, behaviour),
                            (Wire::ChatCompletions, false) => {
                                Self::json(200, &turn.chat_completion(&output))
                            }
                            (Wire::CountTokens | Wire::Models, _) => {
                                Self::json(200, &turn.models())
                            }
                        }
                    }
                }
            }
        }
    }

    fn json(status: u16, value: &Value) -> Self {
        Self {
            status,
            content_type: Self::JSON,
            headers: Vec::new(),
            pieces: vec![Piece {
                after: Duration::ZERO,
                bytes: value.to_string(),
            }],
        }
    }

    fn error(wire: Wire, status: u16, message: &str) -> Self {
        let body = match wire {
            Wire::Messages | Wire::CountTokens | Wire::Models => json!({
                "type": "error",
                "error": { "type": if status == 429 { "rate_limit_error" } else { "api_error" }, "message": message },
            }),
            Wire::Responses | Wire::ChatCompletions => json!({
                "error": { "type": if status == 429 { "rate_limit_exceeded" } else { "server_error" }, "message": message },
            }),
        };
        Self::json(status, &body)
    }

    fn sse(events: Vec<(Duration, String)>) -> Self {
        Self {
            status: 200,
            content_type: Self::SSE,
            headers: Vec::new(),
            pieces: events
                .into_iter()
                .map(|(after, bytes)| Piece { after, bytes })
                .collect(),
        }
    }
}

/// The newest user text in a request, for the reply to quote.
struct Prompt;

impl Prompt {
    const QUOTED: usize = 120;

    fn newest(wire: Wire, body: &Value) -> String {
        let text = match wire {
            Wire::Messages | Wire::CountTokens | Wire::ChatCompletions => body["messages"]
                .as_array()
                .and_then(|messages| {
                    messages
                        .iter()
                        .rev()
                        .find(|message| message["role"] == "user")
                })
                .map(|message| Self::text(&message["content"])),
            Wire::Responses => match &body["input"] {
                Value::String(text) => Some(text.clone()),
                Value::Array(items) => items
                    .iter()
                    .rev()
                    .find(|item| item["role"] == "user")
                    .map(|item| Self::text(&item["content"])),
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => None,
            },
            Wire::Models => None,
        };
        let text = text.unwrap_or_default();
        let text = text.trim();
        match text.char_indices().nth(Self::QUOTED) {
            Some((cut, _)) => format!("{}…", &text[..cut]),
            None => text.to_owned(),
        }
    }

    /// Text from a string, or from the text parts of a content list.
    fn text(content: &Value) -> String {
        match content {
            Value::String(text) => text.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join(" "),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => String::new(),
        }
    }
}

/// What the reply says: text, a tool call, or both.
struct Output {
    text: Vec<String>,
    tool: Option<(&'static str, Value)>,
}

impl Output {
    fn of(behaviour: Behaviour, turn: &Turn) -> Self {
        let quoted = if turn.prompt.is_empty() {
            "This is a fake reply from the TGV dev provider.".to_owned()
        } else {
            format!("Fake reply to: {}", turn.prompt)
        };
        match behaviour {
            Behaviour::Tools => Self {
                text: vec!["Let me look at the files.".to_owned()],
                tool: Some((
                    "Bash",
                    json!({ "command": "ls -la", "description": "List files" }),
                )),
            },
            Behaviour::Long => Self {
                text: (0..120)
                    .map(|n| format!("Paragraph {n}: {quoted} The fake provider writes this long reply so pages can be checked with a lot of text. "))
                    .collect(),
                tool: None,
            },
            Behaviour::Text
            | Behaviour::Slow
            | Behaviour::Cut
            | Behaviour::Error
            | Behaviour::RateLimited => Self {
                text: quoted
                    .split_inclusive(' ')
                    .map(str::to_owned)
                    .collect(),
                tool: None,
            },
        }
    }

    fn joined(&self) -> String {
        self.text.concat()
    }

    fn output_tokens(&self) -> u64 {
        u64::try_from(self.joined().len() / 4 + 1).unwrap_or(u64::MAX)
    }
}

/// One fake model turn.
struct Turn {
    model: String,
    serial: u64,
    input_tokens: u64,
    prompt: String,
}

impl Turn {
    fn id(&self, prefix: &str) -> String {
        format!("{prefix}_fake{:06}", self.serial)
    }

    fn pace(behaviour: Behaviour) -> Duration {
        match behaviour {
            Behaviour::Slow => Duration::from_millis(500),
            Behaviour::Text
            | Behaviour::Tools
            | Behaviour::Long
            | Behaviour::Error
            | Behaviour::RateLimited
            | Behaviour::Cut => Duration::from_millis(15),
        }
    }

    fn event(name: Option<&str>, data: &Value) -> String {
        match name {
            Some(name) => format!("event: {name}\ndata: {data}\n\n"),
            None => format!("data: {data}\n\n"),
        }
    }

    fn models(&self) -> Value {
        let data: Vec<Value> = [
            "fake-model",
            "fake-tools",
            "fake-long",
            "fake-slow",
            "fake-cut",
            "fake-error",
            "fake-429",
        ]
        .iter()
        .map(|id| json!({ "id": id, "type": "model", "object": "model", "display_name": id }))
        .collect();
        json!({ "object": "list", "data": data })
    }

    fn message(&self, output: &Output) -> Value {
        let mut content = vec![json!({ "type": "text", "text": output.joined() })];
        if let Some((name, input)) = &output.tool {
            content.push(
                json!({ "type": "tool_use", "id": self.id("toolu"), "name": name, "input": input }),
            );
        }
        json!({
            "id": self.id("msg"),
            "type": "message",
            "role": "assistant",
            "model": self.model,
            "content": content,
            "stop_reason": if output.tool.is_some() { "tool_use" } else { "end_turn" },
            "usage": { "input_tokens": self.input_tokens, "output_tokens": output.output_tokens() },
        })
    }

    fn messages_stream(&self, output: &Output, behaviour: Behaviour) -> Reply {
        let pace = Self::pace(behaviour);
        let mut events = vec![(
            Duration::ZERO,
            Self::event(
                Some("message_start"),
                &json!({ "type": "message_start", "message": {
                    "id": self.id("msg"), "type": "message", "role": "assistant", "model": self.model,
                    "content": [], "stop_reason": null,
                    "usage": { "input_tokens": self.input_tokens, "output_tokens": 1 },
                }}),
            ),
        )];
        events.push((
            Duration::ZERO,
            Self::event(
                Some("content_block_start"),
                &json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
            ),
        ));
        for text in &output.text {
            events.push((
                pace,
                Self::event(
                    Some("content_block_delta"),
                    &json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": text } }),
                ),
            ));
        }
        events.push((
            Duration::ZERO,
            Self::event(
                Some("content_block_stop"),
                &json!({ "type": "content_block_stop", "index": 0 }),
            ),
        ));
        if behaviour == Behaviour::Cut {
            return Reply::sse(events);
        }
        if let Some((name, input)) = &output.tool {
            events.push((
                pace,
                Self::event(
                    Some("content_block_start"),
                    &json!({ "type": "content_block_start", "index": 1, "content_block": {
                        "type": "tool_use", "id": self.id("toolu"), "name": name, "input": {},
                    }}),
                ),
            ));
            events.push((
                pace,
                Self::event(
                    Some("content_block_delta"),
                    &json!({ "type": "content_block_delta", "index": 1, "delta": {
                        "type": "input_json_delta", "partial_json": input.to_string(),
                    }}),
                ),
            ));
            events.push((
                Duration::ZERO,
                Self::event(
                    Some("content_block_stop"),
                    &json!({ "type": "content_block_stop", "index": 1 }),
                ),
            ));
        }
        events.push((
            Duration::ZERO,
            Self::event(
                Some("message_delta"),
                &json!({ "type": "message_delta",
                    "delta": { "stop_reason": if output.tool.is_some() { "tool_use" } else { "end_turn" } },
                    "usage": { "output_tokens": output.output_tokens() } }),
            ),
        ));
        events.push((
            Duration::ZERO,
            Self::event(Some("message_stop"), &json!({ "type": "message_stop" })),
        ));
        Reply::sse(events)
    }

    fn response_object(&self, output: &Output, status: &str) -> Value {
        let mut items = vec![json!({
            "type": "message", "id": self.id("msg"), "role": "assistant", "status": "completed",
            "content": [{ "type": "output_text", "text": output.joined(), "annotations": [] }],
        })];
        if let Some((name, input)) = &output.tool {
            items.push(json!({
                "type": "function_call", "id": self.id("fc"), "call_id": self.id("call"),
                "name": name, "arguments": input.to_string(), "status": "completed",
            }));
        }
        json!({
            "id": self.id("resp"),
            "object": "response",
            "model": self.model,
            "status": status,
            "output": if status == "completed" { Value::Array(items) } else { json!([]) },
            "usage": if status == "completed" {
                json!({ "input_tokens": self.input_tokens, "input_tokens_details": { "cached_tokens": 0 },
                        "output_tokens": output.output_tokens(), "total_tokens": self.input_tokens + output.output_tokens() })
            } else { Value::Null },
        })
    }

    /// The Responses events of one turn, as Codex reads them over SSE or a
    /// WebSocket.
    fn responses_events(&self, output: &Output, behaviour: Behaviour) -> Vec<(Duration, Value)> {
        let pace = Self::pace(behaviour);
        let mut events = vec![
            (
                Duration::ZERO,
                json!({ "type": "response.created", "response": self.response_object(output, "in_progress") }),
            ),
            (
                Duration::ZERO,
                json!({ "type": "response.output_item.added", "output_index": 0, "item": {
                    "type": "message", "id": self.id("msg"), "role": "assistant", "status": "in_progress", "content": [],
                }}),
            ),
        ];
        for text in &output.text {
            events.push((
                pace,
                json!({ "type": "response.output_text.delta", "item_id": self.id("msg"),
                        "output_index": 0, "content_index": 0, "delta": text }),
            ));
        }
        if behaviour == Behaviour::Cut {
            return events;
        }
        events.push((
            Duration::ZERO,
            json!({ "type": "response.output_item.done", "output_index": 0, "item": {
                "type": "message", "id": self.id("msg"), "role": "assistant", "status": "completed",
                "content": [{ "type": "output_text", "text": output.joined(), "annotations": [] }],
            }}),
        ));
        if let Some((name, input)) = &output.tool {
            events.push((
                pace,
                json!({ "type": "response.output_item.done", "output_index": 1, "item": {
                    "type": "function_call", "id": self.id("fc"), "call_id": self.id("call"),
                    "name": name, "arguments": input.to_string(), "status": "completed",
                }}),
            ));
        }
        events.push((
            Duration::ZERO,
            json!({ "type": "response.completed", "response": self.response_object(output, "completed") }),
        ));
        events
    }

    fn responses_stream(&self, output: &Output, behaviour: Behaviour) -> Reply {
        Reply::sse(
            self.responses_events(output, behaviour)
                .into_iter()
                .map(|(after, event)| {
                    let name = event["type"].as_str().map(str::to_owned);
                    (after, Self::event(name.as_deref(), &event))
                })
                .collect(),
        )
    }

    fn chat_completion(&self, output: &Output) -> Value {
        let mut message = json!({ "role": "assistant", "content": output.joined() });
        if let Some((name, input)) = &output.tool {
            message["tool_calls"] = json!([{ "id": self.id("call"), "type": "function",
                "function": { "name": name, "arguments": input.to_string() } }]);
        }
        json!({
            "id": self.id("chatcmpl"),
            "object": "chat.completion",
            "model": self.model,
            "choices": [{ "index": 0, "message": message,
                          "finish_reason": if output.tool.is_some() { "tool_calls" } else { "stop" } }],
            "usage": { "prompt_tokens": self.input_tokens, "completion_tokens": output.output_tokens(),
                       "total_tokens": self.input_tokens + output.output_tokens() },
        })
    }

    fn chat_stream(&self, output: &Output, behaviour: Behaviour) -> Reply {
        let pace = Self::pace(behaviour);
        let chunk = |delta: Value, finish: Value| {
            Self::event(
                None,
                &json!({ "id": self.id("chatcmpl"), "object": "chat.completion.chunk", "model": self.model,
                         "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] }),
            )
        };
        let mut events = vec![(
            Duration::ZERO,
            chunk(json!({ "role": "assistant", "content": "" }), Value::Null),
        )];
        for text in &output.text {
            events.push((pace, chunk(json!({ "content": text }), Value::Null)));
        }
        if behaviour == Behaviour::Cut {
            return Reply::sse(events);
        }
        if let Some((name, input)) = &output.tool {
            events.push((
                pace,
                chunk(
                    json!({ "tool_calls": [{ "index": 0, "id": self.id("call"), "type": "function",
                        "function": { "name": name, "arguments": input.to_string() } }] }),
                    Value::Null,
                ),
            ));
        }
        events.push((
            Duration::ZERO,
            chunk(
                json!({}),
                json!(if output.tool.is_some() {
                    "tool_calls"
                } else {
                    "stop"
                }),
            ),
        ));
        events.push((
            Duration::ZERO,
            Self::event(
                None,
                &json!({ "id": self.id("chatcmpl"), "object": "chat.completion.chunk", "model": self.model,
                         "choices": [], "usage": { "prompt_tokens": self.input_tokens,
                         "completion_tokens": output.output_tokens(),
                         "total_tokens": self.input_tokens + output.output_tokens() } }),
            ),
        ));
        events.push((Duration::ZERO, "data: [DONE]\n\n".to_owned()));
        Reply::sse(events)
    }
}

/// A Codex `response.create` on a WebSocket, answered with Responses events,
/// one message each.
pub struct SocketTurn;

impl SocketTurn {
    /// The events answering one `response.create` message, or `None` for any
    /// other message (an interrupt, or something the fake ignores).
    pub fn answer(message: &str, serial: u64) -> Option<Vec<(Duration, String)>> {
        let body: Value = serde_json::from_str(message).ok()?;
        if body["type"] != "response.create" {
            return None;
        }
        let request = body.get("response").unwrap_or(&body);
        let model = request["model"].as_str().unwrap_or("fake-model").to_owned();
        let turn = Turn {
            input_tokens: u64::try_from(message.len() / 4).unwrap_or(u64::MAX),
            prompt: Prompt::newest(Wire::Responses, request),
            serial,
            model,
        };
        let behaviour = match Behaviour::for_model(&turn.model) {
            behaviour @ (Behaviour::Text
            | Behaviour::Tools
            | Behaviour::Long
            | Behaviour::Slow
            | Behaviour::Cut) => behaviour,
            // A socket has no status: an error is an `error` event.
            Behaviour::Error | Behaviour::RateLimited => {
                return Some(vec![(
                    Duration::ZERO,
                    json!({ "type": "error", "error": { "type": "server_error", "message": "fake upstream error" } })
                        .to_string(),
                )]);
            }
        };
        let output = Output::of(behaviour, &turn);
        Some(
            turn.responses_events(&output, behaviour)
                .into_iter()
                .map(|(after, event)| (after, event.to_string()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Behaviour, Reply, Request, SocketTurn, Wire};
    use serde_json::{Value, json};

    fn reply(wire: Wire, body: &Value) -> Reply {
        Reply::to(&Request {
            wire,
            body: body.to_string().as_bytes(),
            serial: 7,
        })
    }

    fn events(reply: &Reply) -> Vec<Value> {
        reply
            .pieces
            .iter()
            .flat_map(|piece| piece.bytes.lines())
            .filter_map(|line| line.strip_prefix("data: "))
            .filter(|data| *data != "[DONE]")
            .map(|data| serde_json::from_str(data).unwrap())
            .collect()
    }

    #[test]
    fn paths_and_models_pick_the_wire_and_the_behaviour() {
        assert_eq!(
            Wire::for_path("/anthropic/v1/messages"),
            Some(Wire::Messages)
        );
        assert_eq!(
            Wire::for_path("/anthropic/v1/messages/count_tokens"),
            Some(Wire::CountTokens)
        );
        assert_eq!(Wire::for_path("/codex/responses"), Some(Wire::Responses));
        assert_eq!(
            Wire::for_path("/openai/v1/responses/compact"),
            Some(Wire::Responses)
        );
        assert_eq!(
            Wire::for_path("/openai/v1/chat/completions"),
            Some(Wire::ChatCompletions)
        );
        assert_eq!(Wire::for_path("/codex/models"), Some(Wire::Models));
        assert_eq!(Wire::for_path("/elsewhere"), None);
        assert_eq!(Behaviour::for_model("fake-tools"), Behaviour::Tools);
        assert_eq!(Behaviour::for_model("claude-opus-5-5"), Behaviour::Text);
    }

    #[test]
    fn a_messages_stream_quotes_the_prompt_and_reports_usage() {
        let body = json!({ "model": "claude-fixture", "stream": true,
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hello there" }] }] });
        let reply = reply(Wire::Messages, &body);
        assert_eq!(
            (reply.status, reply.content_type),
            (200, "text/event-stream")
        );
        let events = events(&reply);
        let kinds: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(kinds.first(), Some(&"message_start"));
        assert_eq!(kinds.last(), Some(&"message_stop"));
        let text: String = events
            .iter()
            .filter_map(|e| e["delta"]["text"].as_str())
            .collect();
        assert_eq!(text, "Fake reply to: hello there");
        assert_eq!(events[0]["message"]["model"], "claude-fixture");
    }

    #[test]
    fn tools_errors_limits_and_cuts_are_scripted_by_model() {
        let ask = |model: &str, wire: Wire| {
            reply(
                wire,
                &json!({ "model": model, "stream": true, "messages": [], "input": "x" }),
            )
        };
        let tools = events(&ask("fake-tools", Wire::Messages));
        assert!(
            tools
                .iter()
                .any(|e| e["content_block"]["type"] == "tool_use")
        );
        let tools = events(&ask("fake-tools", Wire::Responses));
        assert!(tools.iter().any(|e| e["item"]["type"] == "function_call"));
        assert_eq!(ask("fake-error", Wire::Messages).status, 500);
        let limited = ask("fake-429", Wire::ChatCompletions);
        assert_eq!(limited.status, 429);
        assert!(limited.headers.contains(&("retry-after", "5".to_owned())));
        let cut = events(&ask("fake-cut", Wire::Responses));
        assert!(cut.iter().all(|e| e["type"] != "response.completed"));
        let cut = events(&ask("fake-cut", Wire::Messages));
        assert!(cut.iter().all(|e| e["type"] != "message_stop"));
    }

    #[test]
    fn chat_completions_and_json_replies_have_their_wire_shape() {
        let chat = reply(
            Wire::ChatCompletions,
            &json!({ "model": "m", "stream": true, "messages": [{ "role": "user", "content": "hi" }] }),
        );
        let last = chat.pieces.last().unwrap();
        assert_eq!(last.bytes, "data: [DONE]\n\n");
        let events = events(&chat);
        assert!(
            events
                .iter()
                .any(|e| e["usage"]["prompt_tokens"].is_number())
        );
        let message = reply(
            Wire::Messages,
            &json!({ "model": "m", "messages": [{ "role": "user", "content": "hi" }] }),
        );
        let message: Value = serde_json::from_str(&message.pieces[0].bytes).unwrap();
        assert_eq!(message["content"][0]["text"], "Fake reply to: hi");
        let models = reply(Wire::Models, &Value::Null);
        assert!(models.pieces[0].bytes.contains("fake-tools"));
    }

    #[test]
    fn a_socket_answers_response_create_and_nothing_else() {
        let create = json!({ "type": "response.create", "model": "gpt-fixture", "input": "hi" });
        let events = SocketTurn::answer(&create.to_string(), 1).unwrap();
        let last: Value = serde_json::from_str(&events.last().unwrap().1).unwrap();
        assert_eq!(last["type"], "response.completed");
        assert_eq!(last["response"]["model"], "gpt-fixture");
        assert!(SocketTurn::answer(r#"{"type":"response.cancel"}"#, 2).is_none());
        let failed = json!({ "type": "response.create", "model": "fake-error" });
        let events = SocketTurn::answer(&failed.to_string(), 3).unwrap();
        assert!(events[0].1.contains("\"error\""));
    }
}
