//! What a call's captured bodies show an agent doing: the user text and tool
//! results its request carries, and the tool calls and reply text its
//! response holds. Read as the bytes are captured: a request is gathered by
//! [`RequestReader`], and a response's frames feed a [`ResponseReader`]
//! through the inspection pipeline. Never on the forwarding path.
//!
//! These are facts about the bytes. A tool call in a response was requested,
//! not executed; a tool result in a request is what the client reported.
//! Whatever could not be read is a [`ActivityLimitation`], never a guess.

pub mod anthropic;
pub mod chat;
pub mod openai;

use crate::encoding::{Decompressor, ResponseEncoding};
use crate::inspect::pipeline::{BodyEnd, Protocol};
use crate::sse::SseLimits;
use serde_json::{Map, Value};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Why activity is partial.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ActivityLimitation {
    /// The endpoint's bodies carry no agent activity TGV reads.
    UnsupportedEndpoint,
    /// Stored bytes are missing; reading stopped there.
    CaptureGap,
    /// The body continues past what was read.
    Truncated,
    UnsupportedEncoding,
    DecodingError,
    DecodedTooLarge,
    NotJson,
    /// JSON, but not the shape the protocol documents.
    UnexpectedShape,
    /// A tool call's block was opened but never closed.
    UnfinishedToolCall,
    TooManyToolCalls,
    /// The call's activity has not been derived yet: it finished moments
    /// ago, or its digest is being derived again from the stored bodies.
    NotYetDerived,
}

/// What a client said about a tool result.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ResultStatus {
    Succeeded,
    Failed,
    /// The protocol has no error flag, and nothing else said.
    Unreported,
}

/// A tool call's input as the response carried it.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolInput {
    Json(Value),
    /// Free-form input, or arguments that did not parse as JSON.
    Text(String),
}

/// A tool call a response requested.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolUse {
    /// The identifier the client answers with.
    pub id: Option<String>,
    /// The native item or block type (`tool_use`, `function_call`, ...).
    pub kind: String,
    pub name: String,
    pub input: ToolInput,
}

/// A tool result a request carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    /// The tool call it answers.
    pub id: String,
    pub status: ResultStatus,
    pub output: String,
}

/// What one request carries: the newest user input.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RequestActivity {
    /// User text blocks after the model's last output, in order. Clients
    /// add their own context here too; [`Harness`] filters it.
    ///
    /// [`Harness`]: https://docs.rs/tgv-harness
    pub user_text: Vec<String>,
    pub tool_results: Vec<ToolResult>,
    /// The most output the request lets the model produce, in tokens.
    pub max_output_tokens: Option<u64>,
    pub limitations: Vec<ActivityLimitation>,
}

/// What one response holds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResponseActivity {
    pub tool_uses: Vec<ToolUse>,
    /// The model's text output, joined.
    pub reply: String,
    pub limitations: Vec<ActivityLimitation>,
}

impl ResponseActivity {
    /// Whether another tool call fits under `max`. Past it the response
    /// says so once, and later tool calls are left out.
    fn has_room(&mut self, max: usize) -> bool {
        if self.tool_uses.len() < max {
            return true;
        }
        if !self
            .limitations
            .contains(&ActivityLimitation::TooManyToolCalls)
        {
            self.limitations.push(ActivityLimitation::TooManyToolCalls);
        }
        false
    }

    /// The reply from the response's text parts, in order, without empty
    /// ones.
    fn set_reply(&mut self, parts: Vec<String>) {
        self.reply = parts
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
    }
}

/// How much of a request was gathered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Extent {
    Complete,
    /// The body continues past the gathered bytes.
    Truncated,
    /// Bytes are missing before the end.
    Gap,
}

/// Bounds on one read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityLimits {
    pub max_decoded_bytes: usize,
    /// The most request bytes gathered for reading: the capture limit.
    pub max_request_bytes: usize,
    pub max_tool_uses: usize,
    pub sse: SseLimits,
}

impl Default for ActivityLimits {
    fn default() -> Self {
        Self {
            max_decoded_bytes: 64 * 1024 * 1024,
            max_request_bytes: 16 * 1024 * 1024,
            max_tool_uses: 256,
            sse: SseLimits::default(),
        }
    }
}

/// A response's activity, built from one document or from its events in
/// order, as they arrive.
#[derive(Debug, Clone)]
pub struct ResponseReader {
    protocol: ProtocolResponse,
    limitations: Vec<ActivityLimitation>,
}

