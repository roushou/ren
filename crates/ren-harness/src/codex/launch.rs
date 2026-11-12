//! Launching Codex through the gateway: its model provider replaced, for a
//! session or for its app server.

use crate::{
    Harness,
    endpoint::{GatewayEndpoint, UserToken},
    launch::{LaunchArg, LaunchPlan},
};
use std::collections::BTreeMap;

impl LaunchPlan {
    /// Codex, its model provider replaced by `gateway`.
    pub(crate) fn codex(gateway: &GatewayEndpoint) -> Self {
        let mut arguments: Vec<LaunchArg> = vec!["--no-daemon".into()];
        arguments.extend(Self::codex_provider(gateway));
        Self {
            harness: Harness::Codex,
            executable: "codex",
            arguments,
            environment: BTreeMap::new(),
        }
    }

    /// Codex's configuration overrides that send its model traffic to
    /// `gateway`. They replace the entire provider table so an existing entry
    /// cannot accidentally add a bearer token or credential helper.
    /// WebSockets are offered: a daemon that refuses them answers 426, and
    /// Codex falls back to HTTP/SSE for the session. A user token is sent from
    /// the environment, when it is set.
    pub(crate) fn codex_provider(gateway: &GatewayEndpoint) -> [LaunchArg; 4] {
        let base = toml::Value::String(gateway.url("/codex").to_string());
        [
            "--config".into(),
            "model_provider=\"ren_session\"".into(),
            "--config".into(),
            format!(
                "model_providers.ren_session={{name=\"Ren\",base_url={base},wire_api=\"responses\",requires_openai_auth=true,supports_websockets=true,env_http_headers={{\"{header}\"=\"{variable}\"}}}}",
                header = UserToken::HEADER,
                variable = UserToken::ENV,
            )
            .into(),
        ]
    }
}
