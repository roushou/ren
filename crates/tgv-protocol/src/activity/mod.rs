//! What a call's captured bodies show an agent doing: the user text and tool
//! results its request carries, and the tool calls and reply text its
//! response holds. Read on demand from stored bodies, never on the
//! forwarding path.
//!
//! These are facts about the bytes. A tool call in a response was requested,
//! not executed; a tool result in a request is what the client reported.
//! Whatever could not be read is a [`ActivityLimitation`], never a guess.

pub mod anthropic;
pub mod openai;

use crate::encoding::{Decompressor, ResponseEncoding};
use crate::inspect::pipeline::Protocol;
use crate::sse::{SseFramer, SseLimits, SseOutput};
use serde_json::Value;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::ops::Range;
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

/// How much of a stored body was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyExtent {
    Complete,
    /// More was stored, or more will be: the bytes stop early.
    Truncated,
    /// Bytes are missing before the end.
    Gap,
}

/// One stored body, as the store hands it over.
#[derive(Debug, Clone, Copy)]
pub struct CapturedBody<'a> {
    pub bytes: &'a [u8],
    pub extent: BodyExtent,
    /// The recorded `Content-Encoding`; `None` when none was recorded, as for
    /// requests, whose compression is recognised by its magic bytes.
    pub encoding: Option<ResponseEncoding>,
    /// WebSocket message boundaries within `bytes`; `None` for HTTP.
    pub messages: Option<&'a [Range<usize>]>,
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
#[derive(Debug)]
pub struct ResponseReader {
    protocol: ProtocolResponse,
    limitations: Vec<ActivityLimitation>,
}

#[derive(Debug)]
enum ProtocolResponse {
    Messages(anthropic::Response),
    Responses(openai::Response),
}

impl ResponseReader {
    pub fn new(protocol: Protocol, limits: ActivityLimits) -> Self {
        Self {
            protocol: match protocol {
                Protocol::Messages => ProtocolResponse::Messages(anthropic::Response::new(limits)),
                Protocol::Responses => ProtocolResponse::Responses(openai::Response::new(limits)),
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
            }
        }
    }

    /// A whole response document.
    pub fn document_bytes(&mut self, bytes: &[u8]) {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(Value::Object(document)) => match &mut self.protocol {
                ProtocolResponse::Messages(response) => response.document(document),
                ProtocolResponse::Responses(response) => response.document(document),
            },
            Ok(_) => self.limitations.push(ActivityLimitation::UnexpectedShape),
            Err(_) => self.limitations.push(ActivityLimitation::NotJson),
        }
    }

    pub fn finish(self) -> ResponseActivity {
        let mut activity = match self.protocol {
            ProtocolResponse::Messages(response) => response.finish(),
            ProtocolResponse::Responses(response) => response.finish(),
        };
        let mut limitations = self.limitations;
        limitations.append(&mut activity.limitations);
        activity.limitations = ActivityReader::dedup(limitations);
        activity
    }
}

