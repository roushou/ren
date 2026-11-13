//! Launching Claude Code through the gateway: its Anthropic base URL, and
//! the hint headers the gateway reads.

use crate::{Harness, endpoint::GatewayEndpoint, launch::LaunchPlan};
use std::collections::BTreeMap;

impl LaunchPlan {
    /// Claude Code, its model traffic sent to `gateway`, which reads its hint
    /// headers.
    pub(crate) fn claude_code(gateway: &GatewayEndpoint) -> Self {
        let mut environment = BTreeMap::new();
        environment.insert(
            "ANTHROPIC_BASE_URL".into(),
            gateway.url("/anthropic").to_string().into(),
        );
        environment.insert("CLAUDE_CODE_GATEWAY_HINT_HEADERS".into(), "1".into());
        Self {
            harness: Harness::ClaudeCode,
            executable: "claude",
            arguments: Vec::new(),
            environment,
        }
    }
}
