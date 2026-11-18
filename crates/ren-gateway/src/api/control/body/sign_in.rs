//! DRAFT

use crate::api::read::runs::ProcessExit;
use ren_harness::Harness;
use ren_harness::sign_in::SignInMethod;
use serde::{Deserialize, Serialize};
use std::fmt;
use ts_rs::TS;

/// What Ren knows of a client's sign-in here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum SignInState {
    /// Ren cannot check this client's sign-in: it offers no command Ren
    /// reads, or it is not installed.
    NotChecked,
    /// Its status command is running, and nothing was read before.
    Checking,
    SignedIn {
        method: String,
    },
    SignedOut,
    /// Its status command could not be run or read: whether it is signed in
    /// is unknown.
    Unknown {
        reason: String,
    },
}

impl From<ren_supervisor::SignInState> for SignInState {
    fn from(value: ren_supervisor::SignInState) -> Self {
        match value {
            ren_supervisor::SignInState::NotChecked => Self::NotChecked,
            ren_supervisor::SignInState::Checking => Self::Checking,
            ren_supervisor::SignInState::SignedIn { method } => Self::SignedIn {
                method: method.to_string(),
            },
            ren_supervisor::SignInState::SignedOut => Self::SignedOut,
            ren_supervisor::SignInState::Unknown { reason } => Self::Unknown { reason },
        }
    }
}

/// One run of a client's login.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum Login {
    /// Started; its link has not been printed yet.
    Starting {
        said: Option<String>,
    },
    /// Waiting for the code the page at `link` shows.
    AwaitingCode {
        link: String,
        said: Option<String>,
    },
    /// Waiting for the operator to enter `code` on the page at `link` and
    /// approve (a device login); `code` is none until it is printed.
    AwaitingApproval {
        link: String,
        code: Option<DeviceCode>,
        said: Option<String>,
    },
    /// Waiting for an API key.
    AwaitingKey {
        said: Option<String>,
    },
    /// Waiting for the address the browser could not open after approving
    /// at `link`, to open it on the host.
    AwaitingRedirect {
        link: String,
        said: Option<String>,
    },
    /// The answer was handed to the client; waiting for it to finish.
    Finishing {
        link: Option<String>,
        said: Option<String>,
    },
    Ended(LoginEnd),
}

impl From<ren_supervisor::Login> for Login {
    fn from(value: ren_supervisor::Login) -> Self {
        match value {
            ren_supervisor::Login::Starting { flow: _, said } => Self::Starting { said },
            ren_supervisor::Login::AwaitingCode { link, said } => Self::AwaitingCode {
                link: link.as_str().to_owned(),
                said,
            },
            ren_supervisor::Login::AwaitingApproval { link, code, said } => {
                Self::AwaitingApproval {
                    link: link.as_str().to_owned(),
                    code: code.map(|code| DeviceCode(code.as_str().to_owned())),
                    said,
                }
            }
            ren_supervisor::Login::AwaitingKey { said } => Self::AwaitingKey { said },
            ren_supervisor::Login::AwaitingRedirect { link, said } => Self::AwaitingRedirect {
                link: link.as_str().to_owned(),
                said,
            },
            ren_supervisor::Login::Finishing { link, said } => Self::Finishing {
                link: link.map(|link| link.as_str().to_owned()),
                said,
            },
            ren_supervisor::Login::Ended(v0) => Self::Ended(v0.into()),
        }
    }
}

/// How a login ended. Whether the client is signed in after it is what the
/// status check that follows says, not this.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "end", rename_all = "snake_case")]
#[ts(export)]
pub enum LoginEnd {
    /// The login exited by itself, having last said `said`.
    Exited {
        exit: ProcessExit,
        said: Option<String>,
    },
    /// The operator cancelled it, or a newer login replaced it.
    Cancelled,
    /// No answer or approval came, or the client did not finish, in time.
    TimedOut,
    /// Its output ended unreadably, its input closed, or the address could
    /// not be opened: it was stopped, having last said `said`.
    Broken { said: Option<String> },
}

impl From<ren_supervisor::LoginEnd> for LoginEnd {
    fn from(value: ren_supervisor::LoginEnd) -> Self {
        match value {
            ren_supervisor::LoginEnd::Exited { exit, said } => Self::Exited {
                exit: exit.into(),
                said,
            },
            ren_supervisor::LoginEnd::Cancelled => Self::Cancelled,
            ren_supervisor::LoginEnd::TimedOut => Self::TimedOut,
            ren_supervisor::LoginEnd::Broken { said } => Self::Broken { said },
        }
    }
}

/// A login's end, and the client's sign-in as checked after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LoginOutcome {
    pub end: LoginEnd,
    pub state: SignInState,
}

impl From<ren_supervisor::LoginOutcome> for LoginOutcome {
    fn from(value: ren_supervisor::LoginOutcome) -> Self {
        let ren_supervisor::LoginOutcome { end, state } = value;
        Self {
            end: end.into(),
            state: state.into(),
        }
    }
}

/// Where a waited-on login stands: still going, or ended and checked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum Waited {
    Going { login: Login },
    Done { outcome: LoginOutcome },
}

impl From<ren_supervisor::Waited> for Waited {
    fn from(value: ren_supervisor::Waited) -> Self {
        match value {
            ren_supervisor::Waited::Going { login } => Self::Going {
                login: login.into(),
            },
            ren_supervisor::Waited::Done { outcome } => Self::Done {
                outcome: outcome.into(),
            },
        }
    }
}

/// The one-time code a device login shows, for the operator to enter on the
/// sign-in page. Sent to the operator, never shown by `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, type = "string")]
pub struct DeviceCode(String);

impl DeviceCode {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DeviceCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeviceCode(..)")
    }
}

/// A client's sign-in here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SignInBody {
    #[ts(as = "String")]
    pub harness: Harness,
    pub sign_in: SignInState,
}

/// A login started: the client, how it signs in, and where the login is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LoginBody {
    #[ts(as = "String")]
    pub harness: Harness,
    #[ts(as = "String")]
    pub method: SignInMethod,
    pub login: Login,
}

/// A login answered: how it ended, and the sign-in checked after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OutcomeBody {
    #[ts(as = "String")]
    pub harness: Harness,
    pub outcome: LoginOutcome,
}

/// A login waited on: still going, or ended and checked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WaitedBody {
    #[ts(as = "String")]
    pub harness: Harness,
    pub waited: Waited,
}

/// A login cancelled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Cancelled {
    pub ok: bool,
}
