//! Launching opencode through the gateway: its two providers' base URLs, in
//! the inline configuration it reads from the environment.

use crate::{Harness, endpoint::GatewayEndpoint, launch::LaunchPlan};
use std::collections::BTreeMap;

impl LaunchPlan {
    /// The environment variable opencode reads inline configuration from.
    pub(crate) const OPENCODE_CONFIG: &'static str = "OPENCODE_CONFIG_CONTENT";

    /// opencode, through its documented runtime override, which it merges
    /// over the global and project files. Only the two providers' base URLs
    /// are set: keys, sign-in, and every other provider stay opencode's own.
    /// Its OpenAI WebSocket is refused with 426, and opencode keeps the
    /// session on HTTP/SSE.
    pub(crate) fn opencode(gateway: &GatewayEndpoint) -> Self {
        let config = serde_json::json!({
            "provider": {
                "anthropic": {
                    "options": { "baseURL": gateway.url("/anthropic/v1").as_str() }
                },
                "openai": {
                    "options": { "baseURL": gateway.url("/openai/v1").as_str() }
                },
            }
        });
        let mut environment = BTreeMap::new();
        environment.insert(Self::OPENCODE_CONFIG.into(), config.to_string().into());
        Self {
            harness: Harness::OpenCode,
            executable: "opencode",
            arguments: Vec::new(),
            environment,
        }
    }
}
