//! Work items, pure: what an operator marked to be done (a [`WorkOrder`]:
//! the page it was marked on, its marks, and what was on screen besides),
//! and the lifecycle that carries it to production ([`WorkState`]): queued,
//! sent to a session that works on it in a worktree of its own, landed on
//! `main` as a commit, shipped in the build production runs, or dismissed.
//! Transitions are a pure function of the state and an event; an event the
//! state does not take is a typed error, never a silent no-op.

use crate::id::{SessionId, WorkItemId};
use ren_git::Commit;
use ren_release::Revision;
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

/// A preview of the change a session is making: its branch's build on a
/// copy of production's history, where the marked page can be opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkPreview {
    /// The preview's UI.
    pub ui: PreviewUrl,
    /// The build it runs.
    pub revision: Revision,
}

/// An `http` or `https` origin a preview's UI is reached at, such as
/// `https://ren.example.ts.net:20201`: no path, no spaces.
#[derive(Debug, Clone, PartialEq, Eq, SerializeDisplay, DeserializeFromStr)]
pub struct PreviewUrl(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a preview's origin (`http(s)://host[:port]`): {0:?}")]
pub struct NotAPreviewUrl(pub String);

impl std::str::FromStr for PreviewUrl {
    type Err = NotAPreviewUrl;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refused = || NotAPreviewUrl(text.to_owned());
        let rest = text
            .strip_prefix("https://")
            .or_else(|| text.strip_prefix("http://"))
            .ok_or_else(refused)?;
        let host = rest.strip_suffix('/').unwrap_or(rest);
        let fine = !host.is_empty()
            && host.len() <= 255
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".-:[]".contains(&byte));
        if fine {
            Ok(Self(text.trim_end_matches('/').to_owned()))
        } else {
            Err(refused())
        }
    }
}

impl PreviewUrl {
    /// Its port, when it names one.
    pub fn port(&self) -> Option<u16> {
        let host = self
            .0
            .split_once("://")
            .map_or(self.0.as_str(), |(_, host)| host);
        let (_, port) = host.rsplit_once(':')?;
        port.parse().ok()
    }
}

impl std::fmt::Display for PreviewUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A commit on `main` and the work items it names as landing: each a
/// `Work-Item: <id>` trailer in its message, as a brief asks an agent to
/// write in its commits and its pull request's description, so a squash
/// merge keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Landing {
    pub commit: Commit,
    pub items: Vec<WorkItemId>,
}

impl Landing {
    /// The trailer's key.
    pub const TRAILER: &str = "Work-Item";

    /// The items `message` names, once each, in the order named.
    pub fn read(commit: Commit, message: &str) -> Self {
        Self {
            commit,
            items: Self::named([message], &[]),
        }
    }

    /// The items `given`, then those `messages` name, once each, in that
    /// order: what a branch's commits address. A line is a trailer when it
    /// is the key, a colon, and ids, wherever it is.
    pub fn named<'a>(
        messages: impl IntoIterator<Item = &'a str>,
        given: &[WorkItemId],
    ) -> Vec<WorkItemId> {
        let mut items = given.to_vec();
        for line in messages.into_iter().flat_map(str::lines) {
            let Some((key, value)) = line.trim().split_once(':') else {
                continue;
            };
            if !key.trim().eq_ignore_ascii_case(Self::TRAILER) {
                continue;
            }
            for id in value.split([',', ' ']).filter(|part| !part.is_empty()) {
                if let Ok(id) = id.trim().trim_matches('`').parse::<WorkItemId>()
                    && !items.contains(&id)
                {
                    items.push(id);
                }
            }
        }
        items
    }
}

