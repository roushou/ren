//! `/resume`: every run, what needs you first, searched by typing, with the
//! chosen one's conversation beside the list.

use super::frame::{Frame, Here};
use super::session::SessionScreen;
use super::{Action, Ctx, Look};
use crate::Key;
use crate::keys::HelpRow;
use crate::model::desk::{Attention, Run};
use crate::model::{Model, Read};
use crate::ui::{Anchor, Ink, Line, Lines, Size, Span, Text, View};
use ren_recorder::id::RunId;

/// The runs, searched, one chosen.
#[derive(Debug)]
pub(crate) struct Resume {
    search: String,
    chosen: usize,
    /// The run whose recorded turns were asked for last.
    previewed: Option<RunId>,
}

impl Resume {
    pub fn new(search: String) -> Self {
        Self {
            search,
            chosen: 0,
            previewed: None,
        }
    }

    /// Shown: the recorded runs are read again.
    pub fn shown(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let mut actions = vec![Action::Read(Read::Recorded)];
        actions.extend(self.preview(ctx.model));
        actions
    }

    pub fn changed(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        self.preview(ctx.model)
    }

    /// The chosen run's recorded turns, once per run, once its session is
    /// known.
    fn preview(&mut self, model: &Model) -> Vec<Action> {
        let Some(run) = self.list(model).get(self.chosen).map(|run| run.id) else {
            return Vec::new();
        };
        if self.previewed == Some(run) {
            return Vec::new();
        }
        let read = model.history(run, false);
        if let Read::History { .. } = read {
            self.previewed = Some(run);
        }
        vec![Action::Read(read)]
    }

    pub fn press(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        match key {
            Key::Char(ch) => {
                self.search.push(ch);
                self.chosen = 0;
            }
            Key::Backspace => {
                self.search.pop();
                self.chosen = 0;
            }
            Key::Up => self.chosen = self.chosen.saturating_sub(1),
            Key::Down => self.chosen += 1,
            Key::Esc => return vec![Action::Pop],
            Key::Interrupt => return vec![Action::Quit],
            Key::Enter => {
                return match self.list(ctx.model).get(self.chosen).map(|run| run.id) {
                    Some(run) => vec![Action::Pop, Action::Open(run)],
                    None => Vec::new(),
                };
            }
            Key::Tab
            | Key::BackTab
            | Key::Left
            | Key::Right
            | Key::Delete
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
            | Key::AltLeft
            | Key::AltRight
            | Key::Newline => {}
        }
        // The chosen row stays within the list.
        self.chosen = self
            .chosen
            .min(self.list(ctx.model).len().saturating_sub(1));
        self.preview(ctx.model)
    }

    /// The runs its search finds, what needs you first.
    fn list<'a>(&self, model: &'a Model) -> Vec<&'a Run> {
        let search = self.search.to_lowercase();
        model
            .desk
            .all()
            .into_iter()
            .filter(|run| {
                search.is_empty()
                    || run.title().to_lowercase().contains(&search)
                    || run.workdir.to_lowercase().contains(&search)
            })
            .collect()
    }

    /// Its keys, for the help.
    pub fn keys() -> Vec<HelpRow> {
        [
            ("type", "search the runs"),
            ("↑ ↓", "choose"),
            ("enter", "open the chosen run"),
            ("esc", "back"),
        ]
        .into_iter()
        .map(|(label, does)| HelpRow {
            label: label.to_owned(),
            does: does.to_owned(),
        })
        .collect()
    }

    pub fn view(&self, look: &Look<'_>) -> View {
        let room = Frame::body(look);
        let width = usize::from(room.width);
        let list = self.list(look.model);
        let chosen = self.chosen.min(list.len().saturating_sub(1));
        let side_width = (room.width / 3).clamp(24, 48).min(room.width / 2);
        let preview = list.get(chosen).map_or_else(Vec::new, |run| {
            SessionScreen::conversation(
                run,
                None,
                width.saturating_sub(usize::from(side_width) + 6),
            )
        });
        let body = View::Columns(vec![
            (
                Size::Fill,
                View::Lines(Lines {
                    lines: preview,
                    anchor: Anchor::Newest,
                    scroll: 0,
                }),
            ),
            (
                Size::Exact(side_width),
                View::pad(
                    1,
                    0,
                    View::lines(
                        Self::side(&list, chosen, usize::from(side_width)),
                        Anchor::Top,
                    ),
                ),
            ),
        ]);
        Frame::new(
            look,
            &Here::Resume,
            body,
            Line::new(vec![
                Span::new(format!(" search: {}▏", self.search), Ink::Text),
                Span::new(" · ↑↓ choose · enter opens · esc back", Ink::Muted),
            ]),
            "type to search the runs",
        )
        .view()
    }

    /// The list, grouped by how much each run needs you.
    fn side(list: &[&Run], chosen: usize, width: usize) -> Vec<Line> {
        let mut lines = Vec::new();
        let mut group = None;
        for (index, run) in list.iter().enumerate() {
            let attention = run.attention();
            if group != Some(attention) {
                group = Some(attention);
                let heading = match attention {
                    Attention::NeedsYou => "needs you",
                    Attention::YourTurn => "your turn",
                    Attention::Working | Attention::Starting => "working",
                    Attention::Ended => "ended",
                };
                if !lines.is_empty() {
                    lines.push(Line::default());
                }
                lines.push(Line::of(heading, Ink::Muted));
            }
            let (mark, ink) = if index == chosen {
                ("› ", Ink::Focus)
            } else {
                ("  ", Ink::Text)
            };
            let room = width.saturating_sub(4);
            let project = Text(run.project()).summary(room / 3);
            let title = Text(&run.title()).summary(room.saturating_sub(Text(&project).width() + 2));
            lines.push(Line::new(vec![
                Span::new(mark, ink),
                Span::new(format!("{project}  "), Ink::Muted),
                Span::new(title, ink),
            ]));
        }
        if list.is_empty() {
            lines.push(Line::of("no run matches", Ink::Muted));
        }
        lines
    }
}
