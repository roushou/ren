//! What the console shows, as styled lines for its current width: the
//! header, the conversation, the approval card, the message box, and the
//! hint under it. Pure: `draw` puts a [`Frame`] on the terminal.

use crate::{
    Console, Control, Projects, Screen, Tone,
    desk::{Attention, Run},
    slash::Suggestion,
    transcript::Entry,
};
use ren_harness::drive::{ApprovalAsk, ItemStatus, TurnEnd};
use ren_protocol::activity::ResultStatus;
use ren_store::activity::TurnActivity;
use ren_tui::Ink;
use unicode_width::UnicodeWidthChar;

/// A run of text in one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Styled {
    pub text: String,
    pub ink: Ink,
}

/// One line of styled text.
pub type Line = Vec<Styled>;

/// Everything to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
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

fn styled(text: impl Into<String>, ink: Ink) -> Styled {
    Styled {
        text: text.into(),
        ink,
    }
}

impl Frame {
    /// `text` cut into lines no wider than `width` columns, at spaces where
    /// it can be, keeping its own line breaks.
    pub fn wrap(text: &str, width: usize) -> Vec<String> {
        let width = width.max(8);
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            let mut used = 0;
            for word in paragraph.split(' ') {
                let wide: usize = word.chars().map(|ch| ch.width().unwrap_or(0)).sum();
                if used > 0 && used + 1 + wide > width {
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                if used > 0 {
                    line.push(' ');
                    used += 1;
                }
                // A word wider than the line is cut.
                for ch in word.chars() {
                    let w = ch.width().unwrap_or(0);
                    if used + w > width {
                        lines.push(std::mem::take(&mut line));
                        used = 0;
                    }
                    line.push(ch);
                    used += w;
                }
            }
            lines.push(line);
        }
        lines
    }

    /// The column width of `text`.
    pub fn columns(text: &str) -> usize {
        text.chars().map(|ch| ch.width().unwrap_or(0)).sum()
    }
}

impl Console {
    /// What to draw now.
    pub fn frame(&self) -> Frame {
        let width = usize::from(self.size.width);
        let (title, body, side) = match &self.screen {
            Screen::Run(id) => match self.desk.get(*id) {
                Some(run) => (
                    self.title_of(run),
                    self.conversation(run, width.saturating_sub(4)),
                    None,
                ),
                None => (vec![styled(" run", Ink::Text)], Vec::new(), None),
            },
            Screen::Home { chosen } => (self.home_title(), self.home(*chosen, width), None),
            Screen::Resume { search, .. } => {
                let list = self.resume_list();
                let chosen = self.resume_chosen().min(list.len().saturating_sub(1));
                let side_width = (width / 3).clamp(24, 48);
                let preview = list.get(chosen).map_or_else(Vec::new, |run| {
                    self.conversation(run, width.saturating_sub(side_width + 6))
                });
                (
                    vec![
                        styled(" /resume", Ink::Text),
                        styled(format!("   search: {search}▏"), Ink::Muted),
                    ],
                    preview,
                    Some(self.resume_side(&list, chosen, side_width)),
                )
            }
        };
        let text = self.composer.text();
        let composer: Vec<String> = text.split('\n').map(str::to_owned).collect();
        let before: String = text.chars().take(self.composer.cursor()).collect();
        let cursor_line = before.matches('\n').count();
        let cursor_col = Frame::columns(before.rsplit('\n').next().unwrap_or_default());
        let suggestions = self.suggestions();
        let choice = self.choice.min(suggestions.len().saturating_sub(1));
        let (hint, hint_right) = self.hint();
        Frame {
            width: self.size.width,
            height: self.size.height,
            title,
            strip: self.strip(),
            body,
            scroll: self.scroll,
            side,
            popup: suggestions
                .into_iter()
                .enumerate()
                .map(|(index, suggestion)| (suggestion, index == choice))
                .collect(),
            composer,
            cursor: (cursor_line, cursor_col),
            placeholder: self.composer.is_empty().then(|| self.placeholder()),
            hint,
            hint_right,
        }
    }

    fn title_of(&self, run: &Run) -> Line {
        let ink = match run.attention() {
            Attention::NeedsYou => Ink::Warning,
            Attention::YourTurn | Attention::Working | Attention::Starting => Ink::Text,
            Attention::Ended => Ink::Muted,
        };
        vec![
            styled(format!(" {}", run.project()), Ink::Text),
            styled(format!(" · {} · ", run.client()), Ink::Muted),
            styled(run.phase(), ink),
        ]
    }

