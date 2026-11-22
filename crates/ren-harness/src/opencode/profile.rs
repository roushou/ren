//! opencode's facts: `opencode`, driven over ACP; it switches model and
//! effort mid-run and takes steering; its approvals are its configuration,
//! set at launch, where it allows everything unless told to ask, so it
//! offers asking first and letting edits go ahead; it takes an ACP `image`
//! block when its `initialize` answer says `promptCapabilities.image`; it
//! sends no user token; a driven run names its default model; it signs in
//! to ChatGPT through OpenAI's token sharing, or takes an API key.

use super::OpenCodeTool;
use crate::{
    attachment::{Delivery, Takes},
    launch::LaunchPlan,
    profile::{ClientProfile, DefaultModel, HomeEntry, LaunchNotice, TokenCarrier, sign_ins},
    settings::{ApprovalPolicy, Midway},
};
use ren_depot::package::PackageId;

impl ClientProfile {
    pub(crate) const OPENCODE: Self = Self {
        program: "opencode",
        packages: &[PackageId::OPENCODE],
        midway: &[Midway::Steer, Midway::Model, Midway::Effort],
        approvals: &[ApprovalPolicy::Ask, ApprovalPolicy::Edits],
        images: Takes::IfOffered(Delivery::InlineBase64),
        // Its header configuration is not yet confirmed.
        token: TokenCarrier::None,
        // Ren's override names opencode's `anthropic` and `openai`
        // providers, which opencode then counts as set up whether or not it
        // has a key for them, and with no model chosen it takes the first of
        // their models; its listing, made without the override, names the
        // default it would take on its own.
        default_model: DefaultModel::Named,
        notices: &[
            LaunchNotice::Replaces {
                variable: LaunchPlan::OPENCODE_CONFIG,
                advice: "keep other settings in opencode.json",
            },
            LaunchNotice::Routes(
                "Ren routes opencode's anthropic and openai providers; other providers keep their own endpoints.",
            ),
        ],
        sign_in: sign_ins! {
            Chatgpt => Pipes, PasteRedirect, || LaunchPlan::opencode_auth_login("openai", "chatgpt-token-sharing");
            AnthropicApiKey => Terminal, EnterKey, || LaunchPlan::opencode_auth_login("anthropic", "key");
            OpenaiApiKey => Terminal, EnterKey, || LaunchPlan::opencode_auth_login("openai", "key");
        },
        login_prompt: None,
        keeps: &[
            HomeEntry::new(".config/opencode"),
            HomeEntry::new(".local/share/opencode"),
            HomeEntry::new(".local/state/opencode"),
        ],
        user_context: OpenCodeTool::CONTEXT,
    };
}
