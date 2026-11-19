//! Work items, pure: what an operator marked to be done (a [`WorkOrder`]:
//! the page it was marked on, its marks, and what was on screen besides),
//! and the lifecycle that carries it to production ([`WorkState`]): queued,
//! sent to a session that works on it in a workspace of its own, landed on
//! `main` as a commit, shipped in the build production runs, or dismissed.
//! Transitions are a pure function of the state and an event; an event the
//! state does not take is a typed error, never a silent no-op.

use crate::id::SessionId;
use ren_release::{Commit, Revision};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// What was marked to be done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkOrder {
    pub page: MarkedPage,
    pub marks: Vec<WorkMark>,
    #[serde(default)]
    pub evidence: Evidence,
}

/// The page the marks were made on, as the browser showed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkedPage {
    /// Its path and query, as the address bar showed them.
    pub path: String,
    /// The UI's route id (`/recordings/[id]`), when it has one.
    pub route: Option<String>,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub theme: PageTheme,
    /// What the daemon serving the page was built from, as `/health` said.
    pub revision: Option<Revision>,
}

/// The page's color theme.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum PageTheme {
    Light,
    Dark,
}

/// One mark: what it points at, and what the operator wrote about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkMark {
    /// A CSS selector from the nearest labelled region down to the element.
    pub selector: String,
    /// The element's own text, cut short.
    pub text: String,
    /// The source file and line a development build names, when known.
    pub source: Option<String>,
    pub width: u32,
    pub height: u32,
    pub note: String,
}

/// What else was on screen: the errors the page logged, and the API reads
/// behind what it showed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// Errors the page logged, newest last.
    pub console: Vec<String>,
    /// The read API paths the page loaded, as it requested them.
    pub reads: Vec<String>,
}

/// Why an order is not one Ren keeps.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OrderError {
    #[error("a work order needs at least one mark with a note")]
    NoNote,
    #[error("a work order holds at most {} marks", WorkOrder::MARKS)]
    TooManyMarks,
    #[error("a work order's text is past its limit")]
    TooLong,
}

impl WorkOrder {
    /// The most marks one order holds.
    pub const MARKS: usize = 20;
    /// The most console errors and reads each it keeps.
    pub const EVIDENCE: usize = 50;
    /// The most bytes any one text field holds.
    pub const TEXT: usize = 4096;

    /// The order as Ren keeps it: every mark noted, within its limits; the
    /// evidence keeps its newest entries, each cut to the limit.
    pub fn checked(mut self) -> Result<Self, OrderError> {
        self.marks.retain(|mark| !mark.note.trim().is_empty());
        if self.marks.is_empty() {
            return Err(OrderError::NoNote);
        }
        if self.marks.len() > Self::MARKS {
            return Err(OrderError::TooManyMarks);
        }
        let long = |text: &str| text.len() > Self::TEXT;
        let page = &self.page;
        if long(&page.path)
            || long(&page.title)
            || page.route.as_deref().is_some_and(long)
            || self.marks.iter().any(|mark| {
                long(&mark.selector)
                    || long(&mark.text)
                    || long(&mark.note)
                    || mark.source.as_deref().is_some_and(long)
            })
        {
            return Err(OrderError::TooLong);
        }
        for list in [&mut self.evidence.console, &mut self.evidence.reads] {
            let excess = list.len().saturating_sub(Self::EVIDENCE);
            list.drain(..excess);
            for entry in list.iter_mut() {
                if entry.len() > Self::TEXT {
                    let mut end = Self::TEXT;
                    while !entry.is_char_boundary(end) {
                        end -= 1;
                    }
                    entry.truncate(end);
                }
            }
        }
        Ok(self)
    }

    /// What the order is called: its first note's first line.
    pub fn title(&self) -> &str {
        self.marks
            .first()
            .and_then(|mark| mark.note.trim().lines().next())
            .unwrap_or("Work")
    }
}

