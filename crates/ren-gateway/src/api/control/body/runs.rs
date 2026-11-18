//! DRAFT

use super::sign_in::SignInState;
use crate::api::read::drive::{ApprovalAsk, Attachment, DriveEvent};
use crate::api::read::runs::ProcessExit;
use ren_harness::Harness;
use ren_harness::commands::CommandKind;
use ren_harness::commands::Sigil;
use ren_harness::drive::ApprovalId;
use ren_harness::models::DefaultChoice;
use ren_harness::models::ModelId;
use ren_harness::settings::ApprovalPolicy;
use ren_harness::settings::Effort;
use ren_harness::settings::Midway;
use ren_harness::sign_in::SignInMethod;
use ren_protocol::native::NativeThreadId;
use ren_protocol::native::NativeTurnId;
use ren_recorder::capture::RunEnd;
use ren_recorder::id::RunId;
use ren_recorder::id::SessionId;
use ren_supervisor::ClientStatus;
use ren_supervisor::PlaceKind;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use ts_rs::TS;

/// One run as the supervisor holds it now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunSnapshot {
    #[ts(as = "String")]
    pub run: RunId,
    /// The session the run serves.
    #[ts(as = "String")]
    pub session: SessionId,
    #[ts(as = "String")]
    pub harness: Harness,
    #[ts(as = "String")]
    pub workdir: PathBuf,
    pub state: RunState,
    /// What it was started with; each left out is the client's own.
    pub settings: RunSettings,
}

impl From<ren_supervisor::RunSnapshot> for RunSnapshot {
    fn from(value: ren_supervisor::RunSnapshot) -> Self {
        let ren_supervisor::RunSnapshot {
            run,
            session,
            harness,
            workdir,
            state,
            settings,
        } = value;
        Self {
            run,
            session,
            harness,
            workdir,
            state: state.into(),
            settings: settings.into(),
        }
    }
}

/// A change to a run, as it happens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum RunUpdate {
    State {
        #[ts(as = "String")]
        run: RunId,
        state: RunState,
    },
    Event {
        #[ts(as = "String")]
        run: RunId,
        event: DriveEvent,
    },
}

impl From<ren_supervisor::RunUpdate> for RunUpdate {
    fn from(value: ren_supervisor::RunUpdate) -> Self {
        match value {
            ren_supervisor::RunUpdate::State { run, state } => Self::State {
                run,
                state: state.into(),
            },
            ren_supervisor::RunUpdate::Event { run, event } => Self::Event {
                run,
                event: event.into(),
            },
        }
    }
}

/// Where a run is in its life. Written for the API and never read back:
/// a queued prompt is written as its record, without its images' bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "phase", rename_all = "snake_case")]
#[ts(export)]
pub enum RunState {
    /// The process started; the conversation is not open yet.
    Starting,
    /// The conversation is open and waits for a prompt.
    Idle {
        #[ts(as = "String")]
        thread: NativeThreadId,
    },
    /// A turn was asked for or is running. Its id is known once the agent
    /// reports it; approvals wait for the operator.
    Working {
        #[ts(as = "String")]
        thread: NativeThreadId,
        #[ts(as = "Option<String>")]
        turn: Option<NativeTurnId>,
        approvals: Vec<Waiting>,
        /// The operator's next prompt, sent as the turn ends.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queued: Option<RecordedPrompt>,
    },
    /// Ren asked the process to end and waits for it to exit.
    Stopping { cause: StopCause },
    /// The process is gone. Terminal.
    Ended {
        #[ts(as = "String")]
        end: RunEnd,
        exit: Option<ProcessExit>,
    },
}

impl From<ren_supervisor::RunState> for RunState {
    fn from(value: ren_supervisor::RunState) -> Self {
        match value {
            ren_supervisor::RunState::Starting => Self::Starting,
            ren_supervisor::RunState::Idle { thread } => Self::Idle { thread },
            ren_supervisor::RunState::Working {
                thread,
                turn,
                approvals,
                queued,
            } => Self::Working {
                thread,
                turn,
                approvals: approvals.iter().cloned().map(Into::into).collect(),
                queued: queued.map(|prompt| RecordedPrompt::from(&prompt)),
            },
            ren_supervisor::RunState::Stopping { cause } => Self::Stopping {
                cause: cause.into(),
            },
            ren_supervisor::RunState::Ended { end, exit } => Self::Ended {
                end,
                exit: exit.map(Into::into),
            },
        }
    }
}

