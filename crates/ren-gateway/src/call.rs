//! A call's way through its planned candidates: which candidate the next
//! attempt uses, and whether an attempt's outcome falls over to the next one,
//! goes to the client, or ends the call.
//!
//! Each attempt's own lifecycle is [`HttpAttempt`]; this machine sits above
//! it. Every decision here is made before anything of a response reached the
//! client, so falling over never repeats a delivered response, and only
//! [`RetryPolicy`] decides which outcomes may fall over.
//!
//! [`HttpAttempt`]: crate::attempt::HttpAttempt

use crate::attempt::SendFailure;
use crate::retry::{Delivery, RetryDecision, RetryPolicy, WireFact};

/// Where a call is among its candidates. A step consumes it, so a stale
/// position cannot be reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct CallRun {
    candidate: usize,
    candidates: usize,
}

/// How one attempt's dispatch ended, before anything was relayed. `R` is the
/// response the shell holds.
#[derive(Debug, Clone)]
pub enum Dispatched<R> {
    /// Response headers arrived.
    Responded { status: u16, response: R },
    /// Nothing arrived: the send failed or timed out.
    Failed(SendFailure),
    /// The daemon is shutting down.
    Aborted,
    /// The operator cancelled the call.
    Cancelled,
    /// The attempt already ended (the client went away during upload).
    Ended,
}

/// What the shell does after an attempt.
#[derive(Debug)]
#[must_use]
pub enum CallStep<R> {
    /// The attempt was refused: record it, and start `next`.
    FallOver { refused: Refused<R>, next: CallRun },
    /// Relay this response to the client.
    Relay(R),
    /// End the call with this attempt's outcome.
    Stop(CallStop),
}

/// An attempt that falls over, as the shell records it.
#[derive(Debug)]
pub enum Refused<R> {
    /// A response the policy falls over on: drained, never relayed.
    Response(R),
    /// A send that provably never reached the upstream.
    Failure(SendFailure),
}

/// Why a call ends without relaying a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStop {
    /// The last attempt's send failed, and nothing may follow it.
    Failed(SendFailure),
    Shutdown,
    Operator,
    /// The attempt already ended on its own.
    Ended,
}

impl CallRun {
    /// A call about to make its first attempt, with `candidates` planned.
    pub fn new(candidates: usize) -> Self {
        Self {
            candidate: 0,
            candidates,
        }
    }

    /// The candidate the current attempt uses.
    pub fn candidate(&self) -> usize {
        self.candidate
    }