    fn home_title(&self) -> Line {
        let words = match (&self.control, self.desk.needing()) {
            (Control::Unavailable(_), _) => " runs",
            (Control::Unread, _) => " reading runs…",
            (Control::Ready, 0) => " nothing needs you",
            (Control::Ready, _) => " runs",
        };
        vec![styled(words, Ink::Text)]
    }

    /// One mark per live run, what needs you first: `◆` is the open one,
    /// `◇` one that waits for you, `▪` the rest.
    fn strip(&self) -> Line {
        let current = self.current();
        let order = self.desk.ordered();
        let mut line: Line = Vec::new();
        for run in &order {
            let needs = run.attention() == Attention::NeedsYou;
            let (mark, ink) = match (Some(run.id) == current, needs) {
                (true, true) => ("◆ ", Ink::Warning),
                (true, false) => ("◆ ", Ink::Focus),
                (false, true) => ("◇ ", Ink::Warning),
                (false, false) => ("▪ ", Ink::Muted),
            };
            line.push(styled(mark, ink));
        }
        let needing = self.desk.needing();
        let counts = match needing {
            0 => format!("  {} live ", order.len()),
            1 => format!("  1 needs you · {} live ", order.len()),
            n => format!("  {n} need you · {} live ", order.len()),
        };
        line.push(styled(
            counts,
            if needing > 0 {
                Ink::Warning
            } else {
                Ink::Muted
            },
        ));
        line
    }

    fn placeholder(&self) -> String {
        match (&self.screen, self.run()) {
            (Screen::Run(_), Some(run)) => match run.attention() {
                Attention::NeedsYou => "1–4 answers, or type to reply instead".to_owned(),
                Attention::Working | Attention::Starting => {
                    format!("Message {}: it reads it when this turn ends", run.client())
                }
                Attention::YourTurn => format!("Message {}…", run.client()),
                Attention::Ended => "Type to continue this conversation in a new run".to_owned(),
            },
            (Screen::Home { .. } | Screen::Run(_) | Screen::Resume { .. }, _) => {
                "/new <project> <prompt> starts a run · /help".to_owned()
            }
        }
    }

    fn hint(&self) -> (Line, Line) {
        if let Some(notice) = &self.notice {
            let ink = match notice.tone {
                Tone::Info => Ink::Muted,
                Tone::Refusal => Ink::Error,
            };
            return (vec![styled(format!(" {}", notice.text), ink)], Vec::new());
        }
        if let Control::Unavailable(why) = &self.control {
            return (
                vec![styled(
                    format!(" runs cannot be driven from here: {why}"),
                    Ink::Error,
                )],
                Vec::new(),
            );
        }
        if let Projects::Failed(why) = &self.projects {
            return (
                vec![styled(format!(" cannot list projects: {why}"), Ink::Error)],
                Vec::new(),
            );
        }
        let left = match self.run() {
            Some(run) => match (&run.queued, run.attention()) {
                (Some(queued), _) => vec![styled(
                    format!(
                        " queued: {} · sends when this turn ends",
                        Run::shorten(queued.as_str(), 40)
                    ),
                    Ink::Muted,
                )],
                (None, Attention::NeedsYou) => vec![styled(" 1–4 answers", Ink::Muted)],
                (None, Attention::Working | Attention::Starting) => {
                    vec![styled(" esc interrupts", Ink::Muted)]
                }
                (None, Attention::YourTurn | Attention::Ended) => {
                    vec![styled(" enter sends · ctrl-j new line", Ink::Muted)]
                }
            },
            None => match &self.screen {
                Screen::Resume { .. } => {
                    vec![styled(" ↑↓ choose · enter opens · esc back", Ink::Muted)]
                }
                Screen::Home { .. } | Screen::Run(_) => {
                    vec![styled(" ↑↓ choose · enter opens", Ink::Muted)]
                }
            },
        };
        let right = vec![styled("alt-←→ runs · /resume · /help ", Ink::Muted)];
        (left, right)
    }

