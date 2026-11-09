//! What the fake provider answers, as pure data: a request's wire format and
//! body in, a scripted reply out. No I/O; the server around it only writes
//! the reply's pieces, waiting between them as told.
//!
//! The dev traffic chooses what happens with a request header the gateway
//! forwards but never records (`x-dev-reply`); without it, a request gets an
//! ordinary text answer. What the answers say comes from [`Writing`]:
//! invented, realistic engineering replies.

use super::writing::Writing;
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

/// What the dev traffic asks a reply to do.
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
    /// A short conversation title, as a client's side call asks for.
    Title,
}

impl Behaviour {
    /// The request header that chooses a reply.
    pub const HEADER: &'static str = "x-dev-reply";

    /// The behaviour a header value names (`text`, `tools`, `long`, `slow`,
    /// `cut`, `title`, `error`, `rate_limited`); anything else, or none, is
    /// text.
    pub fn for_label(label: Option<&str>) -> Self {
        match label {
            Some("tools") => Self::Tools,
            Some("long") => Self::Long,
            Some("error") => Self::Error,
            Some("rate_limited") => Self::RateLimited,
            Some("slow") => Self::Slow,
            Some("cut") => Self::Cut,
            Some("title") => Self::Title,
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
    /// The wait before each streamed piece (the dev slot's pace).
    pub gap: Duration,
    pub behaviour: Behaviour,
}

impl Reply {
    const SSE: &'static str = "text/event-stream";
    const JSON: &'static str = "application/json";

    pub fn to(request: &Request<'_>) -> Self {
        let body: Value = serde_json::from_slice(request.body).unwrap_or(Value::Null);
        let model = body["model"]
            .as_str()
            .unwrap_or(Turn::DEFAULT_MODEL)
            .to_owned();
        let stream = body["stream"].as_bool().unwrap_or(false);
        // A rough, deterministic count: four bytes a token.
        let input_tokens = u64::try_from(request.body.len() / 4).unwrap_or(u64::MAX);
        let turn = Turn {
            model,
            gap: request.gap,
            serial: request.serial,
            input_tokens,
            prompt: Prompt::newest(request.wire, &body),
            asks: Asks::read(request.wire, &body),
        };
        match request.wire {
            Wire::Models => Self::json(200, &turn.models()),
            Wire::CountTokens => Self::json(200, &json!({ "input_tokens": input_tokens })),
            Wire::Messages | Wire::Responses | Wire::ChatCompletions => {
                match request.behaviour {
                    Behaviour::Error => Self::error(request.wire, 500),
                    Behaviour::RateLimited => {
                        let mut reply = Self::error(request.wire, 429);
                        reply.headers.push(("retry-after", "5".to_owned()));
                        reply
                    }
                    behaviour @ (Behaviour::Text
                    | Behaviour::Tools
                    | Behaviour::Long
                    | Behaviour::Slow
                    | Behaviour::Cut
                    | Behaviour::Title) => {
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

    /// An error as each provider words it.
    fn error(wire: Wire, status: u16) -> Self {
        let limited = status == 429;
        let body = match wire {
            Wire::Messages | Wire::CountTokens | Wire::Models => json!({
                "type": "error",
                "error": if limited {
                    json!({ "type": "rate_limit_error", "message": "This request would exceed the rate limit for your organization. Please try again later." })
                } else {
                    json!({ "type": "api_error", "message": "Internal server error" })
                },
            }),
            Wire::Responses | Wire::ChatCompletions => json!({
                "error": if limited {
                    json!({ "type": "requests", "code": "rate_limit_exceeded", "message": "Rate limit reached for requests. Please try again in 5s." })
                } else {
                    json!({ "type": "server_error", "code": null, "message": "The server had an error while processing your request. Sorry about that!" })
                },
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

/// What a request asks of the model beyond its prompt, read the way the
/// provider reads it.
#[derive(Debug, Clone, Copy, Default)]
struct Asks {
    /// Prompt caching: `cache_control` marks (Messages) or a
    /// `prompt_cache_key` (Responses).
    cache: bool,
    /// Thinking (Messages) or reasoning (Responses) before the answer.
    reasoning: bool,
    /// The most output tokens the reply may have (Messages' `max_tokens`).
    max_tokens: Option<u64>,
}

impl Asks {
    fn read(wire: Wire, body: &Value) -> Self {
        match wire {
            Wire::Messages => Self {
                cache: Self::marks_cache(body),
                reasoning: matches!(
                    body["thinking"]["type"].as_str(),
                    Some("enabled" | "adaptive")
                ),
                max_tokens: body["max_tokens"].as_u64(),
            },
            Wire::Responses => Self {
                cache: body["prompt_cache_key"].is_string(),
                reasoning: body["reasoning"].is_object(),
                max_tokens: None,
            },
            Wire::ChatCompletions | Wire::CountTokens | Wire::Models => Self::default(),
        }
    }

    /// Whether any block in the request carries a `cache_control` mark.
    fn marks_cache(value: &Value) -> bool {
        match value {
            Value::Object(fields) => {
                fields.contains_key("cache_control") || fields.values().any(Self::marks_cache)
            }
            Value::Array(items) => items.iter().any(Self::marks_cache),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
        }
    }
}

/// Why a reply ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// The model finished its answer.
    End,
    /// The model called a tool.
    Tool,
    /// The reply reached the request's `max_tokens`, this many.
    MaxTokens(u64),
}

/// What the reply says: thinking, text, a tool call, or some of these.
struct Output {
    thinking: Vec<String>,
    text: Vec<String>,
    tool: Option<(&'static str, Value)>,
    stop: Stop,
}

impl Output {
    fn of(behaviour: Behaviour, turn: &Turn) -> Self {
        let words =
            |text: &str| -> Vec<String> { text.split_inclusive(' ').map(str::to_owned).collect() };
        let (text, tool) = match behaviour {
            Behaviour::Tools => {
                let (lead, name, input) = Writing::tool(turn.serial);
                (words(lead), Some((name, input)))
            }
            // A long answer streams a sentence at a time.
            Behaviour::Long => (
                Writing::long_answer(&turn.prompt, turn.serial)
                    .split_inclusive(". ")
                    .map(str::to_owned)
                    .collect(),
                None,
            ),
            Behaviour::Title => (words(Writing::title(&turn.prompt)), None),
            // Word by word: at the normal pace a short answer streams for
            // about a second, as a real model's does.
            Behaviour::Text
            | Behaviour::Slow
            | Behaviour::Cut
            | Behaviour::Error
            | Behaviour::RateLimited => (words(Writing::answer(&turn.prompt, turn.serial)), None),
        };
        let thinking = if turn.asks.reasoning && behaviour != Behaviour::Title {
            words(Writing::thinking(turn.serial))
        } else {
            Vec::new()
        };
        let stop = if tool.is_some() {
            Stop::Tool
        } else {
            Stop::End
        };
        let output = Self {
            thinking,
            text,
            tool,
            stop,
        };
        match turn.asks.max_tokens {
            Some(max) if output.output_tokens() > max => output.cut_to(max),
            Some(_) | None => output,
        }
    }

    /// The reply cut where it reaches `max` tokens (at least one word), as a
    /// model stops at `max_tokens`.
    fn cut_to(self, max: u64) -> Self {
        let budget = usize::try_from(max.saturating_mul(4)).unwrap_or(usize::MAX);
        let mut used = 0;
        let mut text = Vec::new();
        for word in self.text {
            used += word.len();
            if !text.is_empty() && used > budget {
                break;
            }
            text.push(word);
        }
        Self {
            thinking: Vec::new(),
            text,
            tool: None,
            stop: Stop::MaxTokens(max),
        }
    }

    fn joined(&self) -> String {
        self.text.concat()
    }

    fn thought(&self) -> String {
        self.thinking.concat()
    }

    fn tokens(text: &str) -> u64 {
        u64::try_from(text.len() / 4 + 1).unwrap_or(u64::MAX)
    }

    /// Output tokens, thinking included, as providers count them.
    fn output_tokens(&self) -> u64 {
        match self.stop {
            // A reply cut at `max_tokens` used exactly that many.
            Stop::MaxTokens(max) => Self::tokens(&self.joined()).min(max),
            Stop::End | Stop::Tool => {
                let thinking = if self.thinking.is_empty() {
                    0
                } else {
                    Self::tokens(&self.thought())
                };
                Self::tokens(&self.joined()) + thinking
            }
        }
    }

    fn reasoning_tokens(&self) -> u64 {
        if self.thinking.is_empty() {
            0
        } else {
            Self::tokens(&self.thought())
        }
    }
}

/// Prompt-cache usage: tokens read from the cache and written to it.
#[derive(Debug, Clone, Copy, Default)]
struct Cache {
    read: u64,
    written: u64,
}

/// One fake model turn.
struct Turn {
    model: String,
    /// The wait before each streamed piece, but for a slow reply's.
    gap: Duration,
    serial: u64,
    input_tokens: u64,
    prompt: String,
    asks: Asks,
}

impl Turn {
    /// The model a request that names none gets.
    pub const DEFAULT_MODEL: &'static str = "claude-sonnet-5-5";

    /// An identifier in a provider's style: `prefix`, then letters and
    /// digits that differ for each request.
    fn id(&self, prefix: &str) -> String {
        const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        let mut state = self.serial ^ u64::try_from(prefix.len()).unwrap_or(0);
        let mut id = String::from(prefix);
        for _ in 0..24 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let index = usize::try_from(state >> 58).unwrap_or(0);
            id.push(char::from(ALPHABET[index % ALPHABET.len()]));
        }
        id
    }

    /// Prompt-cache usage: when the request asks for caching, most of its
    /// context comes from the cache and a little is written to it, in amounts
    /// that differ per request as a session's context does.
    fn cache(&self) -> Cache {
        if !self.asks.cache {
            return Cache::default();
        }
        let mut mixed = self.serial.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^= mixed >> 31;
        Cache {
            read: 18_000 + mixed % 42_000,
            written: 200 + (mixed >> 32) % 2_800,
        }
    }

    /// A thinking block's signature, in the provider's style.
    fn signature(&self) -> String {
        format!("{}{}", self.id("EqQBCkgIBhABGAIiQ"), self.id("Ewz"))
    }

    fn stop_reason(output: &Output) -> &'static str {
        match output.stop {
            Stop::End => "end_turn",
            Stop::Tool => "tool_use",
            Stop::MaxTokens(_) => "max_tokens",
        }
    }

    fn messages_usage(&self, output_tokens: u64) -> Value {
        let cache = self.cache();
        json!({ "input_tokens": self.input_tokens,
                "cache_creation_input_tokens": cache.written,
                "cache_read_input_tokens": cache.read,
                "output_tokens": output_tokens })
    }

    fn pace(&self, behaviour: Behaviour) -> Duration {
        match behaviour {
            Behaviour::Slow => Duration::from_millis(500),
            Behaviour::Text
            | Behaviour::Title
            | Behaviour::Tools
            | Behaviour::Long
            | Behaviour::Error
            | Behaviour::RateLimited
            | Behaviour::Cut => self.gap,
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
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-haiku-5-5",
            "gpt-5.5",
            "gpt-5.5-codex",
            "deepseek-chat",
        ]
        .iter()
        .map(|id| json!({ "id": id, "type": "model", "object": "model", "display_name": id }))
        .collect();
        json!({ "object": "list", "data": data })
    }

    fn message(&self, output: &Output) -> Value {
        let mut content = Vec::new();
        if !output.thinking.is_empty() {
            content.push(json!({ "type": "thinking", "thinking": output.thought(), "signature": self.signature() }));
        }
        content.push(json!({ "type": "text", "text": output.joined() }));
        if let Some((name, input)) = &output.tool {
            content.push(
                json!({ "type": "tool_use", "id": self.id("toolu_01"), "name": name, "input": input }),
            );
        }
        json!({
            "id": self.id("msg_01"),
            "type": "message",
            "role": "assistant",
            "model": self.model,
            "content": content,
            "stop_reason": Self::stop_reason(output),
            "usage": self.messages_usage(output.output_tokens()),
        })
    }

    fn messages_stream(&self, output: &Output, behaviour: Behaviour) -> Reply {
        let pace = self.pace(behaviour);
        let event = |name: &str, data: Value| Self::event(Some(name), &data);
        let mut events = vec![
            (
                Duration::ZERO,
                event(
                    "message_start",
                    json!({ "type": "message_start", "message": {
                        "id": self.id("msg_01"), "type": "message", "role": "assistant", "model": self.model,
                        "content": [], "stop_reason": null, "stop_sequence": null,
                        "usage": self.messages_usage(1),
                    }}),
                ),
            ),
            (Duration::ZERO, event("ping", json!({ "type": "ping" }))),
        ];
        let mut index = 0;
        if !output.thinking.is_empty() {
            events.push((
                Duration::ZERO,
                event(
                    "content_block_start",
                    json!({ "type": "content_block_start", "index": index,
                            "content_block": { "type": "thinking", "thinking": "", "signature": "" } }),
                ),
            ));
            for text in &output.thinking {
                events.push((
                    pace,
                    event(
                        "content_block_delta",
                        json!({ "type": "content_block_delta", "index": index,
                                "delta": { "type": "thinking_delta", "thinking": text } }),
                    ),
                ));
            }
            events.push((
                Duration::ZERO,
                event(
                    "content_block_delta",
                    json!({ "type": "content_block_delta", "index": index,
                            "delta": { "type": "signature_delta", "signature": self.signature() } }),
                ),
            ));
            events.push((
                Duration::ZERO,
                event(
                    "content_block_stop",
                    json!({ "type": "content_block_stop", "index": index }),
                ),
            ));
            index += 1;
        }
        events.push((
            Duration::ZERO,
            event(
                "content_block_start",
                json!({ "type": "content_block_start", "index": index, "content_block": { "type": "text", "text": "" } }),
            ),
        ));
        for text in &output.text {
            events.push((
                pace,
                event(
                    "content_block_delta",
                    json!({ "type": "content_block_delta", "index": index, "delta": { "type": "text_delta", "text": text } }),
                ),
            ));
        }
        events.push((
            Duration::ZERO,
            event(
                "content_block_stop",
                json!({ "type": "content_block_stop", "index": index }),
            ),
        ));
        if behaviour == Behaviour::Cut {
            return Reply::sse(events);
        }
        if let Some((name, input)) = &output.tool {
            index += 1;
            events.push((
                pace,
                event(
                    "content_block_start",
                    json!({ "type": "content_block_start", "index": index, "content_block": {
                        "type": "tool_use", "id": self.id("toolu_01"), "name": name, "input": {},
                    }}),
                ),
            ));
            events.push((
                pace,
                event(
                    "content_block_delta",
                    json!({ "type": "content_block_delta", "index": index, "delta": {
                        "type": "input_json_delta", "partial_json": input.to_string(),
                    }}),
                ),
            ));
            events.push((
                Duration::ZERO,
                event(
                    "content_block_stop",
                    json!({ "type": "content_block_stop", "index": index }),
                ),
            ));
        }
        events.push((
            Duration::ZERO,
            event(
                "message_delta",
                json!({ "type": "message_delta",
                    "delta": { "stop_reason": Self::stop_reason(output), "stop_sequence": null },
                    "usage": { "output_tokens": output.output_tokens() } }),
            ),
        ));
        events.push((
            Duration::ZERO,
            event("message_stop", json!({ "type": "message_stop" })),
        ));
        Reply::sse(events)
    }

    fn reasoning_item(&self, output: &Output) -> Value {
        json!({ "type": "reasoning", "id": self.id("rs_"),
                "summary": [{ "type": "summary_text", "text": output.thought() }],
                "encrypted_content": format!("gAAAAAB{}", self.id("p")) })
    }

    fn response_object(&self, output: &Output, status: &str) -> Value {
        let mut items = Vec::new();
        if !output.thinking.is_empty() {
            items.push(self.reasoning_item(output));
        }
        items.push(json!({
            "type": "message", "id": self.id("msg_01"), "role": "assistant", "status": "completed",
            "content": [{ "type": "output_text", "text": output.joined(), "annotations": [] }],
        }));
        if let Some((name, input)) = &output.tool {
            items.push(json!({
                "type": "function_call", "id": self.id("fc_"), "call_id": self.id("call_"),
                "name": name, "arguments": input.to_string(), "status": "completed",
            }));
        }
        json!({
            "id": self.id("resp_"),
            "object": "response",
            "model": self.model,
            "status": status,
            "output": if status == "completed" { Value::Array(items) } else { json!([]) },
            "usage": if status == "completed" {
                // Responses counts cached tokens inside `input_tokens`.
                let cached = self.cache().read;
                let input = self.input_tokens + cached;
                json!({ "input_tokens": input, "input_tokens_details": { "cached_tokens": cached },
                        "output_tokens": output.output_tokens(),
                        "output_tokens_details": { "reasoning_tokens": output.reasoning_tokens() },
                        "total_tokens": input + output.output_tokens() })
            } else { Value::Null },
        })
    }

    /// The Responses events of one turn, as Codex reads them over SSE or a
    /// WebSocket.
    fn responses_events(&self, output: &Output, behaviour: Behaviour) -> Vec<(Duration, Value)> {
        let pace = self.pace(behaviour);
        let mut events = vec![(
            Duration::ZERO,
            json!({ "type": "response.created", "response": self.response_object(output, "in_progress") }),
        )];
        let mut index = 0;
        if !output.thinking.is_empty() {
            let id = self.id("rs_");
            events.push((
                Duration::ZERO,
                json!({ "type": "response.output_item.added", "output_index": index,
                        "item": { "type": "reasoning", "id": id, "summary": [] } }),
            ));
            events.push((
                Duration::ZERO,
                json!({ "type": "response.reasoning_summary_part.added", "item_id": id, "output_index": index,
                        "summary_index": 0, "part": { "type": "summary_text", "text": "" } }),
            ));
            for text in &output.thinking {
                events.push((
                    pace,
                    json!({ "type": "response.reasoning_summary_text.delta", "item_id": id,
                            "output_index": index, "summary_index": 0, "delta": text }),
                ));
            }
            events.push((
                Duration::ZERO,
                json!({ "type": "response.output_item.done", "output_index": index,
                        "item": self.reasoning_item(output) }),
            ));
            index += 1;
        }
        events.push((
            Duration::ZERO,
            json!({ "type": "response.output_item.added", "output_index": index, "item": {
                "type": "message", "id": self.id("msg_01"), "role": "assistant", "status": "in_progress", "content": [],
            }}),
        ));
        for text in &output.text {
            events.push((
                pace,
                json!({ "type": "response.output_text.delta", "item_id": self.id("msg_01"),
                        "output_index": index, "content_index": 0, "delta": text }),
            ));
        }
        if behaviour == Behaviour::Cut {
            return events;
        }
        events.push((
            Duration::ZERO,
            json!({ "type": "response.output_item.done", "output_index": index, "item": {
                "type": "message", "id": self.id("msg_01"), "role": "assistant", "status": "completed",
                "content": [{ "type": "output_text", "text": output.joined(), "annotations": [] }],
            }}),
        ));
        if let Some((name, input)) = &output.tool {
            events.push((
                pace,
                json!({ "type": "response.output_item.done", "output_index": index + 1, "item": {
                    "type": "function_call", "id": self.id("fc_"), "call_id": self.id("call_"),
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

    fn finish_reason(output: &Output) -> &'static str {
        match output.stop {
            Stop::End => "stop",
            Stop::Tool => "tool_calls",
            Stop::MaxTokens(_) => "length",
        }
    }

    fn chat_completion(&self, output: &Output) -> Value {
        let mut message = json!({ "role": "assistant", "content": output.joined() });
        if let Some((name, input)) = &output.tool {
            message["tool_calls"] = json!([{ "id": self.id("call_"), "type": "function",
                "function": { "name": name, "arguments": input.to_string() } }]);
        }
        json!({
            "id": self.id("chatcmpl-"),
            "object": "chat.completion",
            "model": self.model,
            "choices": [{ "index": 0, "message": message,
                          "finish_reason": Self::finish_reason(output) }],
            "usage": { "prompt_tokens": self.input_tokens, "completion_tokens": output.output_tokens(),
                       "total_tokens": self.input_tokens + output.output_tokens() },
        })
    }

    fn chat_stream(&self, output: &Output, behaviour: Behaviour) -> Reply {
        let pace = self.pace(behaviour);
        let chunk = |delta: Value, finish: Value| {
            Self::event(
                None,
                &json!({ "id": self.id("chatcmpl-"), "object": "chat.completion.chunk", "model": self.model,
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
                    json!({ "tool_calls": [{ "index": 0, "id": self.id("call_"), "type": "function",
                        "function": { "name": name, "arguments": input.to_string() } }] }),
                    Value::Null,
                ),
            ));
        }
        events.push((
            Duration::ZERO,
            chunk(json!({}), json!(Self::finish_reason(output))),
        ));
        events.push((
            Duration::ZERO,
            Self::event(
                None,
                &json!({ "id": self.id("chatcmpl-"), "object": "chat.completion.chunk", "model": self.model,
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
    /// other message (an interrupt, or something this provider ignores).
    /// `behaviour` comes from the socket's handshake.
    pub fn answer(
        message: &str,
        serial: u64,
        gap: Duration,
        behaviour: Behaviour,
    ) -> Option<Vec<(Duration, String)>> {
        let body: Value = serde_json::from_str(message).ok()?;
        if body["type"] != "response.create" {
            return None;
        }
        let request = body.get("response").unwrap_or(&body);
        let model = request["model"]
            .as_str()
            .unwrap_or(Turn::DEFAULT_MODEL)
            .to_owned();
        let turn = Turn {
            gap,
            input_tokens: u64::try_from(message.len() / 4).unwrap_or(u64::MAX),
            prompt: Prompt::newest(Wire::Responses, request),
            asks: Asks::read(Wire::Responses, request),
            serial,
            model,
        };
        let behaviour = match behaviour {
            behaviour @ (Behaviour::Text
            | Behaviour::Tools
            | Behaviour::Long
            | Behaviour::Slow
            | Behaviour::Cut
            | Behaviour::Title) => behaviour,
            // A socket has no status: an error is an `error` event.
            Behaviour::Error | Behaviour::RateLimited => {
                return Some(vec![(
                    Duration::ZERO,
                    json!({ "type": "error", "error": { "type": "server_error", "message": "The server had an error while processing your request. Sorry about that!" } })
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
    use std::time::Duration;

    fn reply_as(wire: Wire, body: &Value, behaviour: Behaviour) -> Reply {
        Reply::to(&Request {
            wire,
            body: body.to_string().as_bytes(),
            serial: 7,
            gap: Duration::from_millis(15),
            behaviour,
        })
    }

    fn reply(wire: Wire, body: &Value) -> Reply {
        reply_as(wire, body, Behaviour::Text)
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

    fn events_of(reply: &Reply) -> Vec<Value> {
        events(reply)
    }

    fn all_text(reply: &Reply) -> String {
        reply
            .pieces
            .iter()
            .map(|piece| piece.bytes.as_str())
            .collect()
    }

    #[test]
    fn paths_and_labels_pick_the_wire_and_the_behaviour() {
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
        assert_eq!(Behaviour::for_label(Some("tools")), Behaviour::Tools);
        assert_eq!(
            Behaviour::for_label(Some("rate_limited")),
            Behaviour::RateLimited
        );
        assert_eq!(Behaviour::for_label(None), Behaviour::Text);
    }

    #[test]
    fn a_messages_stream_answers_the_prompt_and_reports_usage() {
        let body = json!({ "model": "claude-opus-5-5", "stream": true,
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "Make the order list newest first; where is it sorted?" }] }] });
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
        assert!(text.contains("ORDER BY created_at DESC"), "{text}");
        assert_eq!(events[0]["message"]["model"], "claude-opus-5-5");
        let id = events[0]["message"]["id"].as_str().unwrap();
        assert!(id.starts_with("msg_01") && id.len() == 30, "{id}");
    }

    #[test]
    fn caching_and_thinking_are_answered_when_the_request_asks() {
        let body = json!({ "model": "claude-opus-5-5", "stream": true, "max_tokens": 32000,
            "thinking": { "type": "enabled", "budget_tokens": 31999 },
            "system": [{ "type": "text", "text": "You are a coding agent.", "cache_control": { "type": "ephemeral" } }],
            "messages": [{ "role": "user", "content": "Where is the order list sorted?" }] });
        let events = events(&reply(Wire::Messages, &body));
        let usage = &events[0]["message"]["usage"];
        assert!(usage["cache_read_input_tokens"].as_u64().unwrap() >= 18_000);
        assert!(usage["cache_creation_input_tokens"].as_u64().unwrap() >= 200);
        assert_eq!(events[1]["type"], "ping");
        assert_eq!(events[2]["content_block"]["type"], "thinking");
        assert!(
            events
                .iter()
                .any(|e| e["delta"]["type"] == "thinking_delta")
        );
        assert!(
            events
                .iter()
                .any(|e| e["delta"]["type"] == "signature_delta")
        );
        let text_start = events
            .iter()
            .find(|e| e["content_block"]["type"] == "text")
            .unwrap();
        assert_eq!(text_start["index"], 1);

        let plain = json!({ "model": "claude-haiku-5-5", "stream": true,
            "messages": [{ "role": "user", "content": "hi" }] });
        let events = events_of(&reply(Wire::Messages, &plain));
        assert_eq!(events[0]["message"]["usage"]["cache_read_input_tokens"], 0);
        assert!(
            events
                .iter()
                .all(|e| e["content_block"]["type"] != "thinking")
        );

        let body = json!({ "model": "gpt-5.5-codex", "stream": true, "prompt_cache_key": "k",
            "reasoning": { "effort": "medium", "summary": "auto" }, "input": "Where is it sorted?" });
        let events = events_of(&reply(Wire::Responses, &body));
        assert_eq!(events[1]["item"]["type"], "reasoning");
        let done = events.last().unwrap();
        let usage = &done["response"]["usage"];
        assert!(
            usage["input_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap()
                >= 18_000
        );
        assert!(
            usage["output_tokens_details"]["reasoning_tokens"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(done["response"]["output"][0]["type"], "reasoning");
        assert_eq!(done["response"]["output"][1]["type"], "message");
    }

    #[test]
    fn side_calls_get_a_title_or_stop_at_max_tokens() {
        let probe = json!({ "model": "claude-haiku-5-5", "max_tokens": 1,
            "messages": [{ "role": "user", "content": "quota" }] });
        let message: Value =
            serde_json::from_str(&reply(Wire::Messages, &probe).pieces[0].bytes).unwrap();
        assert_eq!(message["stop_reason"], "max_tokens");
        assert_eq!(message["usage"]["output_tokens"], 1);
        assert!(
            !message["content"][0]["text"]
                .as_str()
                .unwrap()
                .trim()
                .contains(" ")
        );

        let title = json!({ "model": "claude-haiku-5-5", "max_tokens": 512, "stream": true,
            "messages": [{ "role": "user", "content": "Find where the order list is sorted and make it newest first." }] });
        let text: String = events_of(&reply_as(Wire::Messages, &title, Behaviour::Title))
            .iter()
            .filter_map(|e| e["delta"]["text"].as_str())
            .collect();
        assert_eq!(text, "Sort order list newest first");
    }

    #[test]
    fn tools_errors_limits_and_cuts_are_chosen_by_the_header() {
        let ask = |behaviour, wire| {
            reply_as(
                wire,
                &json!({ "model": "claude-opus-5-5", "stream": true, "messages": [], "input": "x" }),
                behaviour,
            )
        };
        let tools = events(&ask(Behaviour::Tools, Wire::Messages));
        assert!(
            tools
                .iter()
                .any(|e| e["content_block"]["type"] == "tool_use")
        );
        let tools = events(&ask(Behaviour::Tools, Wire::Responses));
        assert!(tools.iter().any(|e| e["item"]["type"] == "function_call"));
        let error = ask(Behaviour::Error, Wire::Messages);
        assert_eq!(error.status, 500);
        assert!(all_text(&error).contains("Internal server error"));
        let limited = ask(Behaviour::RateLimited, Wire::ChatCompletions);
        assert_eq!(limited.status, 429);
        assert!(limited.headers.contains(&("retry-after", "5".to_owned())));
        let cut = events(&ask(Behaviour::Cut, Wire::Responses));
        assert!(cut.iter().all(|e| e["type"] != "response.completed"));
        let cut = events(&ask(Behaviour::Cut, Wire::Messages));
        assert!(cut.iter().all(|e| e["type"] != "message_stop"));
    }

    #[test]
    fn nothing_a_reply_shows_says_it_is_fake() {
        for behaviour in [
            Behaviour::Text,
            Behaviour::Tools,
            Behaviour::Long,
            Behaviour::Slow,
            Behaviour::Cut,
            Behaviour::Title,
            Behaviour::Error,
            Behaviour::RateLimited,
        ] {
            for wire in [
                Wire::Messages,
                Wire::Responses,
                Wire::ChatCompletions,
                Wire::Models,
            ] {
                let body = json!({ "model": "gpt-5.5", "stream": true,
                    "messages": [{ "role": "user", "content": "hi" }], "input": "hi" });
                let text = all_text(&reply_as(wire, &body, behaviour)).to_lowercase();
                assert!(!text.contains("fake"), "{behaviour:?} {wire:?}: {text}");
            }
        }
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
        assert!(message["content"][0]["text"].as_str().unwrap().len() > 100);
        let models = reply(Wire::Models, &Value::Null);
        assert!(models.pieces[0].bytes.contains("claude-opus-5-5"));
    }

    #[test]
    fn a_socket_answers_response_create_and_nothing_else() {
        let create = json!({ "type": "response.create", "model": "gpt-5.5-codex", "input": "hi" });
        let events =
            SocketTurn::answer(&create.to_string(), 1, Duration::ZERO, Behaviour::Text).unwrap();
        let last: Value = serde_json::from_str(&events.last().unwrap().1).unwrap();
        assert_eq!(last["type"], "response.completed");
        assert_eq!(last["response"]["model"], "gpt-5.5-codex");
        assert!(
            SocketTurn::answer(
                r#"{"type":"response.cancel"}"#,
                2,
                Duration::ZERO,
                Behaviour::Text
            )
            .is_none()
        );
        let events =
            SocketTurn::answer(&create.to_string(), 3, Duration::ZERO, Behaviour::Error).unwrap();
        assert!(events[0].1.contains("\"error\""));
    }
}