/// Where a work item stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkState {
    /// Marked, waiting to be sent; queued items can be sent together.
    Queued,
    /// A session works on it, in a workspace of its own.
    Sent { session: SessionId },
    /// Its change is on `main`, as this commit.
    Landed { session: SessionId, commit: Commit },
    /// Production runs a build with its change. It stays so.
    Shipped {
        session: SessionId,
        commit: Commit,
        revision: Revision,
    },
    /// Not to be done. It stays so.
    Dismissed,
}

/// What happens to a work item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkEvent {
    /// A session takes it up. A sent item can be sent again, to another
    /// session, as when the first was deleted.
    Send {
        session: SessionId,
    },
    Land {
        commit: Commit,
    },
    Ship {
        revision: Revision,
    },
    Dismiss,
}

/// A work item's state, as a label.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum WorkStateKind {
    Queued,
    Sent,
    Landed,
    Shipped,
    Dismissed,
}

/// An event a work item's state does not take.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a {state} work item cannot be {event}")]
pub struct WorkTransitionError {
    pub state: WorkStateKind,
    pub event: &'static str,
}

impl WorkEvent {
    fn name(&self) -> &'static str {
        match self {
            Self::Send { .. } => "sent",
            Self::Land { .. } => "landed",
            Self::Ship { .. } => "shipped",
            Self::Dismiss => "dismissed",
        }
    }
}

impl WorkState {
    pub fn kind(&self) -> WorkStateKind {
        match self {
            Self::Queued => WorkStateKind::Queued,
            Self::Sent { .. } => WorkStateKind::Sent,
            Self::Landed { .. } => WorkStateKind::Landed,
            Self::Shipped { .. } => WorkStateKind::Shipped,
            Self::Dismissed => WorkStateKind::Dismissed,
        }
    }

    /// The session that works on it, once sent.
    pub fn session(&self) -> Option<&SessionId> {
        match self {
            Self::Queued | Self::Dismissed => None,
            Self::Sent { session }
            | Self::Landed { session, .. }
            | Self::Shipped { session, .. } => Some(session),
        }
    }