    /// The list when nothing is open: live runs, what needs you first.
    fn home(&self, chosen: usize, width: usize) -> Vec<Line> {
        let order = self.desk.ordered();
        if order.is_empty() {
            let mut lines = vec![Vec::new()];
            let text = match (&self.control, &self.projects) {
                (Control::Unavailable(why), _) => format!("runs cannot be driven from here: {why}"),
                (_, Projects::Read(projects)) if !projects.is_empty() => format!(
                    "no runs yet. start one: /new {} <prompt>",
                    projects
                        .first()
                        .map_or("<project>", |project| project.name.as_str())
                ),
                (_, Projects::Read(_)) => {
                    "no runs and no projects: add a folder to the daemon's [runs] roots".to_owned()
                }
                (_, Projects::Unread | Projects::Failed(_)) => "no runs yet".to_owned(),
            };
            lines.push(vec![styled(format!("    {text}"), Ink::Muted)]);
            return lines;
        }
        let name_width = order
            .iter()
            .map(|run| Frame::columns(run.project()))
            .max()
            .unwrap_or(0);
        let mut lines = vec![Vec::new()];
        for (index, run) in order.iter().enumerate() {
            let mark = if index == chosen { "  › " } else { "    " };
            let doing = match run.attention() {
                Attention::Working => run.transcript.doing().unwrap_or_default(),
                Attention::YourTurn => run
                    .transcript
                    .last_reply()
                    .map(|reply| Run::shorten(reply, 40))
                    .unwrap_or_default(),
                Attention::NeedsYou => format!(
                    "wants {}",
                    Self::asked(run.waiting().map(|waiting| &waiting.ask))
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
            let title = Run::shorten(&run.title(), 28);
            let mut line = vec![
                styled(mark, Ink::Focus),
                styled(format!("{:<name_width$}   ", run.project()), Ink::Text),
                styled(format!("{title:<28}   "), Ink::Text),
                styled(format!("{:<10} ", run.phase()), phase_ink),
            ];
            let used: usize = line.iter().map(|span| Frame::columns(&span.text)).sum();
            line.push(styled(
                Run::shorten(&doing, width.saturating_sub(used + 2)),
                Ink::Muted,
            ));
            lines.push(line);
        }
        lines
    }

    /// `/resume`'s list, grouped by how much each run needs you.
    fn resume_side(&self, list: &[&Run], chosen: usize, width: usize) -> Vec<Line> {
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
                    lines.push(Vec::new());
                }
                lines.push(vec![styled(heading, Ink::Muted)]);
            }
            let (mark, ink) = if index == chosen {
                ("› ", Ink::Focus)
            } else {
                ("  ", Ink::Text)
            };
            let room = width.saturating_sub(4);
            let project = Run::shorten(run.project(), room / 3);
            let title = Run::shorten(
                &run.title(),
                room.saturating_sub(Frame::columns(&project) + 2),
            );
            lines.push(vec![
                styled(mark, ink),
                styled(format!("{project}  "), Ink::Muted),
                styled(title, ink),
            ]);
        }
        if list.is_empty() {
            lines.push(vec![styled("no run matches", Ink::Muted)]);
        }
        lines
    }

    /// A run's conversation: its recorded turns, then what streamed since,
    /// then what it waits for.
    fn conversation(&self, run: &Run, width: usize) -> Vec<Line> {
        let mut lines: Vec<Line> = Vec::new();
        if let (Some(handoff), Some(current)) = (&self.handoff, self.current())
            && current == run.id
        {
            lines.push(vec![styled(format!("  ↳ {handoff}"), Ink::Muted)]);
        }
        for turn in &run.history {
            lines.push(Vec::new());
            Self::recorded_turn(turn, width, &mut lines);
        }
        if !run.transcript.is_empty() {
            for entry in run.transcript.entries() {
                Self::entry(entry, width, &mut lines);
            }
        }
        if let Some(waiting) = run.waiting() {
            lines.push(Vec::new());
            Self::card(&run.client(), &waiting.ask, width, &mut lines);
        }
        if run.history.is_empty() && run.transcript.is_empty() && run.waiting().is_none() {
            lines.push(Vec::new());
            let text = match run.attention() {
                Attention::Starting => format!("starting {} in {}…", run.client(), run.workdir),
                Attention::YourTurn => {
                    format!(
                        "{} is ready in {}. What should it do?",
                        run.client(),
                        run.workdir
                    )
                }
                Attention::Working | Attention::NeedsYou => "working…".to_owned(),
                Attention::Ended => "this run ended".to_owned(),
            };
            lines.push(vec![styled(format!("  {text}"), Ink::Muted)]);
        }
        lines
    }

    fn recorded_turn(turn: &TurnActivity, width: usize, lines: &mut Vec<Line>) {
        if let Some(prompt) = &turn.prompt {
            for (index, text) in Frame::wrap(&prompt.text, width.saturating_sub(4))
                .into_iter()
                .enumerate()
            {
                let lead = if index == 0 { "  › " } else { "    " };
                lines.push(vec![styled(format!("{lead}{text}"), Ink::Prompt)]);
            }
        }
        for call in &turn.calls {
            for tool in &call.tool_calls {
                let target = tool
                    .effects
                    .first()
                    .map_or_else(|| tool.input.text.clone(), |effect| effect.target.clone());
                let failed = tool
                    .result
                    .as_ref()
                    .is_some_and(|result| result.status.known() == Some(ResultStatus::Failed));
                Self::tool_line(
                    &tool.name,
                    &target,
                    if failed { "failed" } else { "" },
                    width,
                    lines,
                );
            }
            if let Some(reply) = &call.reply
                && !reply.text.trim().is_empty()
            {
                for text in Frame::wrap(&reply.text, width.saturating_sub(4)) {
                    lines.push(vec![styled(format!("    {text}"), Ink::Text)]);
                }
            }
        }
    }

    fn tool_line(name: &str, target: &str, end: &str, width: usize, lines: &mut Vec<Line>) {
        let name = format!("  ● {name:<6} ");
        let room = width.saturating_sub(Frame::columns(&name) + Frame::columns(end) + 2);
        let target = Run::shorten(target.lines().next().unwrap_or_default(), room);
        let gap = width
            .saturating_sub(Frame::columns(&name) + Frame::columns(&target) + Frame::columns(end));
        lines.push(vec![
            styled(name, Ink::Tool),
            styled(target, Ink::Code),
            styled(format!("{}{end}", " ".repeat(gap)), Ink::Muted),
        ]);
    }

    fn entry(entry: &Entry, width: usize, lines: &mut Vec<Line>) {
        match entry {
            Entry::Prompt(text) => {
                lines.push(Vec::new());
                for (index, text) in Frame::wrap(text, width.saturating_sub(4))
                    .into_iter()
                    .enumerate()
                {
                    let lead = if index == 0 { "  › " } else { "    " };
                    lines.push(vec![styled(format!("{lead}{text}"), Ink::Prompt)]);
                }
            }
            Entry::Message { text, done, .. } => {
                let mut wrapped = Frame::wrap(text, width.saturating_sub(4));
                if !done && let Some(last) = wrapped.last_mut() {
                    last.push('▍');
                }
                for text in wrapped {
                    lines.push(vec![styled(format!("    {text}"), Ink::Text)]);
                }
            }
            Entry::Reasoning { done: false, .. } => {
                lines.push(vec![styled("    thinking…", Ink::Muted)]);
            }
            Entry::Reasoning { done: true, .. } => {}
            Entry::Command {
                command,
                status,
                output,
                exit_code,
                ..
            } => {
                let end = match (status, exit_code) {
                    (ItemStatus::InProgress, _) => "running…".to_owned(),
                    (_, Some(code)) => format!("exit {code}"),
                    (ItemStatus::Completed, None) => "done".to_owned(),
                    (ItemStatus::Failed, None) => "failed".to_owned(),
                    (ItemStatus::Declined, None) => "denied".to_owned(),
                    (ItemStatus::Other(other), None) => other.clone(),
                };
                Self::tool_line("Bash", command, &end, width, lines);
                // A failed command shows the end of its output.
                let failed =
                    matches!(status, ItemStatus::Failed) || exit_code.is_some_and(|code| code != 0);
                if failed && let Some(output) = output {
                    let tail: Vec<&str> = output.lines().rev().take(3).collect();
                    for text in tail.into_iter().rev() {
                        lines.push(vec![styled(
                            format!(
                                "           {}",
                                Run::shorten(text, width.saturating_sub(12))
                            ),
                            Ink::Muted,
                        )]);
                    }
                }
            }
            Entry::FileChange { paths, status, .. } => {
                let end = match status {
                    ItemStatus::InProgress => "editing…",
                    ItemStatus::Completed => "",
                    ItemStatus::Failed => "failed",
                    ItemStatus::Declined => "denied",
                    ItemStatus::Other(other) => other.as_str(),
                };
                let target = if paths.is_empty() {
                    "files".to_owned()
                } else {
                    paths.join(", ")
                };
                let mut line_lines = Vec::new();
                Self::tool_line("Edit", &target, end, width, &mut line_lines);
                for mut line in line_lines {
                    if let Some(first) = line.first_mut() {
                        first.text = first.text.replacen('●', "✎", 1);
                    }
                    lines.push(line);
                }
            }
            Entry::Tool { name, status, .. } => {
                let (tool, target) = name.split_once(' ').unwrap_or((name.as_str(), ""));
                let end = match status {
                    ItemStatus::InProgress => "running…",
                    ItemStatus::Completed => "",
                    ItemStatus::Failed => "failed",
                    ItemStatus::Declined => "denied",
                    ItemStatus::Other(other) => other.as_str(),
                };
                Self::tool_line(tool, target, end, width, lines);
            }
            Entry::Notice { text, retrying } => {
                let (text, ink) = if *retrying {
                    (format!("    ! {text} · retrying"), Ink::Warning)
                } else {
                    (format!("    ! {text}"), Ink::Error)
                };
                lines.push(vec![styled(text, ink)]);
            }
            Entry::TurnEnd(end) => match end {
                TurnEnd::Completed => {}
                TurnEnd::Interrupted => {
                    lines.push(vec![styled("    turn interrupted", Ink::Muted)]);
                }
                TurnEnd::Failed { message } => lines.push(vec![styled(
                    format!(
                        "    turn failed{}",
                        message
                            .as_ref()
                            .map_or_else(String::new, |message| format!(": {message}"))
                    ),
                    Ink::Error,
                )]),
                TurnEnd::Unknown { status } => {
                    lines.push(vec![styled(
                        format!("    turn ended: {status}"),
                        Ink::Muted,
                    )]);
                }
            },
        }
    }

    /// The approval card: what the client wants, and the four answers.
    fn card(client: &str, ask: &ApprovalAsk, width: usize, lines: &mut Vec<Line>) {
        let (verb, detail, reason) = match ask {
            ApprovalAsk::Command {
                command,
                cwd,
                reason,
            } => (
                "run".to_owned(),
                command.clone().unwrap_or_else(|| "a command".to_owned()),
                match (reason, cwd) {
                    (Some(reason), _) => Some(reason.clone()),
                    (None, Some(cwd)) => Some(format!("in {cwd}")),
                    (None, None) => None,
                },
            ),
            ApprovalAsk::FileChange { reason } => (
                "change files".to_owned(),
                reason.clone().unwrap_or_else(|| "files".to_owned()),
                None,
            ),
            ApprovalAsk::Tool { tool, detail } => (
                format!("use {tool}"),
                detail.clone().unwrap_or_default(),
                None,
            ),
        };
        let inner = width.saturating_sub(4).clamp(20, 88);
        let title = format!("┌ {client} wants to {verb} ");
        let fill = (inner + 3).saturating_sub(Frame::columns(&title));
        lines.push(vec![styled(
            format!("  {title}{}┐", "─".repeat(fill)),
            Ink::Warning,
        )]);
        let row = |text: &str, ink: Ink| -> Line {
            let pad = inner.saturating_sub(Frame::columns(text));
            vec![
                styled("  │ ", Ink::Warning),
                styled(format!("{text}{}", " ".repeat(pad)), ink),
                styled(" │", Ink::Warning),
            ]
        };
        for text in Frame::wrap(&detail, inner) {
            lines.push(row(&text, Ink::Code));
        }
        if let Some(reason) = reason {
            for text in Frame::wrap(&reason, inner) {
                lines.push(row(&text, Ink::Muted));
            }
        }
        lines.push(row("", Ink::Text));
        let choices = "1 Allow once   2 Allow for this run   3 Deny   4 Deny and stop the turn";
        for text in Frame::wrap(choices, inner) {
            lines.push(row(&text, Ink::Focus));
        }
        lines.push(vec![styled(
            format!("  └{}┘", "─".repeat(inner + 2)),
            Ink::Warning,
        )]);
    }
}
