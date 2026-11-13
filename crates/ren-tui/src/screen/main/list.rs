//! The session list: every session worth a look, grouped by the folder a
//! run works in, or by what each needs from you. By folder, the folders
//! with a run that needs you come first, and each lists its runs by what
//! they need, then the newest; sessions started outside Ren, whose folder
//! Ren does not know, follow. By state: runs that need you, working, your
//! turn, then sessions watched, then the newest runs that ended.

use super::recording::RecordingPane;
use crate::model::Model;
use crate::model::desk::{Attention, Run};
use crate::ui::{Ink, Line, Span, Text, Tone};
use ren_harness::attribution::HintSource;
use ren_protocol::label::Labeled;
use ren_recorder::id::{RecordingId, RunId};
use ren_recorder::time::UnixMillis;

/// One entry of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Item {
    /// A run Ren drives, live or ended.
    Run(RunId),
    /// A session started outside Ren, watched.
    Recording(RecordingId),
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

/// How the list is grouped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Grouping {
    /// By the folder each run works in.
    #[default]
    Folder,
    /// By what each needs from you.
    State,
}

impl Grouping {
    /// The other grouping.
    pub fn other(self) -> Self {
        match self {
            Self::Folder => Self::State,
            Self::State => Self::Folder,
        }
    }
}

/// The heading a row is listed under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Heading {
    State(Group),
    /// A folder runs work in, by its path.
    Folder(String),
    /// Recordings started outside Ren: their folder is not known.
    Elsewhere,
}

