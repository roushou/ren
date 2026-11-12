//! A conversation as the screens show it, read once and drawn the same
//! everywhere: the operator's prompts, the model's replies, what it asked
//! tools to do and what came back. It is read from recorded calls
//! (`read`) or from a run's live events ([`Conversation::live`]), and laid
//! out at two densities: a wall pane's, compact with each model call
//! marked, and a run's page, with room around each turn.

mod read;

pub(crate) use read::{ResultMark, ToolLine};

use crate::model::transcript::Entry;
use crate::ui::{Emphasis, Ink, Line, Span, Text, Tone};
use ren_harness::drive::{ItemStatus, TurnEnd};
use ren_protocol::activity::ResultStatus;

/// One thing said or done, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Block {
    /// The start of a model call: its place among the turn's calls.
    Call {
        index: Option<usize>,
    },
    /// The operator's own words.
    Prompt(String),
    /// A result a call's request carried back for one of the previous
    /// call's tool calls (or a run of like ones).
    Result(ToolLine),
    Reply(String),
    /// Reply text still streaming: a cursor where it has got to.
    Streaming(String),
    Thinking,
    /// Tool calls the model asked for, as recorded.
    Tools(ToolLine),
    /// A step a client reports as it runs it: a command, an edit, a tool.
    Step(Step),
    Notice {
        text: String,
        tone: Tone,
    },
    /// An open call whose response head has not arrived.
    Waiting,
}

/// A step a client runs, as it reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Step {
    pub kind: StepKind,
    pub name: String,
    pub target: String,
    /// How it went, in a word or two; empty when it went as asked.
    pub end: String,
    /// The end of a failed command's output.
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepKind {
    Run,
    Edit,
}

/// How much room a conversation is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Density {
    /// A wall pane: every model call marked, nothing indented.
    Pane,
    /// A run's page: each prompt set apart, text indented under its marks.
    Page,
}

/// Blocks, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Conversation {
    pub blocks: Vec<Block>,
}