/// Why Ren is stopping a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StopCause {
    Operator,
    Shutdown,
    /// The client refused to open the conversation.
    OpenRefused,
    /// The client sat idle between turns.
    Idle,
}

impl From<ren_supervisor::StopCause> for StopCause {
    fn from(value: ren_supervisor::StopCause) -> Self {
        match value {
            ren_supervisor::StopCause::Operator => Self::Operator,
            ren_supervisor::StopCause::Shutdown => Self::Shutdown,
            ren_supervisor::StopCause::OpenRefused => Self::OpenRefused,
            ren_supervisor::StopCause::Idle => Self::Idle,
        }
    }
}

/// Where runs can start: every place, and the folder scratch runs get
/// theirs in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Places {
    pub projects: Vec<Project>,
    /// None when runs cannot start in scratch here.
    pub scratch: Option<String>,
}

impl From<ren_supervisor::Places> for Places {
    fn from(value: ren_supervisor::Places) -> Self {
        let ren_supervisor::Places { projects, scratch } = value;
        Self {
            projects: projects.into_iter().map(Into::into).collect(),
            scratch,
        }
    }
}

/// A place a run can start in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Project {
    /// The absolute path to start a run in.
    pub path: String,
    /// Its folder's name, with its parent's when two of a kind share it.
    pub name: String,
    /// The path as the operator reads it: `~/…` under their home.
    pub shown: String,
    #[ts(as = "String")]
    pub kind: PlaceKind,
    pub git: ProjectGit,
}

impl From<ren_supervisor::Project> for Project {
    fn from(value: ren_supervisor::Project) -> Self {
        let ren_supervisor::Project {
            path,
            name,
            shown,
            kind,
            git,
        } = value;
        Self {
            path,
            name,
            shown,
            kind,
            git: git.into(),
        }
    }
}

/// A project's repository, as its `HEAD` reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ProjectGit {
    /// On a branch.
    Branch { name: String },
    /// At a commit, not on a branch: its abbreviated id.
    Detached { commit: String },
    /// A repository whose `HEAD` could not be read.
    Unreadable,
    /// No repository.
    None,
}

impl From<ren_supervisor::ProjectGit> for ProjectGit {
    fn from(value: ren_supervisor::ProjectGit) -> Self {
        match value {
            ren_supervisor::ProjectGit::Branch { name } => Self::Branch { name },
            ren_supervisor::ProjectGit::Detached { commit } => Self::Detached { commit },
            ren_supervisor::ProjectGit::Unreadable => Self::Unreadable,
            ren_supervisor::ProjectGit::None => Self::None,
        }
    }
}

/// A client, whether a run can start with it here, whether it is signed
/// in, and the approval policies it can run under.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Client {
    #[ts(as = "String")]
    pub harness: Harness,
    #[ts(as = "String")]
    pub status: ClientStatus,
    #[ts(as = "Vec<String>")]
    pub approvals: Vec<ApprovalPolicy>,
    /// What a running conversation takes mid-run: steering, and which
    /// settings change from the next turn.
    #[ts(as = "Vec<String>")]
    pub midway: Vec<Midway>,
    /// Its sign-in here, as its status command last said.
    pub sign_in: SignInState,
    /// How Ren can sign it in, first the usual way; none where it cannot.
    #[ts(as = "Vec<String>")]
    pub sign_in_methods: Vec<SignInMethod>,
}

impl From<ren_supervisor::Client> for Client {
    fn from(value: ren_supervisor::Client) -> Self {
        let ren_supervisor::Client {
            harness,
            status,
            approvals,
            midway,
            sign_in,
            sign_in_methods,
        } = value;
        Self {
            harness,
            status,
            approvals,
            midway,
            sign_in: sign_in.into(),
            sign_in_methods,
        }
    }
}

