//! The lifecycle of one HTTP attempt, as a pure state machine.
//!
//! The relay feeds it events from three owners: the upload body, the dispatch
//! that waits for the response head, and the response body. Each transition
//! returns the actions the shell must run. The machine performs no I/O, so the
//! classification of every terminal is testable without sockets.
//!
//! Facts are kept apart: whether anything was sent, what status came
//! back, and how the body ended are separate events, never one merged error.

use ren_recorder::capture::{AttemptPhase, AttemptTerminal, TerminalCause, TerminalRecord};

/// Where an attempt is. `Ended` is the only terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum HttpAttempt {
    /// The request was handed to the upstream; no response head yet.
    Dispatched {
        upload: Upload,
    },
    /// The response head arrived with this status; its body is streaming.
    /// The upload no longer decides the outcome, so it is not tracked here.
    Streaming {
        status: u16,
    },
    Ended(AttemptEnd),
}

/// The client's request body, which streams concurrently with dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upload {
    Streaming,
    Ended,
    /// The client's body failed, which means the client went away mid-upload.
    Failed,
}

/// What the relay observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpEvent {
    UploadEnded,
    UploadFailed,
    /// The upstream's response head arrived.
    Responded {
        status: u16,
    },
    /// Dispatch failed before a response head.
    SendFailed(SendFailure),
    /// The response body reached its end.
    BodyEnded,
    /// The server stopped reading the response body after relaying exactly
    /// the length its head declared, so the stream's own end was not
    /// observed. The declared body is complete: HTTP/1.1 frames a body by its
    /// `content-length`, and HTTP/2 requires DATA to add up to it. An HTTP/2
    /// upstream can send END_STREAM after the last DATA, and a server writing
    /// HTTP/1.1 stops polling once the length is written.
    LengthDelivered,
    BodyFailed(StreamFailure),
    Cancelled(Canceller),
    /// The owner of the attempt was dropped without a terminal: the handler
    /// future before the head, or the response body after it.
    Dropped,
}

/// Why dispatch produced no response head. The variants are the acceptance
/// facts: only a connection failure or a refused stream proves the
/// upstream did not accept the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendFailure {
    /// The connection could not be established.
    NotSent,
    /// The upstream reset the stream with HTTP/2 `REFUSED_STREAM`: the request
    /// was sent but closed before any processing (RFC 9113 section 8.7).
    Refused,
    /// Sending failed on a connection that was in use: a write on a reused
    /// connection, or a close before the response head.
    SendFailed,
    /// The header deadline passed. The request may have been accepted.
    HeaderTimeout,
}

/// Why a response body stopped early. An idle timeout and a broken stream are
/// different evidence and stay distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFailure {
    IdleTimeout,
    /// A reset, an abort, or fewer bytes than the framing promised.
    BodyError,
}

/// Who stopped an attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Canceller {
    Client,
    /// The daemon's shutdown drain deadline passed.
    Shutdown,
    /// The operator, through the control API.
    Operator,
}

/// How an attempt ended, and how far it had got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptEnd {
    pub how: EndKind,
    pub phase: AttemptPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndKind {
    /// The response body reached its end. Not model completion.
    BodyComplete,
    /// The declared length was relayed whole and the server let go of the
    /// body before its end was observed. Also not model completion.
    LengthDelivered,
    Unsent(SendFailure),
    StreamFailed(StreamFailure),
    Cancelled(Canceller),
    /// Dropped without an observed end.
    Interrupted,
}

/// What the shell must do after a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptAction {
    /// Record the terminal, stop any upload still streaming, drop the upstream
    /// exchange, and release the attempt's admission.
    Finish(AttemptEnd),
}

/// The state after an event, and the actions it requires.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct Transition {
    pub state: HttpAttempt,
    pub actions: Vec<AttemptAction>,
}

/// An event that cannot happen in the current state. After the end, late
/// events are expected races (a send error after a cancel, say): the shell
/// receives `AfterEnd` and decides, instead of the machine ignoring them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IllegalTransition {
    #[error("the attempt already ended")]
    AfterEnd { event: HttpEvent },
    #[error("a body event arrived before the response head")]
    BodyBeforeHead { event: HttpEvent },
    #[error("a dispatch event arrived after the response head")]
    DispatchAfterHead { event: HttpEvent },
}

