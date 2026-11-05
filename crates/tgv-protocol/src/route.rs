//! Where a call goes: the upstream provider and the native endpoint.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// An upstream provider TGV forwards to.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Provider {
    Anthropic,
    /// The ChatGPT-backed Codex endpoint, not the public OpenAI API.
    Codex,
    #[strum(to_string = "openai")]
    OpenAi,
}

/// A native endpoint, independent of its URL path on any provider.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Endpoint {
    Messages,
    CountTokens,
    Models,
    Responses,
    ResponsesCompact,
}

/// How a call was requested: an HTTP method, or a WebSocket message.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Method {
    #[strum(to_string = "GET")]
    Get,
    #[strum(to_string = "POST")]
    Post,
    /// A `response.create` message on a WebSocket channel.
    #[strum(to_string = "MESSAGE")]
    Message,
}

/// How a channel reaches TGV.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Transport {
    Http,
    #[strum(to_string = "websocket")]
    WebSocket,
}
