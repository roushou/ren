//! Claude Code's facts: `claude`, driven in print mode; it switches model
//! and permission mode mid-run; it takes images in base64 in the message;
//! a launched session sends a user token as a custom header; it signs in
//! with a Claude subscription or an Anthropic Console account, pasting back
//! the code the page shows.

use super::{ClaudeAuth, ClaudeCodeTool};
use crate::{
    attachment::{Delivery, Takes},
    launch::LaunchPlan,
    profile::{ClientProfile, DefaultModel, TokenCarrier, sign_ins},
    settings::{ApprovalPolicy, Midway},
};
use ren_depot::package::PackageId;
use strum::VariantArray;

impl ClientProfile {
    pub(crate) const CLAUDE_CODE: Self = Self {
        program: "claude",
        packages: &[PackageId::CLAUDE],
        midway: &[Midway::Model, Midway::Approvals],
        approvals: ApprovalPolicy::VARIANTS,
        // An `image` content block.
        images: Takes::Always(Delivery::InlineBase64),
        token: TokenCarrier::CustomHeader,
        default_model: DefaultModel::ClientOwn,
        notices: &[],
        sign_in: sign_ins! {
            ClaudeSubscription => Pipes, PasteCode, || LaunchPlan::claude_auth_login("--claudeai");
            AnthropicConsole => Pipes, PasteCode, || LaunchPlan::claude_auth_login("--console");
        },
        login_prompt: Some(ClaudeAuth::PROMPT),
        user_context: ClaudeCodeTool::CONTEXT,
    };
}
