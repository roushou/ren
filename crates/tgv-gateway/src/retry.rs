//! Retry and failover eligibility, as a pure function of three separate
//! facts: what happened on the wire, the status, and whether anything reached
//! the client.
//!
//! Retries are narrow on purpose. A duplicated inference spends subscription
//! allowance, can repeat server-side tools, and can corrupt stored-response
//! state, so anything that may have been accepted, or that already reached
//! the client, is never retried.

use crate::attempt::SendFailure;

/// What happened on the wire for one attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFact {
    /// Positive evidence the upstream did not accept the request: a refused
    /// connection, or a stream refused before processing.
    NotAccepted,
    /// The request was, or may have been, sent, and no status arrived.
    AcceptanceUnknown,
    Responded {
        status: u16,
    },
}

impl From<SendFailure> for WireFact {
    fn from(failure: SendFailure) -> Self {
        match failure {
            SendFailure::NotSent | SendFailure::Refused => Self::NotAccepted,
            SendFailure::SendFailed | SendFailure::HeaderTimeout => Self::AcceptanceUnknown,
        }
    }
}

/// Whether anything of the response has reached the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Nothing,
    Started,
}

/// Which responded statuses may fall over to the next candidate. Empty means
/// statuses always pass through to the client.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct RetryPolicy {
    fall_over_on: Vec<u16>,
}

/// What to do after an attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "decision", content = "reason")]
pub enum RetryDecision {
    /// Try the next candidate, if there is one.
    NextCandidate,
    /// Keep this attempt's outcome.
    Stop(StopReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// A response the policy does not fall over on: it goes to the client.
    PassThrough,
    /// The upstream may have accepted the request.
    AcceptanceUnknown,
    /// Something already reached the client.
    Delivered,
}

impl RetryPolicy {
    /// Fall over on these statuses, such as a plan-limit `429`.
    pub fn falling_over_on(statuses: Vec<u16>) -> Self {
        Self {
            fall_over_on: statuses,
        }
    }

    pub fn eligible(&self, wire: WireFact, delivery: Delivery) -> RetryDecision {
        match (delivery, wire) {
            (
                Delivery::Started,
                WireFact::NotAccepted | WireFact::AcceptanceUnknown | WireFact::Responded { .. },
            ) => RetryDecision::Stop(StopReason::Delivered),
            (Delivery::Nothing, WireFact::AcceptanceUnknown) => {
                RetryDecision::Stop(StopReason::AcceptanceUnknown)
            }
            (Delivery::Nothing, WireFact::NotAccepted) => RetryDecision::NextCandidate,
            (Delivery::Nothing, WireFact::Responded { status }) => {
                if self.fall_over_on.contains(&status) {
                    RetryDecision::NextCandidate
                } else {
                    RetryDecision::Stop(StopReason::PassThrough)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Delivery, RetryDecision, RetryPolicy, StopReason, WireFact};
    use crate::attempt::SendFailure;

    #[test]
    fn the_d2_table_holds() {
        let default = RetryPolicy::default();
        let fallback = RetryPolicy::falling_over_on(vec![429]);
        let next = RetryDecision::NextCandidate;
        let stop = RetryDecision::Stop;
        for (policy, wire, delivery, expected) in [
            (&default, WireFact::NotAccepted, Delivery::Nothing, next),
            (
                &default,
                WireFact::Responded { status: 429 },
                Delivery::Nothing,
                stop(StopReason::PassThrough),
            ),
            (
                &default,
                WireFact::Responded { status: 503 },
                Delivery::Nothing,
                stop(StopReason::PassThrough),
            ),
            (
                &fallback,
                WireFact::Responded { status: 429 },
                Delivery::Nothing,
                next,
            ),
            (
                &fallback,
                WireFact::Responded { status: 500 },
                Delivery::Nothing,
                stop(StopReason::PassThrough),
            ),
            // Never, whatever the policy says.
            (
                &fallback,
                WireFact::AcceptanceUnknown,
                Delivery::Nothing,
                stop(StopReason::AcceptanceUnknown),
            ),
            (
                &fallback,
                WireFact::NotAccepted,
                Delivery::Started,
                stop(StopReason::Delivered),
            ),
            (
                &fallback,
                WireFact::Responded { status: 429 },
                Delivery::Started,
                stop(StopReason::Delivered),
            ),
        ] {
            assert_eq!(
                policy.eligible(wire, delivery),
                expected,
                "{wire:?} {delivery:?}"
            );
        }
    }

    #[test]
    fn only_a_refused_connection_is_not_sent() {
        assert_eq!(WireFact::from(SendFailure::NotSent), WireFact::NotAccepted);
        assert_eq!(WireFact::from(SendFailure::Refused), WireFact::NotAccepted);
        assert_eq!(
            WireFact::from(SendFailure::SendFailed),
            WireFact::AcceptanceUnknown
        );
        assert_eq!(
            WireFact::from(SendFailure::HeaderTimeout),
            WireFact::AcceptanceUnknown
        );
    }
}