#[derive(Debug, Clone)]
enum ProtocolResponse {
    Messages(anthropic::Response),
    Responses(openai::Response),
    ChatCompletions(chat::Response),
}

impl ResponseReader {
    pub fn new(protocol: Protocol, limits: ActivityLimits) -> Self {
        Self {
            protocol: match protocol {
                Protocol::Messages => ProtocolResponse::Messages(anthropic::Response::new(limits)),
                Protocol::Responses => ProtocolResponse::Responses(openai::Response::new(limits)),
                Protocol::ChatCompletions => {
                    ProtocolResponse::ChatCompletions(chat::Response::new(limits))
                }
            },
            limitations: Vec::new(),
        }
    }

    /// One streamed event's data, or one WebSocket message.
    pub fn event_bytes(&mut self, data: &[u8]) {
        // An event that is not a JSON object holds nothing to read.
        if let Ok(Value::Object(event)) = serde_json::from_slice::<Value>(data) {
            match &mut self.protocol {
                ProtocolResponse::Messages(response) => response.event(&event),
                ProtocolResponse::Responses(response) => response.event(event),
                ProtocolResponse::ChatCompletions(response) => response.event(&event),
            }
        }
    }

    /// A whole response document.
    pub fn document_bytes(&mut self, bytes: &[u8]) {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(Value::Object(document)) => match &mut self.protocol {
                ProtocolResponse::Messages(response) => response.document(document),
                ProtocolResponse::Responses(response) => response.document(document),
                ProtocolResponse::ChatCompletions(response) => response.document(&document),
            },
            Ok(_) => self.limitations.push(ActivityLimitation::UnexpectedShape),
            Err(_) => self.limitations.push(ActivityLimitation::NotJson),
        }
    }

    /// What the response holds so far, without ending the reading: the
    /// reply received to now, and the tool calls begun (an input still
    /// arriving may be partial). For showing a call while it streams; the
    /// finished reading is the record.
    pub fn so_far(&self) -> ResponseActivity {
        self.clone().finish()
    }

    pub fn finish(self) -> ResponseActivity {
        let mut activity = match self.protocol {
            ProtocolResponse::Messages(response) => response.finish(),
            ProtocolResponse::Responses(response) => response.finish(),
            ProtocolResponse::ChatCompletions(response) => response.finish(),
        };
        let mut limitations = self.limitations;
        limitations.append(&mut activity.limitations);
        activity.limitations = ActivityLimitation::unique(limitations);
        activity
    }
}

/// A request body, gathered as it is captured and read once it ends. The
/// newest user input is at the end of a conversation, so the whole body is
/// needed; up to `max_request_bytes` are kept.
#[derive(Debug)]
pub struct RequestReader {
    protocol: Protocol,
    limits: ActivityLimits,
    bytes: Vec<u8>,
    extent: Extent,
    /// A WebSocket call's request is one message.
    framing: RequestFraming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestFraming {
    Http,
    Message,
}

impl RequestReader {
    pub fn new(protocol: Protocol, limits: ActivityLimits) -> Self {
        Self {
            protocol,
            limits,
            bytes: Vec::new(),
            extent: Extent::Complete,
            framing: RequestFraming::Http,
        }
    }

    /// The request is a WebSocket message: its bytes are one document.
    pub fn message(&mut self) {
        self.framing = RequestFraming::Message;
    }

    /// The next contiguous captured bytes.
    pub fn feed(&mut self, bytes: &[u8]) {
        if self.extent != Extent::Complete {
            return;
        }
        let room = self
            .limits
            .max_request_bytes
            .saturating_sub(self.bytes.len());
        if bytes.len() > room {
            self.extent = Extent::Truncated;
        }
        self.bytes
            .extend_from_slice(bytes.get(..room.min(bytes.len())).unwrap_or_default());
    }

    /// Captured bytes are missing here; nothing after them is read.
    pub fn gap(&mut self) {
        self.extent = Extent::Gap;
    }