/// Where a work item stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkState {
    /// Marked, waiting to be sent; queued items can be sent together.
    Queued,
    /// A session works on it, in a worktree of its own.
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
    /// A preview of its change, or none any longer. Only a sent item has
    /// one; it leaves the state as it is.
    Preview,
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
            Self::Preview => "previewed",
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
            (Self::Sent { .. }, WorkEvent::Preview) => Ok(self.clone()),
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
            )
            | (
                Self::Queued | Self::Landed { .. } | Self::Shipped { .. } | Self::Dismissed,
                event @ WorkEvent::Preview,
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
    fn a_fact_moves_from_exactly_the_states_its_event_does() {
        use crate::batch::WorkFact;
        let commit: ren_git::Commit = COMMIT.parse().unwrap();
        let landed = WorkState::Landed {
            session: session(1),
            commit: commit.clone(),
        };
        let states = [
            WorkState::Queued,
            WorkState::Sent {
                session: session(1),
            },
            landed.clone(),
            landed
                .apply(WorkEvent::Ship {
                    revision: COMMIT.parse().unwrap(),
                })
                .unwrap(),
            WorkState::Dismissed,
        ];
        let pairs = [
            (
                WorkEvent::Send {
                    session: session(2),
                },
                WorkFact::Sent {
                    session: session(2),
                },
            ),
            (
                WorkEvent::Land {
                    commit: commit.clone(),
                },
                WorkFact::Landed { commit },
            ),
            (
                WorkEvent::Ship {
                    revision: COMMIT.parse().unwrap(),
                },
                WorkFact::Shipped {
                    revision: COMMIT.parse().unwrap(),
                },
            ),
            (WorkEvent::Dismiss, WorkFact::Dismissed),
            (WorkEvent::Preview, WorkFact::Previewed { preview: None }),
        ];
        for (event, fact) in pairs {
            for state in &states {
                assert_eq!(
                    state.apply(event.clone()).is_ok(),
                    fact.from_states().contains(&state.kind()),
                    "{fact:?} from {state:?}"
                );
            }
        }
    }

    #[test]
    fn a_preview_is_an_http_origin() {
        use super::PreviewUrl;
        for text in [
            "https://ren.example.ts.net:20201",
            "http://127.0.0.1:20201/",
            "http://[::1]:8",
        ] {
            assert!(text.parse::<PreviewUrl>().is_ok(), "{text}");
        }
        assert_eq!(
            "http://127.0.0.1:20201/"
                .parse::<PreviewUrl>()
                .unwrap()
                .to_string(),
            "http://127.0.0.1:20201"
        );
        let port = |text: &str| text.parse::<PreviewUrl>().unwrap().port();
        assert_eq!(port("https://ren.example.ts.net:20201"), Some(20_201));
        assert_eq!(port("http://[::1]:8"), Some(8));
        assert_eq!(port("https://ren.example.ts.net"), None);
        for text in [
            "",
            "ftp://x",
            "https://",
            "https://x/path",
            "https://a b",
            "javascript:alert(1)",
        ] {
            assert!(text.parse::<PreviewUrl>().is_err(), "{text}");
        }
    }

    #[test]
    fn a_commit_names_the_work_items_it_lands_in_its_trailers() {
        use super::Landing;
        use crate::id::WorkItemId;
        let a = "0193e0f1-0000-7000-8000-000000000001";
        let b = "0193e0f1-0000-7000-8000-000000000002";
        let message = format!(
            "feat: show turn counts (#70)\n\nBody text.\n\nWork-Item: {a}\nwork-item: `{b}`, {a}\nWork-Item: not-an-id\nCo-Authored-By: Someone <x@example.com>\n"
        );
        let landing = Landing::read(COMMIT.parse().unwrap(), &message);
        let ids: Vec<WorkItemId> = [a, b].iter().map(|id| id.parse().unwrap()).collect();
        assert_eq!(landing.items, ids);
        assert!(
            Landing::read(COMMIT.parse().unwrap(), "fix: nothing named")
                .items
                .is_empty()
        );
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
        // Only a sent item has a preview, and it keeps its state.
        let sent = WorkState::Sent {
            session: session(1),
        };
        assert_eq!(sent.apply(WorkEvent::Preview).unwrap(), sent);
        assert!(WorkState::Queued.apply(WorkEvent::Preview).is_err());
        assert_eq!(
            WorkState::Dismissed
                .apply(WorkEvent::Dismiss)
                .unwrap_err()
                .to_string(),
            "a dismissed work item cannot be dismissed"
        );
    }
}