/// One model a client offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Model {
    #[ts(as = "String")]
    pub id: ModelId,
    /// The name the client shows for it.
    pub name: String,
    pub description: Option<String>,
    /// The provider serving it, where the client offers several (Pi).
    pub provider: Option<String>,
    /// Which model a run gets when none is chosen.
    #[ts(as = "String")]
    pub choice: DefaultChoice,
    /// The efforts it offers, least first; empty when it offers no choice.
    #[ts(as = "Vec<String>")]
    pub efforts: Vec<Effort>,
    /// The effort it runs at when none is chosen, when the client says.
    #[ts(as = "Option<String>")]
    pub effort: Option<Effort>,
}

impl From<ren_harness::models::Model> for Model {
    fn from(value: ren_harness::models::Model) -> Self {
        let ren_harness::models::Model {
            id,
            name,
            description,
            provider,
            choice,
            efforts,
            effort,
        } = value;
        Self {
            id,
            name,
            description,
            provider,
            choice,
            efforts,
            effort,
        }
    }
}

/// One command a running client offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClientCommand {
    /// Its name as the client lists it, without the sigil.
    pub name: String,
    /// What it does, in the client's words; empty when it gives none.
    pub description: String,
    /// What it takes after its name (`<file or PR>`), when the client says.
    #[serde(default)]
    pub hint: Option<String>,
    #[ts(as = "String")]
    pub kind: CommandKind,
    /// How a prompt names it.
    #[ts(as = "String")]
    pub sigil: Sigil,
    /// What a prompt writes to run it: its sigil and its name.
    pub invocation: String,
}

impl From<ren_harness::commands::ClientCommand> for ClientCommand {
    fn from(value: ren_harness::commands::ClientCommand) -> Self {
        let invocation = value.invocation();
        let ren_harness::commands::ClientCommand {
            name,
            description,
            hint,
            kind,
            sigil,
        } = value;
        Self {
            name,
            description,
            hint,
            kind,
            sigil,
            invocation,
        }
    }
}

/// A run's settings; each one left out is the client's own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunSettings {
    #[ts(as = "Option<String>")]
    pub model: Option<ModelId>,
    #[ts(as = "Option<String>")]
    pub effort: Option<Effort>,
    #[ts(as = "Option<String>")]
    pub approvals: Option<ApprovalPolicy>,
}

impl From<ren_harness::settings::RunSettings> for RunSettings {
    fn from(value: ren_harness::settings::RunSettings) -> Self {
        let ren_harness::settings::RunSettings {
            model,
            effort,
            approvals,
        } = value;
        Self {
            model,
            effort,
            approvals,
        }
    }
}

/// A prompt as Ren records and shows it: its text alone, or its text with
/// what each image sent with it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum RecordedPrompt {
    Text(String),
    WithImages {
        text: String,
        images: Vec<Attachment>,
    },
}

impl From<&ren_harness::prompt::Prompt> for RecordedPrompt {
    fn from(prompt: &ren_harness::prompt::Prompt) -> Self {
        let record = prompt.record();
        let text = record.as_str().to_owned();
        match record.images() {
            [] => Self::Text(text),
            images => Self::WithImages {
                text,
                images: images.iter().cloned().map(Into::into).collect(),
            },
        }
    }
}

/// One approval waiting for the operator's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Waiting {
    #[ts(as = "String")]
    pub approval: ApprovalId,
    pub ask: ApprovalAsk,
}

impl From<ren_supervisor::Waiting> for Waiting {
    fn from(waiting: ren_supervisor::Waiting) -> Self {
        let ren_supervisor::Waiting { approval, ask } = waiting;
        Self {
            approval,
            ask: ask.into(),
        }
    }
}

/// The runs the supervisor holds now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunsBody {
    pub runs: Vec<RunSnapshot>,
}

/// The clients a run can start with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClientsBody {
    pub clients: Vec<Client>,
}

/// The models a client offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelsBody {
    #[ts(as = "String")]
    pub harness: Harness,
    pub models: Vec<Model>,
}

/// The commands a client offers a prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CommandsBody {
    pub commands: Vec<ClientCommand>,
}

/// The `lagged` event of an update feed: it fell behind and skipped this
/// many updates; list again to catch up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Skipped {
    pub skipped: u64,
}
