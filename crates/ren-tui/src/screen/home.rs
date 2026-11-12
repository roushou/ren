//! The console with no run open: the live runs, what needs you first. A
//! run that needs you opens on its own.

use super::chrome::Chrome;
use super::message::Typed;
use super::run::RunScreen;
use super::slash::Slash;
use super::{Action, Ctx, Look, News, Notice, Screen};
use crate::Key;
use crate::model::desk::{Attention, Run};
use crate::model::{Control, Projects};
use crate::ui::{Ink, Line, Span, Text, View};

/// The live runs, one chosen.
#[derive(Debug, Default)]
pub(crate) struct Home {
    chosen: usize,
}

impl Home {
    /// A run that needs you opens; one started here opens.
    pub fn changed(&mut self, ctx: &mut Ctx<'_>, news: &[News]) -> Vec<Action> {
        if let Some(News::Started(run)) = news.first() {
            return vec![Action::Replace(Screen::Run(RunScreen::new(*run)))];
        }
        match ctx.model.desk.next_needing(None) {
            Some(run) => vec![Action::Replace(Screen::Run(RunScreen::new(run)))],
            None => Vec::new(),
        }
    }

    pub fn press(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let key = match ctx.message.press(key, ctx.model.projects()) {
            Typed::Took => return Vec::new(),
            Typed::Submit => return self.submit(ctx),
            Typed::Left(key) => key,
        };
        match key {
            Key::Up => self.chosen = self.chosen.saturating_sub(1),
            Key::Down => {
                self.chosen = (self.chosen + 1).min(ctx.model.desk.live_count().saturating_sub(1));
            }
            Key::AltLeft => return RunScreen::step(ctx.model, None, false),
            Key::AltRight => return RunScreen::step(ctx.model, None, true),
            Key::Interrupt => return vec![Action::Quit],
            Key::Char(_)
            | Key::Tab
            | Key::BackTab
            | Key::Left
            | Key::Right
            | Key::Enter
            | Key::Esc
            | Key::Backspace
            | Key::Delete
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
            | Key::Newline => {}
        }
        Vec::new()
    }

    fn submit(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let text = ctx.message.text();
        if text.trim().is_empty() {
            return match ctx.model.desk.ordered().get(self.chosen) {
                Some(run) => vec![Action::Replace(Screen::Run(RunScreen::new(run.id)))],
                None => Vec::new(),
            };
        }
        if let Some(slash) = Slash::parse(&text) {
            ctx.message.take();
            return slash.act(ctx, None);
        }
        if let Control::Unavailable(why) = &ctx.model.control {
            return vec![Action::Say(Notice::refusal(format!(
                "runs cannot be driven from here: {why}"
            )))];
        }
        vec![Action::Say(Notice::info(
            "open a run first, or start one: /new <project> <prompt>",
        ))]
    }

    pub fn view(&self, look: &Look<'_>) -> View {
        let model = look.model;
        let words = match (&model.control, model.desk.needing()) {
            (Control::Unavailable(_), _) => " runs",
            (Control::Unread, _) => " reading runs…",
            (Control::Ready, 0) => " nothing needs you",
            (Control::Ready, _) => " runs",
        };
        Chrome::new(
            look,
            Line::of(words, Ink::Text),
            self.list(look),
            None,
            RunScreen::idle_placeholder(),
            Line::of(" ↑↓ choose · enter opens", Ink::Muted),
        )
        .view()
    }

    /// The live runs, what needs you first.
    fn list(&self, look: &Look<'_>) -> Vec<Line> {
        let model = look.model;
        let width = usize::from(look.size.width);
        let order = model.desk.ordered();
        if order.is_empty() {
            let text = match (&model.control, &model.projects) {
                (Control::Unavailable(why), _) => format!("runs cannot be driven from here: {why}"),
                (_, Projects::Read(projects)) if !projects.is_empty() => format!(
                    "no live runs. start one with /new {} <prompt>, or /resume an earlier one",
                    projects
                        .first()
                        .map_or("<project>", |project| project.name.as_str())
                ),
                (_, Projects::Read(_)) => {
                    "no live runs, and no projects: add a folder to the daemon's [runs] roots"
                        .to_owned()
                }
                (_, Projects::Unread | Projects::Failed(_)) => "no live runs".to_owned(),
            };
            return vec![Line::default(), Line::of(format!("    {text}"), Ink::Muted)];
        }
        let name_width = order
            .iter()
            .map(|run| Text(run.project()).width())
            .max()
            .unwrap_or(0);
        let mut lines = vec![Line::default()];
        for (index, run) in order.iter().enumerate() {
            let mark = if index == self.chosen {
                "  › "
            } else {
                "    "
            };
            let doing = match run.attention() {
                Attention::Working => run.transcript.doing().unwrap_or_default(),
                Attention::YourTurn => run
                    .transcript
                    .last_reply()
                    .map(|reply| Text(reply).summary(40))
                    .unwrap_or_default(),
                Attention::NeedsYou => format!(
                    "wants {}",
                    Run::asked(run.waiting().map(|waiting| &waiting.ask))
                ),
                Attention::Starting | Attention::Ended => String::new(),
            };
            let phase_ink = match run.attention() {
                Attention::NeedsYou => Ink::Warning,
                Attention::YourTurn
                | Attention::Working
                | Attention::Starting
                | Attention::Ended => Ink::Muted,
            };
            let title = Text(&run.title()).summary(28);
            let mut line = Line::new(vec![
                Span::new(mark, Ink::Focus),
                Span::new(format!("{:<name_width$}   ", run.project()), Ink::Text),
                Span::new(format!("{title:<28}   "), Ink::Text),
                Span::new(format!("{:<10} ", run.phase()), phase_ink),
            ]);
            let used = line.width();
            line.push(Span::new(
                Text(&doing).summary(width.saturating_sub(used + 2)),
                Ink::Muted,
            ));
            lines.push(line);
        }
        lines
    }
}
