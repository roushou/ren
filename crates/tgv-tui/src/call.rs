//! How a call ended, read from what was recorded about it: its lifecycle,
//! its transport terminal, and its model outcome, kept apart so an unknown
//! ending is never rounded up to success or down to failure.

use tgv_protocol::{inspect::ModelOutcome, label::Stored};
use tgv_recorder::capture::{AttemptTerminal, CallRejection, CallState};
use tgv_store::view::CallView;

/// How a call ended, or that it has not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallEnd {
    Open,
    /// TGV answered without forwarding.
    Rejected(Stored<CallRejection>),
    Error(ErrorCause),
    /// The transport stopped before the end.
    Stopped(Stop),
    /// Recorded at a restart: how it ended was never seen.
    Recovered,
    ModelCompleted,
    /// The model's own outcome stopped short of completion.
    Incomplete(Shortfall),
    /// The body ended, with no model outcome to read.
    HttpComplete,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCause {
    HttpStatus,
    Transport,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Cancelled,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortfall {
    Refused,
    Cancelled,
    Incomplete,
}

/// How something reads at a glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Live,
    Error,
    Warning,
    Success,
    Neutral,
}

/// A call's lifecycle, with unknown labels kept apart.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Life {
    Open,
    Finished,
    Recovered,
    Unknown,
}

/// Where the final attempt's transport got to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Transport {
    None,
    Complete,
    Failed,
    Stopped(Stop),
    Unknown,
}

/// What the model's outcome says.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Model {
    None,
    Completed,
    Stopped(Shortfall),
    Failed,
    Unknown,
}

impl CallEnd {
    /// The same reading as the web UI's call status.
    pub fn of(call: &CallView) -> Self {
        let life = match call.state.known() {
            Some(CallState::Open) => Life::Open,
            Some(CallState::Finished) => Life::Finished,
            Some(CallState::Recovered) => Life::Recovered,
            None => Life::Unknown,
        };
        if life == Life::Open {
            return Self::Open;
        }
        if let Some(rejection) = &call.rejection {
            return Self::Rejected(rejection.clone());
        }
        let transport = Self::transport(call.terminal.as_ref());
        let model = Self::model(call.outcome.as_ref());
        // Error evidence recorded before a restart is still true.
        if call.status.is_some_and(|status| status >= 400) {
            return Self::Error(ErrorCause::HttpStatus);
        }
        if transport == Transport::Failed {
            return Self::Error(ErrorCause::Transport);
        }
        if model == Model::Failed {
            return Self::Error(ErrorCause::Model);
        }
        if life == Life::Recovered {
            return Self::Recovered;
        }
        if model == Model::Completed {
            return Self::ModelCompleted;
        }
        if let Transport::Stopped(stop) = transport {
            return Self::Stopped(stop);
        }
        if let Model::Stopped(shortfall) = model {
            return Self::Incomplete(shortfall);
        }
        if life == Life::Unknown || model == Model::Unknown {
            return Self::Unknown;
        }
        if transport == Transport::Complete {
            return Self::HttpComplete;
        }
        Self::Unknown
    }

    fn transport(terminal: Option<&Stored<AttemptTerminal>>) -> Transport {
        let Some(terminal) = terminal else {
            return Transport::None;
        };
        match terminal.known() {
            Some(AttemptTerminal::BodyComplete | AttemptTerminal::TerminalEvent) => {
                Transport::Complete
            }
            Some(
                AttemptTerminal::NotSent
                | AttemptTerminal::Refused
                | AttemptTerminal::AcceptanceUnknown
                | AttemptTerminal::StreamFailed,
            ) => Transport::Failed,
            Some(AttemptTerminal::Cancelled) => Transport::Stopped(Stop::Cancelled),
            Some(AttemptTerminal::Interrupted) => Transport::Stopped(Stop::Interrupted),
            None => Transport::Unknown,
        }
    }

    fn model(outcome: Option<&Stored<ModelOutcome>>) -> Model {
        let Some(outcome) = outcome else {
            return Model::None;
        };
        match outcome.known() {
            Some(ModelOutcome::Completed) => Model::Completed,
            Some(ModelOutcome::Incomplete) => Model::Stopped(Shortfall::Incomplete),
            Some(ModelOutcome::Refused) => Model::Stopped(Shortfall::Refused),
            Some(ModelOutcome::Cancelled) => Model::Stopped(Shortfall::Cancelled),
            Some(ModelOutcome::Failed) => Model::Failed,
            Some(ModelOutcome::Unknown) | None => Model::Unknown,
        }
    }

    /// A short label, with the HTTP status when it is an error status.
    pub fn label(&self, status: Option<u16>) -> String {
        let label = match self {
            Self::Open => "in progress".to_owned(),
            Self::Rejected(rejection) => format!("TGV · {}", rejection.label().replace('_', " ")),
            Self::Error(_) => "error".to_owned(),
            Self::Stopped(Stop::Cancelled) => "cancelled".to_owned(),
            Self::Stopped(Stop::Interrupted) => "interrupted".to_owned(),
            Self::Recovered => "unknown · restart".to_owned(),
            Self::ModelCompleted => "model completed".to_owned(),
            Self::Incomplete(Shortfall::Refused) => "refused".to_owned(),
            Self::Incomplete(Shortfall::Cancelled) => "model cancelled".to_owned(),
            Self::Incomplete(Shortfall::Incomplete) => "incomplete".to_owned(),
            Self::HttpComplete => "HTTP complete".to_owned(),
            Self::Unknown => "unknown".to_owned(),
        };
        match status {
            Some(status) if status >= 400 => format!("{label} {status}"),
            Some(_) | None => label,
        }
    }

    pub fn tone(&self) -> Tone {
        match self {
            Self::Open => Tone::Live,
            Self::Error(_) => Tone::Error,
            Self::Rejected(_) | Self::Stopped(_) | Self::Recovered | Self::Incomplete(_) => {
                Tone::Warning
            }
            Self::ModelCompleted => Tone::Success,
            Self::HttpComplete | Self::Unknown => Tone::Neutral,
        }
    }

    /// Whether this ending is worth a look: anything but an open call or a
    /// completed one.
    pub fn troubled(&self) -> bool {
        match self {
            Self::Open | Self::ModelCompleted | Self::HttpComplete => false,
            Self::Rejected(_)
            | Self::Error(_)
            | Self::Stopped(_)
            | Self::Recovered
            | Self::Incomplete(_)
            | Self::Unknown => true,
        }
    }
}
