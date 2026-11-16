//! The capture lifecycle vocabulary shared by the recorder, the store, and the
//! read API.
//!
//! These labels describe what Ren observed on the wire. None of them claims
//! that a model finished, that a tool ran, or that the client received output.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// How one upstream attempt ended, as observed by the relay. Whether the
/// upstream accepted the request is part of the label: `not_sent` needs
/// positive evidence that nothing left Ren.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum AttemptTerminal {
    /// The HTTP response body reached its end. This is not model completion.
    BodyComplete,
    /// A WebSocket call's terminal event arrived (`response.completed`,
    /// `.failed`, `.incomplete`, or `error`). The event says how the model
    /// call ended; this label only says the call reached its end.
    TerminalEvent,
    /// The connection could not be established, so nothing was sent.
    NotSent,
    /// The request was sent, and the upstream refused it before any
    /// processing (HTTP/2 `REFUSED_STREAM`): not accepted.
    Refused,
    /// The request was, or may have been, sent, and no response head
    /// arrived. The upstream may have accepted it.
    AcceptanceUnknown,
    /// The response stream failed after its head arrived.
    StreamFailed,
    Cancelled,
    /// The relay dropped the attempt without observing an end.
    Interrupted,
}

/// How a terminal reads in history: what the `errors` and `interrupted`
/// filters select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalClass {
    /// The response body ended; the HTTP status and model outcome decide the rest.
    Complete,
    /// The exchange failed on the wire.
    Failed,
    /// The exchange was stopped before its end, by someone or by a drop.
    Stopped,
}

impl AttemptTerminal {
    pub fn class(self) -> TerminalClass {
        match self {
            Self::BodyComplete | Self::TerminalEvent => TerminalClass::Complete,
            Self::NotSent | Self::Refused | Self::AcceptanceUnknown | Self::StreamFailed => {
                TerminalClass::Failed
            }
            Self::Cancelled | Self::Interrupted => TerminalClass::Stopped,
        }
    }
}

/// Why an attempt ended, where the terminal alone does not say.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum TerminalCause {
    /// The connection to the upstream could not be established.
    ConnectFailed,
    /// The upstream reset the stream with HTTP/2 `REFUSED_STREAM`.
    StreamRefused,
    /// No response head before the gateway's header deadline.
    HeaderTimeout,
    /// Sending failed after the connection was in use, for example a write
    /// on a reused connection or a close before the response head.
    SendFailed,
    /// The response stream went quiet for longer than the idle timeout.
    IdleTimeout,
    /// The response stream broke: a reset, or fewer bytes than framed.
    BodyError,
    /// The upstream closed its WebSocket before the call's terminal event.
    UpstreamClosed,
    /// The client stopped: its upload failed, it went away, or (WebSocket)
    /// the upstream accepted its interrupt.
    Client,
    /// The daemon cancelled what remained after the shutdown drain deadline.
    Shutdown,
    /// The operator cancelled the call through the control API.
    Operator,
    /// With `body_complete`: the response's declared `content-length` was
    /// relayed whole, and the server stopped reading before the stream's own
    /// end arrived (an HTTP/2 upstream's END_STREAM, sent after the data).
    DeclaredLength,
}

/// How far an attempt had got when it ended.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum AttemptPhase {
    /// The client's request body was still being forwarded.
    Upload,
    /// The request was forwarded; no response head yet.
    AwaitingHead,
    /// The response head arrived; its body was streaming.
    Response,
}

/// The persisted form of an attempt's end. Cause and phase are absent when the
/// recorder's drop fallback wrote the record, or the stored record has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct TerminalRecord {
    pub terminal: AttemptTerminal,
    pub cause: Option<TerminalCause>,
    pub phase: Option<AttemptPhase>,
}

impl TerminalRecord {
    /// A capture handle dropped without a terminal: the relay lost track of it.
    pub const DROPPED: Self = Self {
        terminal: AttemptTerminal::Interrupted,
        cause: None,
        phase: None,
    };
}

