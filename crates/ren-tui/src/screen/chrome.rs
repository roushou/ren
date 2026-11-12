//! What the console's screens share around their body: the title line with
//! a mark for every live run, a rule, the body (beside a list, for
//! `/resume`), completions over it, the message box, and the hint line.

use super::{Look, NoticeTone, Suggestion};
use crate::model::desk::Attention;
use crate::model::{Control, Projects};
use crate::ui::{Anchor, Emphasis, Ink, Keep, Line, Lines, Place, Rule, Size, Span, Text, View};
use ren_recorder::id::RunId;

/// One console screen, laid out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Chrome {
    pub width: u16,
    pub height: u16,
    /// The top line: what is open, left; the runs and their counts, right.
    pub title: Line,
    pub strip: Line,
    /// The conversation or list, oldest first; the newest lines show.
    pub body: Vec<Line>,
    /// Lines scrolled up from the newest.
    pub scroll: usize,
    /// `/resume`'s list, beside the preview.
    pub side: Option<Vec<Line>>,
    /// Completions over the message box, the chosen one marked.
    pub popup: Vec<(Suggestion, bool)>,
    /// The message box's lines, and where the cursor is in them.
    pub composer: Vec<String>,
    pub cursor: (usize, usize),
    pub placeholder: Option<String>,
    /// Under the message box: a notice, or what the keys do now.
    pub hint: Line,
    pub hint_right: Line,
}

impl Chrome {
    /// A console screen showing `body` under `title`, with `current` the
    /// run it shows; `placeholder` fills an empty message box and `keys`
    /// says what the keys do, unless a notice or a problem says more.
    pub fn new(
        look: &Look<'_>,
        title: Line,
        body: Vec<Line>,
        current: Option<RunId>,
        placeholder: String,
        keys: Line,
    ) -> Self {
        let message = look.message;
        let (composer, cursor) = message.lines();
        Self {
            width: look.size.width,
            height: look.size.height,
            title,
            strip: Self::strip(look, current),
            body,
            scroll: 0,
            side: None,
            popup: message.popup(look.model.projects()),
            composer,
            cursor,
            placeholder: message.is_empty().then_some(placeholder),
            hint: Self::hint(look, keys),
            hint_right: Line::of("alt-←→ runs · /resume · /wall · /help ", Ink::Muted),
        }
    }

    /// One mark per live run, what needs you first: `◆` is the open one,
    /// `◇` one that waits for you, `▪` the rest.
    fn strip(look: &Look<'_>, current: Option<RunId>) -> Line {
        let desk = &look.model.desk;
        let order = desk.ordered();
        let mut line = Line::default();
        for run in &order {
            let needs = run.attention() == Attention::NeedsYou;
            let (mark, ink) = match (Some(run.id) == current, needs) {
                (true, true) => ("◆ ", Ink::Warning),
                (true, false) => ("◆ ", Ink::Focus),
                (false, true) => ("◇ ", Ink::Warning),
                (false, false) => ("▪ ", Ink::Muted),
            };
            line.push(Span::new(mark, ink));
        }
        let needing = desk.needing();
        let counts = match needing {
            0 => format!("  {} live ", order.len()),
            1 => format!("  1 needs you · {} live ", order.len()),
            n => format!("  {n} need you · {} live ", order.len()),
        };
        line.push(Span::new(
            counts,
            if needing > 0 {
                Ink::Warning
            } else {
                Ink::Muted
            },
        ));
        line
    }

    /// The hint line: a notice, then a reason runs cannot be driven, then
    /// what the keys do.
    fn hint(look: &Look<'_>, keys: Line) -> Line {
        if let Some(notice) = look.notice {
            let ink = match notice.tone {
                NoticeTone::Info => Ink::Muted,
                NoticeTone::Refusal => Ink::Error,
            };
            return Line::of(format!(" {}", notice.text), ink);
        }
        if let Control::Unavailable(why) = &look.model.control {
            return Line::of(
                format!(" runs cannot be driven from here: {why}"),
                Ink::Error,
            );
        }
        if let Projects::Failed(why) = &look.model.projects {
            return Line::of(format!(" cannot list projects: {why}"), Ink::Error);
        }
        keys
    }

    /// What to draw.
    pub fn view(&self) -> View {
        if self.width < 4 || self.height < 5 {
            return View::fill(Ink::Ground, View::Empty);
        }
        // The message box: up to six lines, then the hint under it.
        let typed = u16::try_from(self.composer.len().clamp(1, 6)).unwrap_or(1);
        let conversation = View::Lines(Lines {
            lines: self.body.clone(),
            anchor: Anchor::Newest,
            scroll: self.scroll,
        });
        let body = match &self.side {
            Some(side) => {
                let side_width = (self.width / 3).clamp(24, 48).min(self.width / 2);
                View::Columns(vec![
                    (Size::Fill, conversation),
                    (
                        Size::Exact(side_width),
                        View::pad(1, 0, View::lines(side.clone(), Anchor::Top)),
                    ),
                ])
            }
            None => conversation,
        };
        View::fill(
            Ink::Ground,
            View::Rows(vec![
                (
                    Size::Exact(1),
                    View::bar(
                        self.title.clone(),
                        self.strip.clone(),
                        Ink::Ground,
                        Keep::Left,
                    ),
                ),
                (Size::Exact(1), View::Rule(Rule::Plain)),
                (Size::Fill, self.popup(body)),
                (Size::Exact(1), View::Rule(Rule::Plain)),
                (Size::Exact(typed), self.composer(typed)),
                (
                    Size::Exact(1),
                    View::bar(
                        self.hint.clone(),
                        self.hint_right.clone(),
                        Ink::Ground,
                        Keep::Left,
                    ),
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
                Line::new(vec![
                    lead,
                    Span::new(before, Ink::Text),
                    Span::new(" ", Ink::Text).with(Emphasis::Reversed),
                ])
            })
            .collect();
        View::lines(lines, Anchor::Top)
    }
}
