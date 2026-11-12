//! The session list: every session worth a look, grouped by what it needs
//! from you, in the order they are visited. Runs Ren drives come first, by
//! attention; then sessions started outside Ren, watched; then the newest
//! runs that ended.

use super::watched::SessionPane;
use crate::model::Model;
use crate::model::desk::{Attention, Run};
use crate::ui::{Ink, Line, Span, Text, Tone};
use ren_harness::attribution::HintSource;
use ren_protocol::label::Labeled;
use ren_recorder::id::{RunId, SessionId};
use ren_recorder::time::UnixMillis;

/// One entry of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Item {
    /// A run Ren drives, live or ended.
    Run(RunId),
    /// A session started outside Ren, watched.
    Watched(SessionId),
}

/// A group of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    NeedsYou,
    Working,
    YourTurn,
    Watching,
    Ended,
}

/// The list as read now, filtered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listing {
    pub rows: Vec<(Group, Item)>,
}

impl Group {
    fn of(run: &Run) -> Self {
        match run.attention() {
            Attention::NeedsYou => Self::NeedsYou,
            Attention::Working | Attention::Starting => Self::Working,
            Attention::YourTurn => Self::YourTurn,
            Attention::Ended => Self::Ended,
        }
    }

    /// Where the group comes in the list.
    fn rank(self) -> u8 {
        match self {
            Self::NeedsYou => 0,
            Self::Working => 1,
            Self::YourTurn => 2,
            Self::Watching => 3,
            Self::Ended => 4,
        }
    }

    fn heading(self) -> &'static str {
        match self {
            Self::NeedsYou => "NEEDS YOU",
            Self::Working => "WORKING",
            Self::YourTurn => "YOUR TURN",
            Self::Watching => "WATCHING",
            Self::Ended => "ENDED",
        }
    }
}

impl Listing {
    /// How many ended runs the list keeps.
    const ENDED: usize = 8;

    /// Every entry, filtered by `filter` (by name, project, or title).
    pub fn of(model: &Model, filter: &str) -> Self {
        let filter = filter.to_lowercase();
        let matches = |words: &[&str]| {
            filter.is_empty()
                || words
                    .iter()
                    .any(|word| word.to_lowercase().contains(&filter))
        };
        let mut rows: Vec<(Group, Item)> = Vec::new();
        let mut ended = 0;
        for run in model.desk.all() {
            let group = Group::of(run);
            if group == Group::Ended {
                if ended == Self::ENDED {
                    continue;
                }
                ended += 1;
            }
            if matches(&[run.project(), &run.title(), &run.client()]) {
                rows.push((group, Item::Run(run.id)));
            }
        }
        let threads = model.desk.threads();
        for session in model.sessions.active(&threads) {
            // A session a run drives is listed as the run.
            if model.desk.driving(&session.native_id).is_some() {
                continue;
            }
            let id = session.id.to_string();
            let client = Self::client(session.source.known());
            if matches(&[&id, &client]) {
                rows.push((Group::Watching, Item::Watched(session.id)));
            }
        }
        // Ended runs last, after the sessions watched.
        rows.sort_by_key(|(group, _)| group.rank());
        Self { rows }
    }

    fn client(source: Option<HintSource>) -> String {
        source.map_or_else(
            || "session".to_owned(),
            |source| source.harness().label().replace('_', "-"),
        )
    }

    pub fn items(&self) -> Vec<Item> {
        self.rows.iter().map(|(_, item)| *item).collect()
    }

    pub fn position(&self, item: Item) -> Option<usize> {
        self.rows.iter().position(|(_, row)| *row == item)
    }

    /// The list, `width` cells wide, `selected` marked.
    pub fn lines(
        &self,
        model: &Model,
        selected: Option<Item>,
        width: usize,
        now: UnixMillis,
    ) -> Vec<Line> {
        let mut lines = Vec::new();
        let mut group = None;
        for (index, (row_group, item)) in self.rows.iter().enumerate() {
            if group != Some(*row_group) {
                group = Some(*row_group);
                if !lines.is_empty() {
                    lines.push(Line::default());
                }
                lines.push(Line::of(format!(" {}", row_group.heading()), Ink::Muted));
            }
            let (mark, name, doing, tone) = Self::describe(model, *item, now);
            let number = match index {
                0..9 => format!("{} ", index + 1),
                _ => "  ".to_owned(),
            };
            let chosen = selected == Some(*item);
            let name_ink = if chosen { Ink::Focus } else { Ink::Text };
            let room = width.saturating_sub(4 + Text(&name).width() + 2);
            let mut line = Line::new(vec![
                Span::new(format!(" {number}"), Ink::Muted),
                Span::new(format!("{mark} "), Ink::of(tone)),
                Span::new(name, name_ink),
            ]);
            if room > 4 && !doing.is_empty() {
                line.push(Span::new(
                    format!("  {}", Text(&doing).fit(room)),
                    Ink::Muted,
                ));
            }
            lines.push(line);
        }
        if self.rows.is_empty() {
            lines.push(Line::default());
            lines.push(Line::of(" nothing yet · n starts a run", Ink::Muted));
        }
        lines
    }

    /// An entry's mark, name, what it is doing, and its tone.
    pub fn describe(model: &Model, item: Item, now: UnixMillis) -> (char, String, String, Tone) {
        match item {
            Item::Run(id) => {
                let Some(run) = model.desk.get(id) else {
                    return ('·', "run".to_owned(), String::new(), Tone::Neutral);
                };
                let name = Self::run_name(model, run);
                let (mark, tone, doing) = match run.attention() {
                    Attention::NeedsYou => (
                        '◆',
                        Tone::Warning,
                        format!(
                            "wants {}",
                            Run::asked(run.waiting().map(|waiting| &waiting.ask))
                        ),
                    ),
                    Attention::Working => (
                        '●',
                        Tone::Live,
                        run.transcript
                            .doing()
                            .unwrap_or_else(|| "working".to_owned()),
                    ),
                    Attention::Starting => ('●', Tone::Live, "starting".to_owned()),
                    Attention::YourTurn => ('○', Tone::Success, run.title()),
                    Attention::Ended => ('·', Tone::Neutral, run.title()),
                };
                (mark, name, doing, tone)
            }
            Item::Watched(id) => {
                let session = model.sessions.sessions().iter().find(|view| view.id == id);
                let Some(session) = session else {
                    return ('·', "session".to_owned(), String::new(), Tone::Neutral);
                };
                let pane = SessionPane::of(model, session, now);
                let short: String = id.to_string().chars().take(6).collect();
                let name = format!("{} {short}", Self::client(session.source.known()));
                let state = pane.state_line(now).text();
                let tone = pane
                    .alerts
                    .first()
                    .map_or(Tone::Neutral, |alert| alert.tone);
                ('·', name, state, tone)
            }
        }
    }

    /// A run's name: its project, numbered when several runs share it, by
    /// when each was first seen, so each keeps its number.
    pub fn run_name(model: &Model, run: &Run) -> String {
        let project = run.project();
        let earlier = model
            .desk
            .all()
            .iter()
            .filter(|other| other.project() == project && other.seen < run.seen)
            .count();
        match earlier {
            0 => project.to_owned(),
            earlier => format!("{project} {}", earlier + 1),
        }
    }
}
