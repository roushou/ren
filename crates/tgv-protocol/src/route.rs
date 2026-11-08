//! Where a call goes: the upstream provider, the wire format it speaks, and
//! the native endpoint.

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

impl Provider {
    /// The API this provider's own default upstream speaks.
    pub fn native_api(self) -> Api {
        let wire = match self {
            Self::Anthropic => Wire::AnthropicMessages,
            Self::Codex | Self::OpenAi => Wire::OpenAiResponses,
        };
        Api {
            provider: self,
            wire,
        }
    }

    /// This provider's API in `wire`, or why it has none: the only way to
    /// pair a provider with a wire format.
    pub fn speak(self, wire: Wire) -> Result<Api, UnsupportedWire> {
        let supported = match (self, wire) {
            (Self::Anthropic, Wire::AnthropicMessages)
            | (Self::Codex, Wire::OpenAiResponses)
            | (Self::OpenAi, Wire::OpenAiResponses) => true,
            (Self::Anthropic, Wire::OpenAiResponses)
            | (Self::Codex, Wire::AnthropicMessages)
            | (Self::OpenAi, Wire::AnthropicMessages) => false,
        };
        if supported {
            Ok(Api {
                provider: self,
                wire,
            })
        } else {
            Err(UnsupportedWire {
                provider: self,
                wire,
            })
        }
    }
}

/// A native wire format: the request and response shapes, independent of
/// which provider serves them.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Wire {
    AnthropicMessages,
    #[strum(to_string = "openai_responses")]
    OpenAiResponses,
}

/// A provider's API in one wire format. Built only by [`Provider::speak`],
/// so the pair is always one the provider offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Api {
    provider: Provider,
    wire: Wire,
}

impl Api {
    pub fn provider(self) -> Provider {
        self.provider
    }

    pub fn wire(self) -> Wire {
        self.wire
    }
}

/// A provider asked for a wire format it does not offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("provider {provider} does not offer the {wire} wire format")]
pub struct UnsupportedWire {
    pub provider: Provider,
    pub wire: Wire,
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

impl Endpoint {
    /// Whether a call to this endpoint asks a model for output, as opposed
    /// to counting tokens or listing models.
    pub fn generates(self) -> bool {
        match self {
            Self::Messages | Self::Responses | Self::ResponsesCompact => true,
            Self::CountTokens | Self::Models => false,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{Provider, Wire};
    use crate::label::Labeled;

    #[test]
    fn each_provider_speaks_exactly_its_own_wire() {
        let offered: Vec<(Provider, Wire)> = Provider::ALL
            .iter()
            .flat_map(|&provider| Wire::ALL.iter().map(move |&wire| (provider, wire)))
            .filter(|&(provider, wire)| provider.speak(wire).is_ok())
            .collect();
        assert_eq!(
            offered,
            [
                (Provider::Anthropic, Wire::AnthropicMessages),
                (Provider::Codex, Wire::OpenAiResponses),
                (Provider::OpenAi, Wire::OpenAiResponses),
            ]
        );
    }

    #[test]
    fn a_native_api_is_one_the_provider_speaks() {
        for &provider in Provider::ALL {
            let native = provider.native_api();
            assert_eq!(native.provider(), provider);
            assert_eq!(provider.speak(native.wire()), Ok(native));
        }
    }

    #[test]
    fn a_spoken_api_keeps_its_pair_and_a_refusal_names_it() {
        let api = Provider::Codex.speak(Wire::OpenAiResponses).unwrap();
        assert_eq!(
            (api.provider(), api.wire()),
            (Provider::Codex, Wire::OpenAiResponses)
        );
        let refused = Provider::Anthropic
            .speak(Wire::OpenAiResponses)
            .unwrap_err();
        assert_eq!(
            refused.to_string(),
            "provider anthropic does not offer the openai_responses wire format"
        );
    }
}
