//! Launching opencode through the gateway: its two providers' base URLs, in
//! the inline configuration it reads from the environment.

use crate::{Harness, endpoint::GatewayEndpoint, launch::LaunchPlan, settings::ApprovalPolicy};
use serde_json::{Value, json};
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
        Self::opencode_with(gateway, None)
    }

    /// opencode driven in its ACP mode, through the same override, which
    /// also carries the run's approval policy as permission rules (opencode
    /// 2.x's `permissions`: the last matching rule wins, after opencode's
    /// own). Without a policy, opencode's own permissions apply.
    pub(crate) fn opencode_driven(
        gateway: &GatewayEndpoint,
        approvals: Option<ApprovalPolicy>,
        arguments: Vec<String>,
    ) -> Self {
        let mut plan = Self::opencode_with(gateway, approvals.map(Self::opencode_permissions));
        plan.arguments = arguments.into_iter().map(Into::into).collect();
        plan
    }

    /// The rules for a policy: shell commands always ask; edits ask unless
    /// the policy lets edits in the folder go ahead.
    fn opencode_permissions(policy: ApprovalPolicy) -> Value {
        let ask = |action: &str| json!({ "action": action, "resource": "*", "effect": "ask" });
        match policy {
            ApprovalPolicy::Ask => json!([ask("shell"), ask("edit")]),
            ApprovalPolicy::Edits => json!([ask("shell")]),
            // Not offered for opencode; its own permissions decide.
            ApprovalPolicy::Auto => json!([]),
        }
    }

    fn opencode_with(gateway: &GatewayEndpoint, permissions: Option<Value>) -> Self {
        let mut config = json!({
            "provider": {
                "anthropic": {
                    "options": { "baseURL": gateway.url("/anthropic/v1").as_str() }
                },
                "openai": {
                    "options": { "baseURL": gateway.url("/openai/v1").as_str() }
                },
            }
        });
        if let (Some(permissions), Value::Object(fields)) = (permissions, &mut config) {
            fields.insert("permissions".into(), permissions);
        }
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