impl HttpAttempt {
    /// An attempt whose request was just handed to the upstream.
    pub const DISPATCHED: Self = Self::Dispatched {
        upload: Upload::Streaming,
    };

    pub fn step(self, event: HttpEvent) -> Result<Transition, IllegalTransition> {
        match self {
            Self::Ended(_) => Err(IllegalTransition::AfterEnd { event }),
            Self::Dispatched { upload } => Self::dispatched(upload, event),
            Self::Streaming { status } => Self::streaming(status, event),
        }
    }

    fn dispatched(upload: Upload, event: HttpEvent) -> Result<Transition, IllegalTransition> {
        let phase = match upload {
            Upload::Streaming | Upload::Failed => AttemptPhase::Upload,
            Upload::Ended => AttemptPhase::AwaitingHead,
        };
        let end = |how| Ok(Transition::end(AttemptEnd { how, phase }));
        match event {
            HttpEvent::UploadEnded => Ok(Transition::to(Self::Dispatched {
                upload: Upload::Ended,
            })),
            // The send that carries this body can only fail now; the client's
            // abort is the cause, not the transport.
            HttpEvent::UploadFailed => end(EndKind::Cancelled(Canceller::Client)),
            HttpEvent::Responded { status } => Ok(Transition::to(Self::Streaming { status })),
            HttpEvent::SendFailed(failure) => end(EndKind::Unsent(failure)),
            HttpEvent::Cancelled(by) => end(EndKind::Cancelled(by)),
            HttpEvent::Dropped => end(EndKind::Interrupted),
            HttpEvent::BodyEnded | HttpEvent::LengthDelivered | HttpEvent::BodyFailed(_) => {
                Err(IllegalTransition::BodyBeforeHead { event })
            }
        }
    }

    fn streaming(status: u16, event: HttpEvent) -> Result<Transition, IllegalTransition> {
        let end = |how| {
            Ok(Transition::end(AttemptEnd {
                how,
                phase: AttemptPhase::Response,
            }))
        };
        match event {
            // Once the head arrived the upload no longer decides the outcome: a
            // client that left is observed when the response body is dropped,
            // and an upload still streaming is stopped when the attempt ends.
            HttpEvent::UploadEnded | HttpEvent::UploadFailed => {
                Ok(Transition::to(Self::Streaming { status }))
            }
            HttpEvent::BodyEnded => end(EndKind::BodyComplete),
            HttpEvent::LengthDelivered => end(EndKind::LengthDelivered),
            HttpEvent::BodyFailed(failure) => end(EndKind::StreamFailed(failure)),
            HttpEvent::Cancelled(by) => end(EndKind::Cancelled(by)),
            HttpEvent::Dropped => end(EndKind::Interrupted),
            HttpEvent::Responded { .. } | HttpEvent::SendFailed(_) => {
                Err(IllegalTransition::DispatchAfterHead { event })
            }
        }
    }
}

impl Transition {
    fn to(state: HttpAttempt) -> Self {
        Self {
            state,
            actions: Vec::new(),
        }
    }

    fn end(end: AttemptEnd) -> Self {
        Self {
            state: HttpAttempt::Ended(end),
            actions: vec![AttemptAction::Finish(end)],
        }
    }
}