/// Where a call is in its lifecycle.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum CallState {
    Open,
    Finished,
    /// The daemon stopped before the call's terminal was recorded. Its
    /// outcome is unknown.
    Recovered,
}

/// Why a call was answered by Ren without any upstream attempt.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum CallRejection {
    /// The request body exceeded the per-request buffer limit (`413`).
    TooLarge,
    /// The global request buffer budget was exhausted (`503`).
    BufferFull,
    /// The request body did not arrive within the upload timeouts (`408`).
    UploadTimeout,
    /// The client's request body failed: the client went away.
    UploadFailed,
    /// The handler was dropped while the request body was still arriving:
    /// its connection closed under it. No body error was observed, so
    /// which side ended the connection is unknown.
    Abandoned,
    /// The operator paused new calls (`503` with `retry-after`).
    Paused,
    /// An operator session block applied (non-retryable `403`).
    SessionBlocked,
    /// An operator block on the calling user applied (non-retryable `403`).
    UserBlocked,
    /// A configured block rule applied (non-retryable `403`).
    Blocked,
    /// Ren shut down while the request body was still arriving (`503`).
    Shutdown,
    /// The request's user token or Tailscale identity was refused:
    /// unknown, repeated, malformed, or naming two users (`401`).
    IdentityRefused,
    /// The call's upstream entry admits only some named users, and the
    /// caller is not one of them (non-retryable `403`).
    UserNotAllowed,
    /// The caller already had as many calls running as its limit allows
    /// (`429` with `retry-after`).
    UserBusy,
    /// The gateway was running as many calls as `max_concurrent_requests`
    /// allows (`503`).
    Busy,
}

/// How a captured body's byte stream ended.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum BodyEnding {
    /// Still being received, or never terminated.
    Open,
    /// The end of the body was observed.
    Ended,
    /// The stream stopped before its end: failure, cancellation, or restart.
    Unfinished,
}

/// Why captured bytes or records were not stored.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Omission {
    /// The per-body capture limit was reached.
    PolicyLimit,
    /// The recorder's byte budget was exhausted.
    QueueBytes,
    /// The recorder's queue had no free slot.
    QueueSlots,
    /// The call was admitted without a lifecycle reservation.
    Unrecorded,
    /// A storage write failed after the bytes were accepted.
    StorageFailure,
    /// A WebSocket message arrived while no call was open, so it belongs
    /// to no body. Its bytes are counted, not stored.
    Unattributed,
}

/// How a WebSocket channel ended.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ChannelEnd {
    /// The client sent a close frame; it was forwarded upstream.
    ClientClosed,
    /// The upstream sent a close frame; it was forwarded to the client.
    UpstreamClosed,
    /// The client socket failed or ended without a close frame.
    ClientFailed,
    /// The upstream socket failed or ended without a close frame.
    UpstreamFailed,
    /// The client sent nothing, not even a pong to Ren's ping, past the
    /// keepalive deadline; Ren closed the channel.
    ClientUnresponsive,
    /// The upstream sent nothing, not even a pong to Ren's ping, past the
    /// keepalive deadline; Ren closed the channel.
    UpstreamUnresponsive,
    /// The upstream answered pings but sent nothing for the running call past
    /// its deadline; Ren closed the channel.
    UpstreamStalled,
    /// Shutdown closed the channel once no call was open.
    Drained,
    /// Shutdown's deadline passed and the channel was closed with calls open.
    Aborted,
    /// The relay was dropped without observing an end.
    Dropped,
    /// The daemon stopped before the channel's end was recorded. Written
    /// only by storage recovery; the outcome is unknown.
    Recovered,
}

/// How an agent process exited, as the host reported it. Stored and sent as
/// its [`ExitParts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(into = "ExitParts", try_from = "ExitParts")]
pub enum ProcessExit {
    Code(i32),
    Signal(i32),
    /// The host could not say how the process exited.
    Unknown,
}