    /// The newest user input, once the body ended as `end` says.
    pub fn finish(self, end: BodyEnd) -> RequestActivity {
        let extent = match (self.extent, end) {
            (Extent::Complete, BodyEnd::Unfinished) => Extent::Truncated,
            (extent, BodyEnd::Complete | BodyEnd::Unfinished) => extent,
        };
        let mut limitations = Vec::new();
        match extent {
            Extent::Complete => {}
            Extent::Truncated => limitations.push(ActivityLimitation::Truncated),
            Extent::Gap => limitations.push(ActivityLimitation::CaptureGap),
        }
        let protocol = self.protocol;
        let document = match self.framing {
            // A WebSocket message is the document as the client sent it.
            RequestFraming::Message => Some(self.bytes),
            RequestFraming::Http => self.decoded(extent, &mut limitations),
        };
        let Some(document) = document else {
            return RequestActivity {
                limitations,
                ..RequestActivity::default()
            };
        };
        let mut activity = match serde_json::from_slice::<Value>(&document) {
            Ok(Value::Object(request)) => match protocol {
                Protocol::Messages => anthropic::Request::new(&request).activity(),
                Protocol::Responses => openai::Request::new(&request).activity(),
                Protocol::ChatCompletions => chat::Request::new(&request).activity(),
            },
            Ok(_) => RequestActivity {
                limitations: vec![ActivityLimitation::UnexpectedShape],
                ..RequestActivity::default()
            },
            Err(_) => RequestActivity {
                limitations: vec![ActivityLimitation::NotJson],
                ..RequestActivity::default()
            },
        };
        limitations.append(&mut activity.limitations);
        activity.limitations = ActivityLimitation::unique(limitations);
        activity
    }

    /// The body decompressed. Requests record no `Content-Encoding`, so
    /// compression is recognised by its magic bytes.
    fn decoded(
        &self,
        extent: Extent,
        limitations: &mut Vec<ActivityLimitation>,
    ) -> Option<Vec<u8>> {
        let encoding = ResponseEncoding::sniff(&self.bytes).unwrap_or(ResponseEncoding::Identity);
        let Ok(mut decoder) = Decompressor::new(encoding) else {
            limitations.push(ActivityLimitation::UnsupportedEncoding);
            return None;
        };
        let mut decoded = Vec::new();
        let mut oversized = false;
        let max = self.limits.max_decoded_bytes;
        let mut keep = |bytes: &[u8]| {
            let room = max.saturating_sub(decoded.len());
            oversized |= bytes.len() > room;
            decoded.extend_from_slice(bytes.get(..room.min(bytes.len())).unwrap_or_default());
        };
        let decoding = decoder.decode(&self.bytes, &mut keep);
        let finished = decoding.and_then(|()| decoder.finish(&mut keep));
        match (finished, extent) {
            (Ok(()), _) | (Err(_), Extent::Truncated | Extent::Gap) => {}
            (Err(_), Extent::Complete) => limitations.push(ActivityLimitation::DecodingError),
        }
        if oversized {
            limitations.push(ActivityLimitation::DecodedTooLarge);
        }
        Some(decoded)
    }
}

/// What a call's bodies showed: its request's newest input and its final
/// response's output. Raw text: it is sealed (redacted and cut) before it is
/// stored.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reading {
    pub request: RequestActivity,
    pub response: ResponseActivity,
}

impl Reading {
    /// A call on an endpoint whose bodies hold no activity TGV reads.
    pub fn unsupported() -> Self {
        Self {
            request: RequestActivity {
                limitations: vec![ActivityLimitation::UnsupportedEndpoint],
                ..RequestActivity::default()
            },
            response: ResponseActivity::default(),
        }
    }
}

impl ActivityLimitation {
    /// `limitations` in order, each once.
    fn unique(limitations: Vec<Self>) -> Vec<Self> {
        let mut unique = Vec::with_capacity(limitations.len());
        for limitation in limitations {
            if !unique.contains(&limitation) {
                unique.push(limitation);
            }
        }
        unique
    }
}

impl ToolInput {
    /// A tool input from a JSON string, kept as text when it is not JSON.
    pub fn parse(text: &str) -> Self {
        serde_json::from_str(text).map_or_else(|_| Self::Text(text.to_owned()), Self::Json)
    }
}

/// A native JSON object, read without panicking on any shape.
#[derive(Debug, Clone, Copy)]
struct Fields<'a>(&'a Map<String, Value>);

impl<'a> Fields<'a> {
    fn str(self, key: &str) -> Option<&'a str> {
        self.0.get(key).and_then(Value::as_str)
    }

    fn array(self, key: &str) -> &'a [Value] {
        self.0
            .get(key)
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice)
    }

    /// A content field as text: a string, or the text of its blocks.
    fn text(self, key: &str) -> String {
        match self.0.get(key) {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_)) | None => {
                String::new()
            }
        }
    }
}

#[cfg(test)]
mod tests;
