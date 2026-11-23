//! Codex's facts: `codex`, driven through its app server; it sets each
//! turn's model, effort, and approvals and takes steering; it reads an
//! image from its path (`localImage`); a launched session sends a user token
//! from the environment; it signs in to ChatGPT by device code.

use super::CodexTool;
use crate::{
    attachment::{Delivery, Takes},
    launch::LaunchPlan,
    profile::{ClientProfile, DefaultModel, HomeEntry, TokenCarrier, sign_ins},
    settings::{ApprovalPolicy, Midway},
};
use ren_depot::package::PackageId;
use strum::VariantArray;

impl ClientProfile {
    pub(crate) const CODEX: Self = Self {
        program: "codex",
        packages: &[PackageId::CODEX],
        midway: Midway::VARIANTS,
        approvals: ApprovalPolicy::VARIANTS,
        // In `turn/start` and `turn/steer`.
        images: Takes::Always(Delivery::LocalPath),
        token: TokenCarrier::Env,
        default_model: DefaultModel::ClientOwn,
        notices: &[],
        sign_in: sign_ins! {
            Chatgpt => Pipes, DeviceCode, LaunchPlan::codex_device_login;
        },
        login_prompt: None,
        keeps: &[HomeEntry::new(".codex")],
        user_context: CodexTool::CONTEXT,
    };
}