/// The kind of a [`ProcessExit`].
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ExitKind {
    Code,
    Signal,
    Unknown,
}

/// A [`ProcessExit`] as stored and sent: its kind, and the value a code or
/// a signal carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExitParts {
    pub kind: ExitKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<i32>,
}

/// Exit parts whose value does not fit their kind: a code or a signal
/// without one, or an unknown exit with one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a {kind} exit {}", if value.is_some() { "carries no value" } else { "needs a value" })]
pub struct ExitMismatch {
    pub kind: ExitKind,
    pub value: Option<i32>,
}

impl From<ProcessExit> for ExitParts {
    fn from(exit: ProcessExit) -> Self {
        let (kind, value) = match exit {
            ProcessExit::Code(code) => (ExitKind::Code, Some(code)),
            ProcessExit::Signal(signal) => (ExitKind::Signal, Some(signal)),
            ProcessExit::Unknown => (ExitKind::Unknown, None),
        };
        Self { kind, value }
    }
}

impl TryFrom<ExitParts> for ProcessExit {
    type Error = ExitMismatch;

    fn try_from(parts: ExitParts) -> Result<Self, Self::Error> {
        match (parts.kind, parts.value) {
            (ExitKind::Code, Some(code)) => Ok(Self::Code(code)),
            (ExitKind::Signal, Some(signal)) => Ok(Self::Signal(signal)),
            (ExitKind::Unknown, None) => Ok(Self::Unknown),
            (ExitKind::Code | ExitKind::Signal, None) | (ExitKind::Unknown, Some(_)) => {
                Err(ExitMismatch {
                    kind: parts.kind,
                    value: parts.value,
                })
            }
        }
    }
}

/// How a run, one agent process Ren started and drove, ended. The process's
/// exit status, when there is one, is recorded beside the label; neither says
/// whether the agent's work succeeded.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum RunEnd {
    /// The process could not be started; the agent never ran.
    SpawnFailed,
    /// The process started, but the client refused to open the
    /// conversation, so Ren stopped it.
    OpenRefused,
    /// The process exited without Ren asking it to.
    Exited,
    /// The operator stopped the run, and the process then ended.
    Stopped,
    /// Shutdown stopped the run, and the process then ended.
    Drained,
    /// The daemon stopped before the run's end was recorded. Written only by
    /// storage recovery; the outcome is unknown.
    Recovered,
    /// The client sat idle, so Ren stopped it; its session goes on, and the
    /// next prompt starts one again (a draft never asked anything and holding
    /// nothing is discarded instead).
    Idled,
}

#[cfg(test)]
mod tests {
    use super::{ExitKind, ExitMismatch, ExitParts, ProcessExit};
    use serde_json::json;

    #[test]
    fn an_exit_is_sent_as_its_kind_and_value() {
        for (exit, wire) in [
            (ProcessExit::Code(1), json!({ "kind": "code", "value": 1 })),
            (
                ProcessExit::Signal(9),
                json!({ "kind": "signal", "value": 9 }),
            ),
            (ProcessExit::Unknown, json!({ "kind": "unknown" })),
        ] {
            assert_eq!(serde_json::to_value(exit).unwrap(), wire);
            assert_eq!(serde_json::from_value::<ProcessExit>(wire).unwrap(), exit);
        }
    }

    #[test]
    fn parts_whose_value_does_not_fit_their_kind_are_refused() {
        for (kind, value) in [
            (ExitKind::Code, None),
            (ExitKind::Signal, None),
            (ExitKind::Unknown, Some(0)),
        ] {
            assert_eq!(
                ProcessExit::try_from(ExitParts { kind, value }),
                Err(ExitMismatch { kind, value })
            );
        }
        assert!(serde_json::from_value::<ProcessExit>(json!({ "kind": "code" })).is_err());
        assert!(
            serde_json::from_value::<ProcessExit>(json!({ "kind": "exited", "value": 0 })).is_err()
        );
    }
}