    /// The state after `event`.
    pub fn apply(&self, event: WorkEvent) -> Result<Self, WorkTransitionError> {
        let refused = |event: &WorkEvent| WorkTransitionError {
            state: self.kind(),
            event: event.name(),
        };
        match (self, event) {
            (Self::Queued | Self::Sent { .. }, WorkEvent::Send { session }) => {
                Ok(Self::Sent { session })
            }
            (Self::Sent { session }, WorkEvent::Land { commit }) => Ok(Self::Landed {
                session: *session,
                commit,
            }),
            (Self::Landed { session, commit }, WorkEvent::Ship { revision }) => Ok(Self::Shipped {
                session: *session,
                commit: commit.clone(),
                revision,
            }),
            (Self::Queued | Self::Sent { .. }, WorkEvent::Dismiss) => Ok(Self::Dismissed),
            (
                Self::Queued | Self::Landed { .. } | Self::Shipped { .. } | Self::Dismissed,
                event @ WorkEvent::Land { .. },
            )
            | (
                Self::Queued | Self::Sent { .. } | Self::Shipped { .. } | Self::Dismissed,
                event @ WorkEvent::Ship { .. },
            )
            | (
                Self::Landed { .. } | Self::Shipped { .. } | Self::Dismissed,
                event @ (WorkEvent::Send { .. } | WorkEvent::Dismiss),
            ) => Err(refused(&event)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Evidence, MarkedPage, OrderError, PageTheme, WorkEvent, WorkMark, WorkOrder, WorkState,
        WorkStateKind,
    };
    use crate::id::SessionId;

    const COMMIT: &str = "62e9e9e0000000000000000000000000000000ff";

    fn session(n: u8) -> SessionId {
        format!("00000000-0000-4000-8000-0000000000{n:02x}")
            .parse()
            .unwrap()
    }

    fn order(notes: &[&str]) -> WorkOrder {
        WorkOrder {
            page: MarkedPage {
                path: "/recordings".into(),
                route: Some("/recordings".into()),
                title: "Recordings".into(),
                width: 1440,
                height: 900,
                theme: PageTheme::Light,
                revision: Some(COMMIT.parse().unwrap()),
            },
            marks: notes
                .iter()
                .map(|note| WorkMark {
                    selector: "[aria-label=\"Recordings\"] table > tr".into(),
                    text: "acme-api".into(),
                    source: None,
                    width: 600,
                    height: 24,
                    note: (*note).into(),
                })
                .collect(),
            evidence: Evidence::default(),
        }
    }

    #[test]
    fn an_order_keeps_its_noted_marks_within_its_limits() {
        let checked = order(&["  ", "Show the turn count\nand its trend"])
            .checked()
            .unwrap();
        assert_eq!(checked.marks.len(), 1);
        assert_eq!(checked.title(), "Show the turn count");
        assert_eq!(order(&[" "]).checked(), Err(OrderError::NoNote));
        let many: Vec<&str> = vec!["note"; WorkOrder::MARKS + 1];
        assert_eq!(order(&many).checked(), Err(OrderError::TooManyMarks));
        let long = "x".repeat(WorkOrder::TEXT + 1);
        assert_eq!(order(&[&long]).checked(), Err(OrderError::TooLong));

        let mut noisy = order(&["Fix it"]);
        noisy.evidence.console = (0..60).map(|n| format!("error {n}")).collect();
        noisy.evidence.reads = vec!["é".repeat(WorkOrder::TEXT)];
        let kept = noisy.checked().unwrap();
        assert_eq!(kept.evidence.console.len(), WorkOrder::EVIDENCE);
        assert_eq!(kept.evidence.console[0], "error 10");
        assert!(kept.evidence.reads[0].len() <= WorkOrder::TEXT);
    }

    #[test]
    fn a_work_item_goes_from_queued_to_shipped() {
        let sent = WorkState::Queued
            .apply(WorkEvent::Send {
                session: session(1),
            })
            .unwrap();
        // Sent again, to another session.
        let sent = sent
            .apply(WorkEvent::Send {
                session: session(2),
            })
            .unwrap();
        assert_eq!(sent.session(), Some(&session(2)));
        let landed = sent
            .apply(WorkEvent::Land {
                commit: COMMIT.parse().unwrap(),
            })
            .unwrap();
        let shipped = landed
            .apply(WorkEvent::Ship {
                revision: COMMIT.parse().unwrap(),
            })
            .unwrap();
        assert_eq!(shipped.kind(), WorkStateKind::Shipped);
        assert_eq!(shipped.session(), Some(&session(2)));
    }

    #[test]
    fn an_event_a_state_does_not_take_is_refused() {
        let commit = || WorkEvent::Land {
            commit: COMMIT.parse().unwrap(),
        };
        let ship = || WorkEvent::Ship {
            revision: COMMIT.parse().unwrap(),
        };
        let landed = WorkState::Landed {
            session: session(1),
            commit: COMMIT.parse().unwrap(),
        };
        let shipped = landed.apply(ship()).unwrap();
        for (state, event) in [
            (WorkState::Queued, commit()),
            (WorkState::Queued, ship()),
            (
                WorkState::Sent {
                    session: session(1),
                },
                ship(),
            ),
            (landed.clone(), WorkEvent::Dismiss),
            (
                landed.clone(),
                WorkEvent::Send {
                    session: session(2),
                },
            ),
            (shipped.clone(), WorkEvent::Dismiss),
            (shipped, commit()),
            (
                WorkState::Dismissed,
                WorkEvent::Send {
                    session: session(1),
                },
            ),
        ] {
            let error = state.apply(event).unwrap_err();
            assert_eq!(error.state, state.kind());
        }
        assert_eq!(
            WorkState::Queued.apply(WorkEvent::Dismiss).unwrap(),
            WorkState::Dismissed
        );
        assert_eq!(
            WorkState::Dismissed
                .apply(WorkEvent::Dismiss)
                .unwrap_err()
                .to_string(),
            "a dismissed work item cannot be dismissed"
        );
    }
}
