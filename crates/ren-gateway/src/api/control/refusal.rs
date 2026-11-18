//! Why the control API refused a request: one vocabulary for every control
//! route, each code with the one status it answers with. The body is
//! `{"api_version": 1, "error": "<code>"}`, with a `message` where a route
//! explains more. Clients (the CLI, the UI) parse the code into this enum
//! and keep codes they do not know.

use super::ControlApi;
use crate::control::ControlError;
use axum::{
    Json,
    response::{IntoResponse, Response},
};
use http::StatusCode;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// A control API error code.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ControlRefusal {
    // Access.
    /// The request came from a browser (`Origin` or `Sec-Fetch-Site`).
    CrossSite,
    /// No control token and no Tailscale identity.
    Unauthorized,
    /// A Tailscale login the daemon does not list.
    Forbidden,
    /// A body or parameter the route does not take.
    InvalidRequest,
    NotFound,
    /// The service is going away or did not answer.
    Unavailable,

    // Operator controls.
    /// The call to cancel is not running.
    NotRunning,
    /// No room to record the command: the recorder is saturated.
    Unrecorded,
    /// The command could not be recorded at all.
    Unrecordable,
    BackupRunning,
    BackupIncomplete,
    BackupFailed,

    // History edits.
    /// A call of the subject is still open.
    CallOpen,
    /// A run of the subject is still running.
    RunRunning,
    /// What the request needs could not be read.
    Unreadable,

    // Runs and sessions: starting.
    RunsDisabled,
    InvalidDirectory,
    DirectoryNotFound,
    /// A stored session's client is one this daemon does not know.
    NotDrivable,
    NoApprovals,
    NoScratch,
    SpawnFailed,
    /// The client's model listing failed.
    ModelsUnavailable,
    /// The client's command listing failed.
    CommandsUnavailable,

    // Runs and sessions: what the run's state refuses.
    Starting,
    Busy,
    NotWorking,
    TurnNotStarted,
    NoSuchApproval,
    Stopping,
    Ended,
    NothingQueued,
    /// The client has no way to do it mid-run.
    NotOffered,
    NoImages,
    /// The client's protocol is not in a state to take it.
    Protocol,
    ClientFixed,
    FolderFixed,
    Archived,
    NotArchived,
    NoProcess,

    // Attachments.
    NoAttachments,
    AttachmentSent,
    AttachmentTooLarge,
    AttachmentStorage,
    BlankPrompt,
    TooManyAttachments,
    AttachmentName,
    AttachmentEmpty,
    AttachmentUnrecognized,
    AttachmentUnreadable,
    AttachmentTooWide,

    // Sign-in.
    SignInNotOffered,
    NotInstalled,
    NoSignIn,
    NotAwaitingAnswer,
    RedirectNotAnAddress,
    RedirectElsewhere,
    RedirectIncomplete,

    // Tools.
    ToolsDisabled,
    ToolsBusy,
    ToolsNoGeneration,
    ToolsCollected,
    ToolsUnsupported,
    ToolsRefused,
    ToolsFailed,
}

impl ControlRefusal {
    /// The status the code answers with.
    pub fn status(self) -> StatusCode {
        match self {
            Self::InvalidRequest
            | Self::InvalidDirectory
            | Self::DirectoryNotFound
            | Self::NotDrivable
            | Self::NoApprovals => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::CrossSite | Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound | Self::NotRunning | Self::RunsDisabled | Self::ToolsNoGeneration => {
                StatusCode::NOT_FOUND
            }
            Self::BackupRunning
            | Self::CallOpen
            | Self::RunRunning
            | Self::Starting
            | Self::Busy
            | Self::NotWorking
            | Self::TurnNotStarted
            | Self::NoSuchApproval
            | Self::Stopping
            | Self::Ended
            | Self::NothingQueued
            | Self::NotOffered
            | Self::NoImages
            | Self::Protocol
            | Self::ClientFixed
            | Self::FolderFixed
            | Self::Archived
            | Self::NotArchived
            | Self::NoProcess
            | Self::AttachmentSent
            | Self::SignInNotOffered
            | Self::NotInstalled
            | Self::NoSignIn
            | Self::NotAwaitingAnswer
            | Self::RedirectNotAnAddress
            | Self::RedirectElsewhere
            | Self::RedirectIncomplete
            | Self::ToolsBusy
            | Self::ToolsCollected => StatusCode::CONFLICT,
            Self::AttachmentTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::BlankPrompt
            | Self::TooManyAttachments
            | Self::AttachmentName
            | Self::AttachmentEmpty
            | Self::AttachmentUnrecognized
            | Self::AttachmentUnreadable
            | Self::AttachmentTooWide
            | Self::ToolsRefused => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Unrecordable
            | Self::BackupFailed
            | Self::Unreadable
            | Self::AttachmentStorage
            | Self::ToolsFailed => StatusCode::INTERNAL_SERVER_ERROR,
            Self::ToolsUnsupported => StatusCode::NOT_IMPLEMENTED,
            Self::SpawnFailed | Self::ModelsUnavailable | Self::CommandsUnavailable => {
                StatusCode::BAD_GATEWAY
            }
            Self::Unavailable
            | Self::Unrecorded
            | Self::BackupIncomplete
            | Self::NoScratch
            | Self::NoAttachments
            | Self::ToolsDisabled => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    /// The refusal, with `message` saying more.
    pub(crate) fn explained(self, message: Option<String>) -> Response {
        (
            self.status(),
            Json(serde_json::json!({
                "api_version": ControlApi::API_VERSION,
                "error": self,
                "message": message,
            })),
        )
            .into_response()
    }
}

impl IntoResponse for ControlRefusal {
    fn into_response(self) -> Response {
        (
            self.status(),
            Json(serde_json::json!({ "api_version": ControlApi::API_VERSION, "error": self })),
        )
            .into_response()
    }
}

impl From<ControlError> for ControlRefusal {
    fn from(error: ControlError) -> Self {
        match error {
            ControlError::NotRunning => Self::NotRunning,
            ControlError::Unrecorded => Self::Unrecorded,
            ControlError::Unrecordable => Self::Unrecordable,
        }
    }
}