/// The list as read now, filtered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listing {
    pub rows: Vec<(Heading, Item)>,
    grouping: Grouping,
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

    /// Where the group comes in the list, and a run's place in its folder.
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
    /// How many of them a folder shows.
    const ENDED_PER_FOLDER: usize = 3;

    /// Every entry, filtered by `filter` (by name, project, or title),
    /// grouped `by` folder or state.
    pub fn of(model: &Model, filter: &str, by: Grouping) -> Self {
        let filter = filter.to_lowercase();
        let matches = |words: &[&str]| {
            filter.is_empty()
                || words
                    .iter()
                    .any(|word| word.to_lowercase().contains(&filter))
        };
        // Runs, with the newest ended ones only.
        let mut runs: Vec<&Run> = Vec::new();
        let mut ended = 0;
        for run in model.desk.all() {
            if Group::of(run) == Group::Ended {
                if ended == Self::ENDED {
                    continue;
                }
                ended += 1;
            }
            if matches(&[run.project(), &run.workdir, &run.title(), &run.client()]) {
                runs.push(run);
            }
        }
        let threads = model.desk.threads();
        let watched: Vec<Item> = model
            .recordings
            .active(&threads)
            .into_iter()
            // A session a run drives is listed as the run.
            .filter(|session| model.desk.driving(&session.native_id).is_none())
            .filter(|session| {
                matches(&[
                    &session.id.to_string(),
                    &Self::client(session.source.known()),
                ])
            })
            .map(|session| Item::Recording(session.id))
            .collect();
        let mut rows: Vec<(Heading, Item)> = match by {
            Grouping::State => {
                let mut rows: Vec<(Heading, Item)> = runs
                    .iter()
                    .map(|run| (Heading::State(Group::of(run)), Item::Run(run.id)))
                    .collect();
                rows.extend(
                    watched
                        .iter()
                        .map(|item| (Heading::State(Group::Watching), *item)),
                );
                // Ended runs last, after the sessions watched.
                rows.sort_by_key(|(heading, _)| match heading {
                    Heading::State(group) => group.rank(),
                    Heading::Folder(_) | Heading::Elsewhere => 0,
                });
                rows
            }
            Grouping::Folder => {
                // Folders by their most pressing run, then their newest.
                let scratch = model.scratch.as_deref();
                let mut folders: Vec<(&str, u8, u64)> = Vec::new();
                for run in &runs {
                    let rank = Group::of(run).rank();
                    match folders
                        .iter_mut()
                        .find(|(path, ..)| *path == run.folder(scratch))
                    {
                        Some((_, best, newest)) => {
                            *best = (*best).min(rank);
                            *newest = (*newest).max(run.seen);
                        }
                        None => folders.push((run.folder(scratch), rank, run.seen)),
                    }
                }
                folders.sort_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)));
                let mut rows = Vec::new();
                for (path, ..) in folders {
                    let mut inside: Vec<&&Run> = runs
                        .iter()
                        .filter(|run| run.folder(scratch) == path)
                        .collect();
                    inside.sort_by(|a, b| {
                        Group::of(a)
                            .rank()
                            .cmp(&Group::of(b).rank())
                            .then(b.seen.cmp(&a.seen))
                    });
                    // The newest few ended runs of each folder; a search
                    // or the other grouping finds the rest.
                    let mut ended = 0;
                    inside.retain(|run| {
                        if Group::of(run) != Group::Ended {
                            return true;
                        }
                        ended += 1;
                        ended <= Self::ENDED_PER_FOLDER || !filter.is_empty()
                    });
                    rows.extend(
                        inside
                            .into_iter()
                            .map(|run| (Heading::Folder(path.to_owned()), Item::Run(run.id))),
                    );
                }
                rows.extend(watched.into_iter().map(|item| (Heading::Elsewhere, item)));
                rows
            }
        };
        rows.dedup_by(|a, b| a.1 == b.1);
        Self { rows, grouping: by }
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
        let mut heading: Option<&Heading> = None;
        for (index, (row_heading, item)) in self.rows.iter().enumerate() {
            if heading != Some(row_heading) {
                heading = Some(row_heading);
                if !lines.is_empty() {
                    lines.push(Line::default());
                }
                lines.push(Self::heading(model, row_heading, width));
            }
            let (mark, mut name, mut doing, tone) = Self::describe(model, *item, now);
            // Under its folder, a run is named by what it was asked, and
            // says how it stands.
            if let (Grouping::Folder, Item::Run(id)) = (self.grouping, item)
                && let Some(run) = model.desk.get(*id)
            {
                name = Text(&run.title()).summary(width * 3 / 5);
                match run.attention() {
                    Attention::YourTurn | Attention::Ended => doing = run.phase().to_owned(),
                    Attention::NeedsYou | Attention::Working | Attention::Starting => {}
                }
            }
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

    /// A heading: a state's name, or a folder's (its last part) and where
    /// it is, with a mark when a run in it needs you.
    fn heading(model: &Model, heading: &Heading, width: usize) -> Line {
        match heading {
            Heading::State(group) => Line::of(format!(" {}", group.heading()), Ink::Muted),
            Heading::Elsewhere => Line::of(" STARTED ELSEWHERE", Ink::Muted),
            Heading::Folder(path) => {
                let scratch = model.scratch.as_deref();
                let name = if scratch == Some(path.as_str()) {
                    "Scratch"
                } else {
                    path.rsplit('/')
                        .find(|part| !part.is_empty())
                        .unwrap_or(path)
                };
                let needs = model
                    .desk
                    .all()
                    .iter()
                    .filter(|run| {
                        run.folder(scratch) == path.as_str()
                            && run.attention() == Attention::NeedsYou
                    })
                    .count();
                let mut line = Line::new(vec![
                    Span::new(format!(" {name}"), Ink::Text).with(crate::ui::Emphasis::Bold),
                ]);
                if needs > 0 {
                    line.push(Span::new(format!(" ◆{needs}"), Ink::Warning));
                }
                let room = width.saturating_sub(line.width() + 2);
                if room > 6 {
                    let parent = path.strip_suffix(name).unwrap_or(path);
                    line.push(Span::new(
                        format!("  {}", Text(parent).fit(room)),
                        Ink::Muted,
                    ));
                }
                line
            }
        }
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
            Item::Recording(id) => {
                let session = model
                    .recordings
                    .recordings()
                    .iter()
                    .find(|view| view.id == id);
                let Some(session) = session else {
                    return ('·', "session".to_owned(), String::new(), Tone::Neutral);
                };
                let pane = RecordingPane::of(model, session, now);
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
