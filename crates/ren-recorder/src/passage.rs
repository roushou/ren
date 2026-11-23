//! What a run reached through the relay, as recorded: one
//! [`PassageRecord`] per request it made of the relay, read once from the
//! relay's report.

use ren_relay::{Course, Refusal, TunnelEnd, Unreachable};
use serde::Serialize;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// How a request to the relay ended, as stored: a tunnel's end and, when it
/// went nowhere, why.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum PassageEnd {
    /// Open, then closed by either side.
    Closed,
    /// Open, then failed mid-way.
    Broken,
    /// Refused: no listed destination admits it.
    Unlisted,
    /// Refused: its name reached only this host or a private network.
    PrivateAddress,
    /// Admitted, but its name did not resolve.
    Unresolved,
    /// Admitted, but no address it resolved to took the connection.
    ConnectRefused,
    /// Admitted, but resolving or connecting took too long.
    TimedOut,
    /// Not a tunnel, refused as such.
    NotATunnel,
}

/// The bytes a closed tunnel passed each way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PassedBytes {
    pub sent: u64,
    pub received: u64,
}

/// One request a run made of the relay, and how it ended. Read from the
/// relay's [`Course`] alone, so its parts always agree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PassageRecord {
    asked: String,
    end: PassageEnd,
    #[serde(skip_serializing_if = "Option::is_none")]
    through: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<PassedBytes>,
}

impl PassageRecord {
    /// The record of `course`.
    pub fn of(course: &Course) -> Self {
        match course {
            Course::NotATunnel { method } => Self {
                asked: method.to_string(),
                end: PassageEnd::NotATunnel,
                through: None,
                bytes: None,
            },
            Course::Tunnel { target, end } => {
                let (end, through, bytes) = match end {
                    TunnelEnd::Refused(refusal) => (
                        match refusal {
                            Refusal::Unlisted => PassageEnd::Unlisted,
                            Refusal::PrivateAddress => PassageEnd::PrivateAddress,
                        },
                        None,
                        None,
                    ),
                    TunnelEnd::Unreachable { through, why } => (
                        match why {
                            Unreachable::Unresolved => PassageEnd::Unresolved,
                            Unreachable::Refused => PassageEnd::ConnectRefused,
                            Unreachable::TimedOut => PassageEnd::TimedOut,
                        },
                        Some(through),
                        None,
                    ),
                    TunnelEnd::Closed {
                        through,
                        sent,
                        received,
                    } => (
                        PassageEnd::Closed,
                        Some(through),
                        Some(PassedBytes {
                            sent: *sent,
                            received: *received,
                        }),
                    ),
                    TunnelEnd::Broken { through } => (PassageEnd::Broken, Some(through), None),
                };
                Self {
                    asked: target.to_string(),
                    end,
                    through: through.map(ToString::to_string),
                    bytes,
                }
            }
        }
    }

    /// What the run asked for: a tunnel's `host:port`, or the method of a
    /// request that was not one.
    pub fn asked(&self) -> &str {
        &self.asked
    }

    pub fn end(&self) -> PassageEnd {
        self.end
    }

    /// The listed destination that let it through; none when refused.
    pub fn through(&self) -> Option<&str> {
        self.through.as_deref()
    }

    /// The bytes each way, for a tunnel that closed; unknown otherwise.
    pub fn bytes(&self) -> Option<PassedBytes> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::{PassageEnd, PassageRecord, PassedBytes};
    use ren_relay::{Course, Refusal, TunnelEnd};

    #[test]
    fn a_closed_tunnel_keeps_where_it_went_and_its_bytes() {
        let course = Course::Tunnel {
            target: "index.crates.io:443".parse().unwrap(),
            end: TunnelEnd::Closed {
                through: "*.crates.io:443".parse().unwrap(),
                sent: 512,
                received: 4096,
            },
        };
        let record = PassageRecord::of(&course);
        assert_eq!(record.asked(), "index.crates.io:443");
        assert_eq!(record.end(), PassageEnd::Closed);
        assert_eq!(record.through(), Some("*.crates.io:443"));
        assert_eq!(
            record.bytes(),
            Some(PassedBytes {
                sent: 512,
                received: 4096
            })
        );
        assert_eq!(
            serde_json::to_value(&record).unwrap(),
            serde_json::json!({
                "asked": "index.crates.io:443",
                "end": "closed",
                "through": "*.crates.io:443",
                "bytes": { "sent": 512, "received": 4096 },
            })
        );
    }

    #[test]
    fn a_refused_tunnel_went_through_nothing() {
        let course = Course::Tunnel {
            target: "example.com:443".parse().unwrap(),
            end: TunnelEnd::Refused(Refusal::Unlisted),
        };
        let record = PassageRecord::of(&course);
        assert_eq!(record.end(), PassageEnd::Unlisted);
        assert_eq!(record.through(), None);
        assert_eq!(record.bytes(), None);
    }

    #[test]
    fn a_request_that_is_not_a_tunnel_keeps_its_method() {
        let course = Course::NotATunnel {
            method: "GET".into(),
        };
        let record = PassageRecord::of(&course);
        assert_eq!(record.asked(), "GET");
        assert_eq!(record.end(), PassageEnd::NotATunnel);
    }
}
