//! The capture lifecycle vocabulary shared by the recorder, the store, and the
//! read API.
//!
//! These labels describe what TGV observed on the wire. None of them claims
//! that a model finished, that a tool ran, or that the client received output.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// How one upstream attempt ended, as observed by the relay. Whether the
/// upstream accepted the request is part of the label: `not_sent` needs
/// positive evidence that nothing left TGV.
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
/// record was written by the recorder's drop fallback or before they existed.
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

/// Why a call was answered by TGV without any upstream attempt.
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
    /// A configured block rule applied (non-retryable `403`).
    Blocked,
    /// TGV shut down while the request body was still arriving (`503`).
    Shutdown,
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
    /// The client sent nothing, not even a pong to TGV's ping, past the
    /// keepalive deadline; TGV closed the channel.
    ClientUnresponsive,
    /// The upstream sent nothing, not even a pong to TGV's ping, past the
    /// keepalive deadline; TGV closed the channel.
    UpstreamUnresponsive,
    /// The upstream answered pings but sent nothing for the running call past
    /// its deadline; TGV closed the channel.
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
