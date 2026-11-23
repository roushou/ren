//! Pi's facts: `pi` on Node, driven in RPC mode; it switches model and
//! thinking level mid-run and takes steering; it asks for no approvals, so
//! it offers no policy; it takes images in base64 (`images` on `prompt` and
//! `steer`); Ren's extension sends a user token from the environment; it
//! signs in to ChatGPT by device code, or takes an API key, on a terminal.

use crate::{
    attachment::{Delivery, Takes},
    launch::LaunchPlan,
    profile::{
        ClientNeeds, ClientProfile, DefaultModel, HomeEntry, LaunchNotice, TokenCarrier, sign_ins,
    },
    settings::Midway,
    tools::UserContext,
};
use ren_depot::package::PackageId;

impl ClientProfile {
    pub(crate) const PI: Self = Self {
        program: "pi",
        packages: &[PackageId::PI, PackageId::NODE],
        midway: &[Midway::Steer, Midway::Model, Midway::Effort],
        approvals: &[],
        images: Takes::Always(Delivery::InlineBase64),
        token: TokenCarrier::Env,
        default_model: DefaultModel::ClientOwn,
        notices: &[LaunchNotice::Routes(
            "Ren routes Pi's anthropic, openai, and openai-codex providers; other providers keep their own endpoints.",
        )],
        sign_in: sign_ins! {
            Chatgpt => Terminal, DeviceCode, || LaunchPlan::pi_auth_login("openai-codex", "--no-browser");
            AnthropicApiKey => Terminal, EnterKey, || LaunchPlan::pi_auth_login("anthropic", "--api-key");
            OpenaiApiKey => Terminal, EnterKey, || LaunchPlan::pi_auth_login("openai", "--api-key");
        },
        login_prompt: None,
        needs: ClientNeeds {
            keeps: &[HomeEntry::new(".pi")],
            reaches: &[],
        },
        user_context: UserContext::NONE,
    };
}