/// A request body, gathered as it is captured and read once it ends. The
/// newest user input is at the end of a conversation, so the whole body is
/// needed; up to `max_request_bytes` are kept.
#[derive(Debug)]
pub struct RequestReader {
    reader: ActivityReader,
    bytes: Vec<u8>,
    extent: BodyExtent,
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
            reader: ActivityReader::new(protocol, limits),
            bytes: Vec::new(),
            extent: BodyExtent::Complete,
            framing: RequestFraming::Http,
        }
    }

    /// The request is a WebSocket message: its bytes are one document.
    pub fn message(&mut self) {
        self.framing = RequestFraming::Message;
    }

    /// The next contiguous captured bytes.
    pub fn feed(&mut self, bytes: &[u8]) {
        if self.extent != BodyExtent::Complete {
            return;
        }
        let room = self
            .reader
            .limits
            .max_request_bytes
            .saturating_sub(self.bytes.len());
        if bytes.len() > room {
            self.extent = BodyExtent::Truncated;
        }
        self.bytes
            .extend_from_slice(bytes.get(..room.min(bytes.len())).unwrap_or_default());
    }

    /// Captured bytes are missing here; nothing after them is read.
    pub fn gap(&mut self) {
        self.extent = BodyExtent::Gap;
    }

    pub fn finish(self, end: crate::inspect::pipeline::BodyEnd) -> RequestActivity {
        let extent = match (self.extent, end) {
            (BodyExtent::Complete, crate::inspect::pipeline::BodyEnd::Unfinished) => {
                BodyExtent::Truncated
            }
            (extent, _) => extent,
        };
        let whole = 0..self.bytes.len();
        self.reader.request(CapturedBody {
            bytes: &self.bytes,
            extent,
            encoding: None,
            messages: match self.framing {
                RequestFraming::Http => None,
                RequestFraming::Message => Some(std::slice::from_ref(&whole)),
            },
        })
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

/// Reads activity from one protocol's bodies.
#[derive(Debug, Clone, Copy)]
pub struct ActivityReader {
    protocol: Protocol,
    limits: ActivityLimits,
}

/// Decoded body content: one document, a stream of events, or messages.
enum Decoded {
    Document(Vec<u8>),
    Events(Vec<Vec<u8>>),
}

impl ActivityReader {
    pub fn new(protocol: Protocol, limits: ActivityLimits) -> Self {
        Self { protocol, limits }
    }

    /// The newest user input in a request body.
    pub fn request(&self, body: CapturedBody<'_>) -> RequestActivity {
        let mut limitations = Vec::new();
        let document = match self.decode(body, &mut limitations) {
            Some(Decoded::Document(bytes)) => Some(bytes),
            // A WebSocket call's request is one message.
            Some(Decoded::Events(mut events)) => events.pop(),
            None => None,
        };
        let Some(document) = document else {
            return RequestActivity {
                limitations,
                ..RequestActivity::default()
            };
        };
        let mut activity = match serde_json::from_slice::<Value>(&document) {
            Ok(Value::Object(request)) => match self.protocol {
                Protocol::Messages => anthropic::Request::new(&request).activity(),
                Protocol::Responses => openai::Request::new(&request).activity(),
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
        activity.limitations = Self::dedup(limitations);
        activity
    }

    /// The tool calls and reply text in a response body.
    pub fn response(&self, body: CapturedBody<'_>) -> ResponseActivity {
        let mut limitations = Vec::new();
        let mut reader = ResponseReader::new(self.protocol, self.limits);
        match self.decode(body, &mut limitations) {
            Some(Decoded::Document(bytes)) => reader.document_bytes(&bytes),
            Some(Decoded::Events(events)) => {
                for event in &events {
                    reader.event_bytes(event);
                }
            }
            None => {}
        }
        let mut activity = reader.finish();
        limitations.append(&mut activity.limitations);
        activity.limitations = Self::dedup(limitations);
        activity
    }

    /// The body's content, decompressed and split into events or messages.
    fn decode(
        &self,
        body: CapturedBody<'_>,
        limitations: &mut Vec<ActivityLimitation>,
    ) -> Option<Decoded> {
        match body.extent {
            BodyExtent::Complete => {}
            BodyExtent::Truncated => limitations.push(ActivityLimitation::Truncated),
            BodyExtent::Gap => limitations.push(ActivityLimitation::CaptureGap),
        }
        if let Some(messages) = body.messages {
            return Some(Decoded::Events(
                messages
                    .iter()
                    .filter_map(|range| body.bytes.get(range.clone()).map(<[u8]>::to_vec))
                    .collect(),
            ));
        }
        let encoding = body
            .encoding
            // Requests record no `Content-Encoding`: recognise compression
            // by its magic bytes.
            .unwrap_or_else(|| {
                ResponseEncoding::sniff(body.bytes).unwrap_or(ResponseEncoding::Identity)
            });
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
        let decoding = decoder.decode(body.bytes, &mut keep);
        let finished = decoding.and_then(|()| decoder.finish(&mut keep));
        match (finished, body.extent) {
            (Ok(()), _) | (Err(_), BodyExtent::Truncated | BodyExtent::Gap) => {}
            (Err(_), BodyExtent::Complete) => limitations.push(ActivityLimitation::DecodingError),
        }
        if oversized {
            limitations.push(ActivityLimitation::DecodedTooLarge);
        }
        if decoded.trim_ascii_start().first() == Some(&b'{') {
            return Some(Decoded::Document(decoded));
        }
        let mut framer = SseFramer::new(self.limits.sse);
        let mut events = Vec::new();
        framer.feed(&decoded, |output| match output {
            SseOutput::Event { data, .. } => events.push(data.to_vec()),
            SseOutput::Discarded(_) => {}
        });
        Some(Decoded::Events(events))
    }

    fn dedup(limitations: Vec<ActivityLimitation>) -> Vec<ActivityLimitation> {
        let mut unique = Vec::with_capacity(limitations.len());
        for limitation in limitations {
            if !unique.contains(&limitation) {
                unique.push(limitation);
            }
        }
        unique
    }
}

/// Reading native JSON without panicking on any shape.
impl ActivityReader {
    fn str<'a>(object: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
        object.get(key).and_then(Value::as_str)
    }

    fn array<'a>(object: &'a serde_json::Map<String, Value>, key: &str) -> &'a [Value] {
        object
            .get(key)
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice)
    }

    /// Text from a string, or from the `text` of an array of blocks.
    fn text(value: Option<&Value>) -> String {
        match value {
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

    /// A tool input from a JSON string, kept as text when it is not JSON.
    fn input_from_text(text: &str) -> ToolInput {
        serde_json::from_str(text)
            .map_or_else(|_| ToolInput::Text(text.to_owned()), ToolInput::Json)
    }
}

#[cfg(test)]
mod tests;