    /// The next step after the current attempt's dispatch ended.
    pub fn step<R>(self, dispatch: Dispatched<R>, retry: &RetryPolicy) -> CallStep<R> {
        let next = Self {
            candidate: self.candidate.saturating_add(1),
            ..self
        };
        let falls_over = |wire: WireFact| {
            next.candidate < self.candidates
                && retry.eligible(wire, Delivery::Nothing) == RetryDecision::NextCandidate
        };
        match dispatch {
            Dispatched::Responded { status, response } => {
                if falls_over(WireFact::Responded { status }) {
                    CallStep::FallOver {
                        refused: Refused::Response(response),
                        next,
                    }
                } else {
                    CallStep::Relay(response)
                }
            }
            Dispatched::Failed(failure) => {
                if falls_over(WireFact::from(failure)) {
                    CallStep::FallOver {
                        refused: Refused::Failure(failure),
                        next,
                    }
                } else {
                    CallStep::Stop(CallStop::Failed(failure))
                }
            }
            Dispatched::Aborted => CallStep::Stop(CallStop::Shutdown),
            Dispatched::Cancelled => CallStep::Stop(CallStop::Operator),
            Dispatched::Ended => CallStep::Stop(CallStop::Ended),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// A response, by its status, as the shell would hold it.
    type Held = u16;

    fn responded(status: u16) -> Dispatched<Held> {
        Dispatched::Responded {
            status,
            response: status,
        }
    }

    #[test]
    fn a_policy_status_falls_over_until_the_last_candidate() {
        let policy = RetryPolicy::falling_over_on(vec![429]);
        let first = CallRun::new(2);
        let CallStep::FallOver {
            refused: Refused::Response(429),
            next,
        } = first.step(responded(429), &policy)
        else {
            panic!("the first 429 falls over");
        };
        assert_eq!(next.candidate(), 1);
        // The last candidate's 429 goes to the client: nothing follows it.
        assert!(matches!(
            next.step(responded(429), &policy),
            CallStep::Relay(429)
        ));
        // Statuses the policy does not name pass through at once.
        assert!(matches!(
            CallRun::new(2).step(responded(500), &policy),
            CallStep::Relay(500)
        ));
        // Without a policy, every status passes through.
        assert!(matches!(
            CallRun::new(2).step(responded(429), &RetryPolicy::default()),
            CallStep::Relay(429)
        ));
    }

    #[test]
    fn only_sends_that_never_reached_the_upstream_fall_over() {
        let policy = RetryPolicy::default();
        for failure in [SendFailure::NotSent, SendFailure::Refused] {
            assert!(matches!(
                CallRun::new(2).step(Dispatched::<Held>::Failed(failure), &policy),
                CallStep::FallOver {
                    refused: Refused::Failure(_),
                    ..
                }
            ));
            assert!(matches!(
                CallRun::new(1).step(Dispatched::<Held>::Failed(failure), &policy),
                CallStep::Stop(CallStop::Failed(_))
            ));
        }
        // A send that may have been accepted is never repeated.
        for failure in [SendFailure::SendFailed, SendFailure::HeaderTimeout] {
            assert!(matches!(
                CallRun::new(3).step(Dispatched::<Held>::Failed(failure), &policy),
                CallStep::Stop(CallStop::Failed(stopped)) if stopped == failure
            ));
        }
    }

    #[test]
    fn shutdown_cancellation_and_an_ended_attempt_stop_the_call() {
        let policy = RetryPolicy::falling_over_on(vec![429]);
        for (dispatch, stop) in [
            (Dispatched::<Held>::Aborted, CallStop::Shutdown),
            (Dispatched::Cancelled, CallStop::Operator),
            (Dispatched::Ended, CallStop::Ended),
        ] {
            assert!(matches!(
                CallRun::new(3).step(dispatch, &policy),
                CallStep::Stop(found) if found == stop
            ));
        }
    }

    fn dispatches() -> impl Strategy<Value = Dispatched<Held>> {
        prop_oneof![
            prop::sample::select(vec![200_u16, 429, 500, 503, 529]).prop_map(responded),
            prop::sample::select(vec![
                SendFailure::NotSent,
                SendFailure::Refused,
                SendFailure::SendFailed,
                SendFailure::HeaderTimeout,
            ])
            .prop_map(Dispatched::Failed),
            Just(Dispatched::Aborted),
            Just(Dispatched::Cancelled),
            Just(Dispatched::Ended),
        ]
    }

    proptest! {
        /// Whatever the attempts' outcomes, a call makes at most one attempt
        /// per candidate, uses them in order, and ends by relaying or
        /// stopping.
        #[test]
        fn a_call_never_outruns_its_candidates(
            candidates in 1..5_usize,
            outcomes in prop::collection::vec(dispatches(), 1..12),
        ) {
            let policy = RetryPolicy::falling_over_on(vec![429, 503, 529]);
            let mut run = CallRun::new(candidates);
            let mut attempts = 1;
            for dispatch in outcomes {
                prop_assert_eq!(run.candidate(), attempts - 1);
                match run.step(dispatch, &policy) {
                    CallStep::FallOver { next, .. } => {
                        attempts += 1;
                        prop_assert!(attempts <= candidates);
                        run = next;
                    }
                    CallStep::Relay(_) | CallStep::Stop(_) => break,
                }
            }
        }
    }
}