impl Conversation {
    /// What a run's client reported since it was followed.
    pub fn live(entries: &[Entry]) -> Self {
        let ended = |status: &ItemStatus, running: &str| -> String {
            match status {
                ItemStatus::InProgress => running.to_owned(),
                ItemStatus::Completed => String::new(),
                ItemStatus::Failed => "failed".to_owned(),
                ItemStatus::Declined => "denied".to_owned(),
                ItemStatus::Other(other) => other.clone(),
            }
        };
        let blocks = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Prompt(text) => Some(Block::Prompt(text.clone())),
                Entry::Message {
                    text, done: true, ..
                } => Some(Block::Reply(text.clone())),
                Entry::Message {
                    text, done: false, ..
                } => Some(Block::Streaming(text.clone())),
                Entry::Reasoning { done: false, .. } => Some(Block::Thinking),
                Entry::Reasoning { done: true, .. } | Entry::TurnEnd(TurnEnd::Completed) => None,
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
                    // A failed command shows the end of its output.
                    let failed = matches!(status, ItemStatus::Failed)
                        || exit_code.is_some_and(|code| code != 0);
                    let output = match (failed, output) {
                        (true, Some(output)) => {
                            let mut tail: Vec<String> =
                                output.lines().rev().take(3).map(str::to_owned).collect();
                            tail.reverse();
                            tail
                        }
                        (true | false, Some(_) | None) => Vec::new(),
                    };
                    Some(Block::Step(Step {
                        kind: StepKind::Run,
                        name: "Bash".to_owned(),
                        target: command.clone(),
                        end,
                        output,
                    }))
                }
                Entry::FileChange { paths, status, .. } => Some(Block::Step(Step {
                    kind: StepKind::Edit,
                    name: "Edit".to_owned(),
                    target: if paths.is_empty() {
                        "files".to_owned()
                    } else {
                        paths.join(", ")
                    },
                    end: ended(status, "editing…"),
                    output: Vec::new(),
                })),
                Entry::Tool { name, status, .. } => {
                    let (tool, target) = name.split_once(' ').unwrap_or((name.as_str(), ""));
                    Some(Block::Step(Step {
                        kind: StepKind::Run,
                        name: tool.to_owned(),
                        target: target.to_owned(),
                        end: ended(status, "running…"),
                        output: Vec::new(),
                    }))
                }
                Entry::Notice { text, retrying } => Some(match retrying {
                    true => Block::Notice {
                        text: format!("{text} · retrying"),
                        tone: Tone::Warning,
                    },
                    false => Block::Notice {
                        text: text.clone(),
                        tone: Tone::Error,
                    },
                }),
                Entry::TurnEnd(TurnEnd::Interrupted) => Some(Block::Notice {
                    text: "turn interrupted".to_owned(),
                    tone: Tone::Neutral,
                }),
                Entry::TurnEnd(TurnEnd::Failed { message }) => Some(Block::Notice {
                    text: format!(
                        "turn failed{}",
                        message
                            .as_ref()
                            .map_or_else(String::new, |message| format!(": {message}"))
                    ),
                    tone: Tone::Error,
                }),
                Entry::TurnEnd(TurnEnd::Unknown { status }) => Some(Block::Notice {
                    text: format!("turn ended: {status}"),
                    tone: Tone::Neutral,
                }),
            })
            .collect();
        Self { blocks }
    }

    /// The conversation as lines `width` cells wide, oldest first.
    pub fn lines(&self, width: usize, density: Density) -> Vec<Line> {
        let mut lines = Vec::new();
        for block in &self.blocks {
            Self::block(block, width, density, &mut lines);
        }
        lines
    }

    fn block(block: &Block, width: usize, density: Density, lines: &mut Vec<Line>) {
        // A page sets marks two cells in, and text four.
        let (mark, text) = match density {
            Density::Pane => ("", ""),
            Density::Page => ("  ", "    "),
        };
        let body = width.saturating_sub(text.len());
        let indented = |line: Line| -> Line {
            let mut spans = vec![Span::plain(text)];
            spans.extend(line.spans);
            Line::new(spans)
        };
        match block {
            Block::Call { index } => match density {
                Density::Pane => lines.push(Line::of(
                    match index {
                        Some(index) => format!("── call {index} ──"),
                        None => "── call ──".to_owned(),
                    },
                    Ink::Muted,
                )),
                Density::Page => {}
            },
            Block::Prompt(prompt) => {
                if density == Density::Page {
                    lines.push(Line::default());
                }
                lines.extend(Line::hanging(
                    &format!("{mark}› "),
                    prompt,
                    width,
                    Ink::Prompt,
                ));
            }
            Block::Result(tool) => match density {
                Density::Pane => lines.push(Self::result(tool)),
                Density::Page => {}
            },
            Block::Reply(reply) => {
                lines.extend(Self::markdown(reply, body).into_iter().map(indented));
            }
            Block::Streaming(reply) => {
                let mut reply = Self::markdown(reply, body);
                // The cursor sits where the stream has got to.
                match reply.last_mut() {
                    Some(last) if last.width() < body => last.push(Span::new("▌", Ink::Text)),
                    Some(_) | None => reply.push(Line::of("▌", Ink::Text)),
                }
                lines.extend(reply.into_iter().map(indented));
            }
            Block::Thinking => lines.push(indented(
                Span::new("thinking…", Ink::Muted)
                    .with(Emphasis::Italic)
                    .into(),
            )),
            Block::Tools(tool) => match density {
                Density::Pane => lines.push(Self::tools(tool)),
                Density::Page => {
                    let end = match &tool.result {
                        ResultMark::Reported(status)
                            if status.known() == Some(ResultStatus::Failed) =>
                        {
                            "failed".to_owned()
                        }
                        ResultMark::Failures(count) => format!("{count} failed"),
                        ResultMark::Reported(_)
                        | ResultMark::Pending
                        | ResultMark::Unreadable
                        | ResultMark::Missing
                        | ResultMark::AllSucceeded(_) => String::new(),
                    };
                    let name = match tool.count {
                        0 | 1 => tool.name.clone(),
                        count => format!("{} ×{count}", tool.name),
                    };
                    let target = tool.target.as_deref().unwrap_or_default();
                    lines.push(Self::step_line("●", &name, target, &end, width));
                }
            },
            Block::Step(step) => {
                let glyph = match step.kind {
                    StepKind::Run => "●",
                    StepKind::Edit => "✎",
                };
                match density {
                    Density::Pane => {
                        let mut line = Line::new(vec![
                            Span::new(format!("{glyph} "), Ink::Tool),
                            Span::new(step.name.clone(), Ink::Tool).with(Emphasis::Bold),
                            Span::new(format!(" {}", step.target), Ink::Tool),
                        ]);
                        if !step.end.is_empty() {
                            line.push(Span::new(format!(" · {}", step.end), Ink::Muted));
                        }
                        lines.push(line);
                    }
                    Density::Page => {
                        lines.push(Self::step_line(
                            glyph,
                            &step.name,
                            &step.target,
                            &step.end,
                            width,
                        ));
                    }
                }
                for output in &step.output {
                    lines.push(Line::of(
                        format!(
                            "{text}       {}",
                            Text(output).summary(width.saturating_sub(12))
                        ),
                        Ink::Muted,
                    ));
                }
            }
            Block::Notice { text: notice, tone } => {
                lines.push(indented(Line::of(format!("! {notice}"), Ink::of(*tone))));
            }
            Block::Waiting => lines.push(indented(Line::of(
                "request sent · waiting for the provider",
                Ink::Muted,
            ))),
        }
    }

    /// `  ● Bash   cargo test          exit 1`: a page's step, its end
    /// against the right edge.
    fn step_line(glyph: &str, name: &str, target: &str, end: &str, width: usize) -> Line {
        let name = format!("  {glyph} {name:<6} ");
        let room = width.saturating_sub(Text(&name).width() + Text(end).width() + 2);
        let target = Text(target.lines().next().unwrap_or_default()).summary(room);
        let gap =
            width.saturating_sub(Text(&name).width() + Text(&target).width() + Text(end).width());
        Line::new(vec![
            Span::new(name, Ink::Tool),
            Span::new(target, Ink::Code),
            Span::new(format!("{}{end}", " ".repeat(gap)), Ink::Muted),
        ])
    }

    /// A reply's text with its markdown read lightly: fenced code drawn as
    /// a block behind a rule, headings and whole `**bold**` lines in bold,
    /// and emphasis markers left out.
    pub fn markdown(text: &str, width: usize) -> Vec<Line> {
        let mut lines = Vec::new();
        let mut in_code = false;
        for source in text.lines() {
            if source.trim_start().starts_with("```") {
                in_code = !in_code;
                continue;
            }
            if in_code {
                for part in Text(source).wrap(width.saturating_sub(2)) {
                    lines.push(Line::new(vec![
                        Span::new("│ ", Ink::Rule),
                        Span::new(part, Ink::Code),
                    ]));
                }
                continue;
            }
            let trimmed = source.trim_start();
            let heading = trimmed.starts_with('#');
            let shown = if heading {
                trimmed.trim_start_matches('#').trim_start()
            } else {
                source
            };
            let bold = heading || (trimmed.starts_with("**") && trimmed.contains(":**"));
            let plain = shown.replace("**", "").replace("__", "");
            let emphasis = if bold {
                Emphasis::Bold
            } else {
                Emphasis::Plain
            };
            for part in Text(&plain).wrap(width) {
                lines.push(Span::new(part, Ink::Text).with(emphasis).into());
            }
        }
        lines
    }

    /// `● Bash cargo test ×2`
    fn tools(tool: &ToolLine) -> Line {
        let mut line = Line::new(vec![
            Span::new("● ", Ink::Tool),
            Span::new(tool.name.clone(), Ink::Tool).with(Emphasis::Bold),
        ]);
        if let Some(target) = &tool.target {
            line.push(Span::new(format!(" {target}"), Ink::Tool));
        }
        if tool.count > 1 {
            line.push(Span::new(format!(" ×{}", tool.count), Ink::Tool));
        }
        line
    }

    /// `⎿ Read src/lib.rs → fn main() {}`: what the client sent back.
    fn result(tool: &ToolLine) -> Line {
        let mut head = format!("⎿ {}", tool.name);
        if let Some(target) = &tool.target {
            head.push(' ');
            head.push_str(target);
        }
        if tool.count > 1 {
            head.push_str(&format!(" ×{}", tool.count));
        }
        let (outcome, ink) = match &tool.result {
            ResultMark::Pending => ("…".to_owned(), Ink::Muted),
            ResultMark::Unreadable => ("result not readable".to_owned(), Ink::Muted),
            ResultMark::Missing => ("no result".to_owned(), Ink::Warning),
            ResultMark::Reported(status) => match status.known() {
                Some(ResultStatus::Succeeded | ResultStatus::Unreported) => (
                    tool.output.clone().unwrap_or_else(|| "ok".to_owned()),
                    Ink::Muted,
                ),
                Some(ResultStatus::Failed) => (
                    tool.output
                        .as_ref()
                        .map_or_else(|| "failed".to_owned(), |output| format!("failed: {output}")),
                    Ink::Error,
                ),
                None => (status.label().replace('_', " "), Ink::Muted),
            },
            ResultMark::Failures(count) => (format!("{count} failed"), Ink::Error),
            ResultMark::AllSucceeded(count) => (format!("{count} ok"), Ink::Muted),
        };
        Line::new(vec![
            Span::new(head, Ink::Muted),
            Span::new(" → ", Ink::Muted),
            Span::new(outcome, ink),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::{Block, Conversation, Density};

    #[test]
    fn replies_read_their_markdown_lightly() {
        let lines: Vec<String> = Conversation::markdown(
            "## Plan\n**What to look for:** the A\n```rust\nlet x = 1;\n```\n- one",
            40,
        )
        .iter()
        .map(crate::ui::Line::text)
        .collect();
        assert_eq!(
            lines,
            ["Plan", "What to look for: the A", "│ let x = 1;", "- one"]
        );
    }

    #[test]
    fn a_page_sets_prompts_apart_and_indents_replies() {
        let conversation = Conversation {
            blocks: vec![
                Block::Call { index: Some(1) },
                Block::Prompt("fix it".into()),
                Block::Reply("Done.".into()),
                Block::Streaming("And".into()),
            ],
        };
        let text = |density| -> Vec<String> {
            conversation
                .lines(40, density)
                .iter()
                .map(crate::ui::Line::text)
                .collect()
        };
        assert_eq!(
            text(Density::Pane),
            ["── call 1 ──", "› fix it", "Done.", "And▌"]
        );
        assert_eq!(
            text(Density::Page),
            ["", "  › fix it", "    Done.", "    And▌"]
        );
    }
}