impl From<AttemptEnd> for TerminalRecord {
    fn from(end: AttemptEnd) -> Self {
        let (terminal, cause) = match end.how {
            EndKind::BodyComplete => (AttemptTerminal::BodyComplete, None),
            EndKind::LengthDelivered => (
                AttemptTerminal::BodyComplete,
                Some(TerminalCause::DeclaredLength),
            ),
            EndKind::Unsent(SendFailure::NotSent) => {
                (AttemptTerminal::NotSent, Some(TerminalCause::ConnectFailed))
            }
            EndKind::Unsent(SendFailure::Refused) => {
                (AttemptTerminal::Refused, Some(TerminalCause::StreamRefused))
            }
            EndKind::Unsent(SendFailure::SendFailed) => (
                AttemptTerminal::AcceptanceUnknown,
                Some(TerminalCause::SendFailed),
            ),
            EndKind::Unsent(SendFailure::HeaderTimeout) => (
                AttemptTerminal::AcceptanceUnknown,
                Some(TerminalCause::HeaderTimeout),
            ),
            EndKind::StreamFailed(StreamFailure::IdleTimeout) => (
                AttemptTerminal::StreamFailed,
                Some(TerminalCause::IdleTimeout),
            ),
            EndKind::StreamFailed(StreamFailure::BodyError) => (
                AttemptTerminal::StreamFailed,
                Some(TerminalCause::BodyError),
            ),
            EndKind::Cancelled(Canceller::Client) => {
                (AttemptTerminal::Cancelled, Some(TerminalCause::Client))
            }
            EndKind::Cancelled(Canceller::Shutdown) => {
                (AttemptTerminal::Cancelled, Some(TerminalCause::Shutdown))
            }
            EndKind::Cancelled(Canceller::Operator) => {
                (AttemptTerminal::Cancelled, Some(TerminalCause::Operator))
            }
            EndKind::Interrupted => (AttemptTerminal::Interrupted, None),
        };
        Self {
            terminal,
            cause,
            phase: Some(end.phase),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AttemptAction, AttemptEnd, Canceller, EndKind, HttpAttempt, HttpEvent, IllegalTransition,
        SendFailure, StreamFailure,
    };
    use ren_recorder::capture::{AttemptPhase, AttemptTerminal, TerminalCause, TerminalRecord};

    /// Run events from dispatch; return the end, failing on any illegal step.
    fn run(events: &[HttpEvent]) -> Option<AttemptEnd> {
        let mut state = HttpAttempt::DISPATCHED;
        let mut finished = None;
        for event in events {
            let transition = state.step(*event).unwrap();
            for action in transition.actions {
                let AttemptAction::Finish(end) = action;
                assert!(finished.replace(end).is_none(), "finished twice");
            }
            state = transition.state;
        }
        finished
    }

    fn record(events: &[HttpEvent]) -> TerminalRecord {
        TerminalRecord::from(run(events).unwrap())
    }

    #[test]
    fn a_complete_exchange_ends_once_with_body_complete() {
        let end = run(&[
            HttpEvent::UploadEnded,
            HttpEvent::Responded { status: 200 },
            HttpEvent::BodyEnded,
        ]);
        assert_eq!(
            end,
            Some(AttemptEnd {
                how: EndKind::BodyComplete,
                phase: AttemptPhase::Response,
            })
        );
        // An upstream error status is still a complete body; status is separate.
        assert_eq!(
            record(&[HttpEvent::Responded { status: 529 }, HttpEvent::BodyEnded]).terminal,
            AttemptTerminal::BodyComplete
        );
    }

    #[test]
    fn only_a_connection_failure_is_not_sent() {
        let refused = record(&[HttpEvent::SendFailed(SendFailure::Refused)]);
        assert_eq!(
            (refused.terminal, refused.cause),
            (AttemptTerminal::Refused, Some(TerminalCause::StreamRefused))
        );
        let not_sent = record(&[HttpEvent::SendFailed(SendFailure::NotSent)]);
        assert_eq!(
            (not_sent.terminal, not_sent.cause),
            (AttemptTerminal::NotSent, Some(TerminalCause::ConnectFailed))
        );
        // A failed write on a reused connection may have reached the upstream.
        let reused = record(&[
            HttpEvent::UploadEnded,
            HttpEvent::SendFailed(SendFailure::SendFailed),
        ]);
        assert_eq!(
            (reused.terminal, reused.cause, reused.phase),
            (
                AttemptTerminal::AcceptanceUnknown,
                Some(TerminalCause::SendFailed),
                Some(AttemptPhase::AwaitingHead)
            )
        );
    }

    #[test]
    fn a_client_upload_abort_is_cancelled_by_the_client_during_upload() {
        let aborted = record(&[HttpEvent::UploadFailed]);
        assert_eq!(
            aborted,
            TerminalRecord {
                terminal: AttemptTerminal::Cancelled,
                cause: Some(TerminalCause::Client),
                phase: Some(AttemptPhase::Upload),
            }
        );
        // The send error that follows the abort is a late event, not a second end.
        let state = HttpAttempt::DISPATCHED
            .step(HttpEvent::UploadFailed)
            .unwrap()
            .state;
        let late = HttpEvent::SendFailed(SendFailure::SendFailed);
        assert_eq!(
            state.step(late),
            Err(IllegalTransition::AfterEnd { event: late })
        );
    }

    #[test]
    fn events_end_with_their_terminal_cause_and_phase() {
        let ok = HttpEvent::Responded { status: 200 };
        let cases = [
            (
                "a header timeout after a full upload",
                vec![
                    HttpEvent::UploadEnded,
                    HttpEvent::SendFailed(SendFailure::HeaderTimeout),
                ],
                AttemptTerminal::AcceptanceUnknown,
                Some(TerminalCause::HeaderTimeout),
                AttemptPhase::AwaitingHead,
            ),
            (
                "an idle stream",
                vec![ok, HttpEvent::BodyFailed(StreamFailure::IdleTimeout)],
                AttemptTerminal::StreamFailed,
                Some(TerminalCause::IdleTimeout),
                AttemptPhase::Response,
            ),
            (
                "a broken stream",
                vec![ok, HttpEvent::BodyFailed(StreamFailure::BodyError)],
                AttemptTerminal::StreamFailed,
                Some(TerminalCause::BodyError),
                AttemptPhase::Response,
            ),
            (
                "a delivered declared length",
                vec![ok, HttpEvent::LengthDelivered],
                AttemptTerminal::BodyComplete,
                Some(TerminalCause::DeclaredLength),
                AttemptPhase::Response,
            ),
            (
                "shutdown during upload",
                vec![HttpEvent::Cancelled(Canceller::Shutdown)],
                AttemptTerminal::Cancelled,
                Some(TerminalCause::Shutdown),
                AttemptPhase::Upload,
            ),
            (
                "a drop awaiting the head",
                vec![HttpEvent::UploadEnded, HttpEvent::Dropped],
                AttemptTerminal::Interrupted,
                None,
                AttemptPhase::AwaitingHead,
            ),
            (
                "shutdown during the response",
                vec![ok, HttpEvent::Cancelled(Canceller::Shutdown)],
                AttemptTerminal::Cancelled,
                Some(TerminalCause::Shutdown),
                AttemptPhase::Response,
            ),
            (
                "a drop during the response",
                vec![ok, HttpEvent::Dropped],
                AttemptTerminal::Interrupted,
                None,
                AttemptPhase::Response,
            ),
        ];
        for (case, events, terminal, cause, phase) in cases {
            assert_eq!(
                record(&events),
                TerminalRecord {
                    terminal,
                    cause,
                    phase: Some(phase),
                },
                "{case}"
            );
        }
    }

    #[test]
    fn the_upload_does_not_decide_the_outcome_after_the_head() {
        let after_head = HttpAttempt::DISPATCHED
            .step(HttpEvent::Responded { status: 200 })
            .unwrap()
            .state;
        let transition = after_head.step(HttpEvent::UploadFailed).unwrap();
        assert!(transition.actions.is_empty());
        assert_eq!(transition.state, HttpAttempt::Streaming { status: 200 });
    }

    #[test]
    fn illegal_transitions_are_typed_errors() {
        assert_eq!(
            HttpAttempt::DISPATCHED.step(HttpEvent::BodyEnded),
            Err(IllegalTransition::BodyBeforeHead {
                event: HttpEvent::BodyEnded
            })
        );
        assert_eq!(
            HttpAttempt::DISPATCHED.step(HttpEvent::LengthDelivered),
            Err(IllegalTransition::BodyBeforeHead {
                event: HttpEvent::LengthDelivered
            })
        );
        let streaming = HttpAttempt::DISPATCHED
            .step(HttpEvent::Responded { status: 200 })
            .unwrap()
            .state;
        let again = HttpEvent::Responded { status: 500 };
        assert_eq!(
            streaming.step(again),
            Err(IllegalTransition::DispatchAfterHead { event: again })
        );
        let ended = streaming.step(HttpEvent::BodyEnded).unwrap().state;
        assert_eq!(
            ended.step(HttpEvent::Dropped),
            Err(IllegalTransition::AfterEnd {
                event: HttpEvent::Dropped
            })
        );
    }
}
