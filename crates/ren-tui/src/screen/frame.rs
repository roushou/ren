//! The frame every screen is drawn in, so the terminal reads as one app:
//! the places line at the top (the wall, then each live run, what needs
//! you lit wherever you are), the screen's body, the message box, and the
//! status line. Completions open over the body, just above the box.

use super::slash::Known;
use super::{Look, NoticeTone, Suggestion};
use crate::Size;
use crate::model::desk::Attention;
use crate::model::{Control, Projects};
use crate::ui::{Anchor, Emphasis, Ink, Keep, Line, Place, Rule, Size as Room, Span, Text, View};
use ren_recorder::id::RunId;
use ren_recorder::time::UnixMillis;

/// Where the operator is, as the places line marks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Here {
    Wall,
    Run(RunId),
    /// A session watched from the wall: how the line names it.
    Watching(String),
    Resume,
}

/// One screen in the frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    places: Line,
    counts: Line,
    body: View,
    popup: Vec<(Suggestion, bool)>,
    composer: Vec<String>,
    cursor: (usize, usize),
    /// The rest of the chosen completion, after the cursor.
    ghost: Option<String>,
    placeholder: Option<String>,
    status: Line,
    status_right: Line,
    width: u16,
}

impl Frame {
    /// The rows the frame takes with one typed line: the places line, the
    /// rule over the message box, the box, and the status line.
    pub const ROWS: u16 = 4;
    /// The most lines the message box grows to.
    const TYPED: usize = 6;

    /// The room a body has inside the frame.
    pub fn body(look: &Look<'_>) -> Size {
        let typed = u16::try_from(look.message.lines().0.len().clamp(1, Self::TYPED)).unwrap_or(1);
        Size {
            width: look.size.width,
            height: look.size.height.saturating_sub(Self::ROWS - 1 + typed),
        }
    }

    /// `body` in the frame, `here`; `status` says what the keys do or how
    /// things stand, unless a notice says more; `placeholder` fills an
    /// empty message box.
    pub fn new(look: &Look<'_>, here: &Here, body: View, status: Line, placeholder: &str) -> Self {
        let (composer, cursor) = look.message.lines();
        let (places, counts) = Self::places(look, here);
        Self {
            places,
            counts,
            body,
            popup: look.message.popup(&Known::of(look.model)),
            composer,
            cursor,
            ghost: look.message.ghost(&Known::of(look.model)),
            placeholder: look.message.is_empty().then(|| placeholder.to_owned()),
            status: match look.notice {
                Some(notice) => Line::of(
                    format!(" {}", notice.text),
                    match notice.tone {
                        NoticeTone::Info => Ink::Muted,
                        NoticeTone::Refusal => Ink::Error,
                    },
                ),
                None => status,
            },
            status_right: Line::of(format!(" /help │ {} ", Clock(look.now)), Ink::Muted),
            width: look.size.width,
        }
    }

    /// Why runs cannot be driven or started from here, if they cannot.
    pub fn trouble(look: &Look<'_>) -> Option<Line> {
        match (&look.model.control, &look.model.projects) {
            (Control::Unavailable(why), _) => Some(Line::of(
                format!(" runs cannot be driven from here: {why}"),
                Ink::Error,
            )),
            (_, Projects::Failed(why)) => Some(Line::of(
                format!(" cannot list projects: {why}"),
                Ink::Error,
            )),
            (Control::Unread | Control::Ready, Projects::Unread | Projects::Read(_)) => None,
        }
    }

    /// The places, left: the wall, then each live run, what needs you
    /// first (`◇` waits for you, `▪` the rest), the one shown reversed; the
    /// counts, right.
    fn places(look: &Look<'_>, here: &Here) -> (Line, Line) {
        let desk = &look.model.desk;
        let mut line = Line::new(vec![
            Span::new(" ren ", Ink::LiveTitle).with(Emphasis::Bold),
            Span::plain(" "),
        ]);
        let shown = |is: bool, ink: Ink| if is { Ink::Focus } else { ink };
        line.push(Span::new(" wall ", shown(*here == Here::Wall, Ink::Same)));
        match here {
            Here::Watching(name) => line.push(Span::new(format!(" › {name} "), Ink::Focus)),
            Here::Resume => line.push(Span::new(" › /resume ", Ink::Focus)),
            Here::Wall | Here::Run(_) => {}
        }
        let order = desk.ordered();
        for run in &order {
            // Runs in one project are told apart by a count, in the order
            // they were first seen, so each keeps its number.
            let project = run.project();
            let seen = order
                .iter()
                .filter(|other| other.project() == project && other.seen < run.seen)
                .count();
            let name = match seen {
                0 => project.to_owned(),
                seen => format!("{project} {}", seen + 1),
            };
            let needs = run.attention() == Attention::NeedsYou;
            let (mark, ink) = if needs {
                ("◇", Ink::Warning)
            } else {
                ("▪", Ink::Muted)
            };
            line.push(Span::plain(" "));
            line.push(Span::new(
                format!(" {mark} {name} "),
                shown(*here == Here::Run(run.id), ink),
            ));
        }
        let needing = desk.needing();
        let counts = match needing {
            0 => format!(" {} live ", order.len()),
            1 => format!(" 1 needs you · {} live ", order.len()),
            n => format!(" {n} need you · {} live ", order.len()),
        };
        let counts = Line::of(
            counts,
            if needing > 0 {
                Ink::Warning
            } else {
                Ink::Muted
            },
        );
        (line, counts)
    }

    /// What to draw.
    pub fn view(self) -> View {
        if self.width < 4 {
            return View::fill(Ink::Ground, View::Empty);
        }
        let typed = u16::try_from(self.composer.len().clamp(1, Self::TYPED)).unwrap_or(1);
        let body = self.popup(self.body.clone());
        let composer = self.composer(typed);
        View::fill(
            Ink::Ground,
            View::Rows(vec![
                (
                    Room::Exact(1),
                    View::bar(self.places, self.counts, Ink::Bar, Keep::Right),
                ),
                (Room::Fill, body),
                (Room::Exact(1), View::Rule(Rule::Plain)),
                (Room::Exact(typed), composer),
                (
                    Room::Exact(1),
                    View::bar(self.status, self.status_right, Ink::Bar, Keep::Right),
                ),
            ]),
        )
    }

    /// Completions, just above the message box.
    fn popup(&self, body: View) -> View {
        if self.popup.is_empty() {
            return body;
        }
        let shown = self.popup.len().min(8);
        let label_width = self
            .popup
            .iter()
            .map(|(suggestion, _)| Text(&suggestion.label).width())
            .max()
            .unwrap_or(0);
        let chosen = self
            .popup
            .iter()
            .position(|(_, chosen)| *chosen)
            .unwrap_or(0);
        let first = chosen.saturating_sub(shown - 1);
        let rows = self
            .popup
            .iter()
            .skip(first)
            .take(shown)
            .map(|(suggestion, chosen)| {
                let (mark, ink) = if *chosen {
                    ("› ", Ink::Focus)
                } else {
                    ("  ", Ink::Text)
                };
                Line::new(vec![
                    Span::new(format!(" {mark}{:<label_width$}  ", suggestion.label), ink),
                    Span::new(suggestion.detail.clone(), Ink::Muted),
                ])
            })
            .collect();
        View::layer(
            body,
            View::fill(Ink::Bar, View::lines(rows, Anchor::Top)),
            Place::Bottom {
                inset: 2,
                width: self.width.saturating_sub(4).min(80),
                height: u16::try_from(shown).unwrap_or(0),
            },
        )
    }

    /// The message box, its cursor's line kept in view.
    fn composer(&self, rows: u16) -> View {
        let prompt = Span::new(" › ", Ink::Prompt);
        let blank = Span::plain("   ");
        if let Some(placeholder) = &self.placeholder {
            let mut chars = placeholder.chars();
            let first = chars.next().map_or_else(|| " ".to_owned(), String::from);
            return View::lines(
                vec![Line::new(vec![
                    prompt,
                    Span::new(first, Ink::Text).with(Emphasis::Reversed),
                    Span::new(chars.as_str(), Ink::Muted),
                ])],
                Anchor::Top,
            );
        }
        let first = self.cursor.0.saturating_sub(usize::from(rows) - 1);
        let lines = self
            .composer
            .iter()
            .enumerate()
            .skip(first)
            .take(usize::from(rows))
            .map(|(row, text)| {
                let lead = if row == 0 {
                    prompt.clone()
                } else {
                    blank.clone()
                };
                if row != self.cursor.0 {
                    return Line::new(vec![lead, Span::new(text.clone(), Ink::Text)]);
                }
                // The cell under the cursor, reversed.
                let mut before = String::new();
                let mut used = 0;
                let mut chars = text.chars();
                while let Some(ch) = chars.next() {
                    if used >= self.cursor.1 {
                        return Line::new(vec![
                            lead,
                            Span::new(before, Ink::Text),
                            Span::new(ch.to_string(), Ink::Text).with(Emphasis::Reversed),
                            Span::new(chars.as_str(), Ink::Text),
                        ]);
                    }
                    used += Text(&ch.to_string()).width();
                    before.push(ch);
                }
                // At the end: the cursor, and what the chosen completion
                // would fill in, under it and after it.
                let ghost = self.ghost.as_deref().unwrap_or_default();
                let mut rest = ghost.chars();
                let under = rest.next().map_or_else(|| " ".to_owned(), String::from);
                let under_ink = if ghost.is_empty() {
                    Ink::Text
                } else {
                    Ink::Muted
                };
                Line::new(vec![
                    lead,
                    Span::new(before, Ink::Text),
                    Span::new(under, under_ink).with(Emphasis::Reversed),
                    Span::new(rest.as_str(), Ink::Muted),
                ])
            })
            .collect();
        View::lines(lines, Anchor::Top)
    }
}

/// `HH:MM:SS UTC`.
pub(crate) struct Clock(pub UnixMillis);

impl std::fmt::Display for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let seconds = self.0.as_millis() / 1000 % 86_400;
        write!(
            f,
            "{:02}:{:02}:{:02} UTC",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    }
}
