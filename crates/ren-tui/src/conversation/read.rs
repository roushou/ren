//! Reading recorded model calls as a conversation: each call's prompt or
//! the results its request carried back, its reply, and the tool calls it
//! asked for, like ones in a row merged, with paths relative to the
//! directory the turn's commands work in.

use super::{Block, Conversation};
use ren_harness::tools::EffectKind;
use ren_protocol::activity::{ActivityLimitation, ResultStatus};
use ren_protocol::label::Stored;
use ren_recorder::id::CallId;
use ren_store::activity::{CallActivity, ToolCallView, TurnActivity};
use std::collections::HashMap;

/// How a tool call's result reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultMark {
    /// No later call of the turn has been read: the result is still to come.
    Pending,
    /// The request that would carry it could not be read whole.
    Unreadable,
    /// A later call was read and did not answer it.
    Missing,
    Reported(Stored<ResultStatus>),
    /// A run of like calls with this many failures.
    Failures(usize),
    /// A run of like calls that all succeeded.
    AllSucceeded(usize),
}

/// One tool call, or a run of like ones with nothing said between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLine {
    pub name: String,
    pub effect: Option<Stored<EffectKind>>,
    /// What it touched, relative to the turn's working directory.
    pub target: Option<String>,
    pub result: ResultMark,
    /// The first line of what the client reported back.
    pub output: Option<String>,
    /// How many like calls this line stands for.
    pub count: usize,
}

impl ToolLine {
    /// A tool call known only by its name, while its call streams.
    pub fn named(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            effect: None,
            target: None,
            result: ResultMark::Pending,
            output: None,
            count: 1,
        }
    }
}

impl Conversation {
    /// The most blocks a reading keeps; the newest are kept.
    pub const MAX_BLOCKS: usize = 60;

    /// The newest turn's lines: each read call's start, its reply, then its
    /// tool calls, like ones in a row merged. Calls whose text is not
    /// derived yet are left to `streaming`.
    /// A turn's calls as a conversation, led by the turn's prompt when no
    /// call it kept carries it (its first calls were left out, or their
    /// requests could not be read).
    pub fn turn(turn: &TurnActivity, order: &HashMap<CallId, usize>) -> Self {
        let calls: Vec<&CallActivity> = turn.calls.iter().collect();
        let mut read = Self::calls(&calls, Workdir::of(turn).as_ref(), order);
        let prompted = read
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Prompt(_)));
        if let Some(prompt) = turn
            .prompt
            .as_ref()
            .map(|prompt| prompt.text.trim())
            .filter(|text| !prompted && !text.is_empty())
        {
            let at = usize::from(matches!(read.blocks.first(), Some(Block::Call { .. })));
            read.blocks.insert(at, Block::Prompt(prompt.to_owned()));
        }
        read
    }

    /// Calls as a conversation, oldest first: each read call's rule, the
    /// prompt or the results its request carried back, its reply, and its
    /// tool calls. Calls whose text is not derived yet are left out.
    pub fn calls(
        calls: &[&CallActivity],
        workdir: Option<&Workdir>,
        order: &HashMap<CallId, usize>,
    ) -> Self {
        let mut lines = Vec::new();
        // The previous read call's tool lines: this call's request carries
        // their results back.
        let mut asked: Vec<ToolLine> = Vec::new();
        for (index, call) in calls.iter().copied().enumerate() {
            if Self::underived(call) {
                continue;
            }
            lines.push(Block::Call {
                index: order.get(&call.call).copied(),
            });
            match call
                .prompt
                .as_ref()
                .map(|prompt| prompt.text.trim())
                .filter(|text| !text.is_empty())
            {
                Some(prompt) => lines.push(Block::Prompt(prompt.to_owned())),
                None => lines.extend(
                    Self::runs(
                        std::mem::take(&mut asked)
                            .into_iter()
                            .filter(|tool| tool.result != ResultMark::Pending),
                    )
                    .into_iter()
                    .map(Block::Result),
                ),
            }
            asked.clear();
            if let Some(reply) = call
                .reply
                .as_ref()
                .map(|reply| reply.text.trim())
                .filter(|text| !text.is_empty())
            {
                lines.push(Block::Reply(reply.to_owned()));
            }
            let later = calls.get(index + 1..).unwrap_or_default();
            asked = call
                .tool_calls
                .iter()
                .map(|tool| Self::tool_line(tool, later, workdir))
                .collect();
            lines.extend(
                Self::runs(asked.iter().cloned())
                    .into_iter()
                    .map(Block::Tools),
            );
            // A call with nothing to show keeps no rule of its own.
            if matches!(lines.last(), Some(Block::Call { .. })) {
                lines.pop();
            }
        }
        let skip = lines.len().saturating_sub(Self::MAX_BLOCKS);
        Self {
            blocks: lines.into_iter().skip(skip).collect(),
        }
    }

    /// Like tool calls in a row merged into one line each.
    fn runs(tools: impl IntoIterator<Item = ToolLine>) -> Vec<ToolLine> {
        let mut lines = Vec::new();
        let mut run: Option<Run> = None;
        for line in tools {
            run = Some(match run {
                Some(mut same) if same.takes(&line) => {
                    same.push(line);
                    same
                }
                Some(other) => {
                    lines.push(other.finish());
                    Run::new(line)
                }
                None => Run::new(line),
            });
        }
        lines.extend(run.map(Run::finish));
        lines
    }

    pub fn underived(call: &CallActivity) -> bool {
        call.limitations
            .iter()
            .any(|limitation| limitation.known() == Some(ActivityLimitation::NotYetDerived))
    }

    /// Whether Ren could not read a call's text whole.
    fn unreadable(call: &CallActivity) -> bool {
        call.limitations
            .iter()
            .any(|limitation| match limitation.known() {
                Some(
                    ActivityLimitation::CaptureGap
                    | ActivityLimitation::Truncated
                    | ActivityLimitation::NotJson
                    | ActivityLimitation::DecodedTooLarge
                    | ActivityLimitation::UnsupportedEncoding
                    | ActivityLimitation::DecodingError
                    | ActivityLimitation::UnexpectedShape,
                ) => true,
                Some(
                    ActivityLimitation::UnsupportedEndpoint
                    | ActivityLimitation::UnfinishedToolCall
                    | ActivityLimitation::TooManyToolCalls
                    | ActivityLimitation::NotYetDerived,
                )
                | None => false,
            })
    }

    fn tool_line(
        tool: &ToolCallView,
        later: &[&CallActivity],
        workdir: Option<&Workdir>,
    ) -> ToolLine {
        let effect = tool.effects.first();
        // A result rides in the client's next request: until a later call
        // is read, a missing one is still to come.
        let result = match &tool.result {
            Some(result) => ResultMark::Reported(result.status.clone()),
            None if later.iter().all(|call| Self::underived(call)) => ResultMark::Pending,
            None if later.first().is_some_and(|call| Self::unreadable(call)) => {
                ResultMark::Unreadable
            }
            None => ResultMark::Missing,
        };
        ToolLine {
            name: tool.name.clone(),
            effect: effect.map(|effect| effect.kind.clone()),
            target: effect.map(|effect| match workdir {
                Some(workdir) => workdir.shorten(&effect.target),
                None => effect.target.clone(),
            }),
            result,
            output: tool
                .result
                .as_ref()
                .and_then(|result| Self::first_line(&result.output.text)),
            count: 1,
        }
    }

    /// The first line with text in it, cut to 160 characters.
    fn first_line(text: &str) -> Option<String> {
        let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
        Some(line.chars().take(160).collect())
    }
}

/// Like tool calls in a row, with nothing said between them: never empty.
struct Run {
    newest: ToolLine,
    count: usize,
    failed: usize,
    succeeded: usize,
}

impl Run {
    fn new(line: ToolLine) -> Self {
        let mut run = Self {
            count: 0,
            failed: 0,
            succeeded: 0,
            newest: line.clone(),
        };
        run.push(line);
        run
    }

    /// Whether `line` is like the run's calls: the same tool, the same kind
    /// of effect.
    fn takes(&self, line: &ToolLine) -> bool {
        self.newest.name == line.name && self.newest.effect == line.effect
    }

    fn push(&mut self, line: ToolLine) {
        self.count += 1;
        match &line.result {
            ResultMark::Reported(status) => match status.known() {
                Some(ResultStatus::Failed) => self.failed += 1,
                Some(ResultStatus::Succeeded) => self.succeeded += 1,
                Some(ResultStatus::Unreported) | None => {}
            },
            ResultMark::Pending
            | ResultMark::Unreadable
            | ResultMark::Missing
            | ResultMark::Failures(_)
            | ResultMark::AllSucceeded(_) => {}
        }
        self.newest = line;
    }

    /// The run as one line: the newest call, the count, and a tally of
    /// their results when there is more than one.
    fn finish(self) -> ToolLine {
        let mut line = self.newest;
        if self.count > 1 {
            if self.failed > 0 {
                line.result = ResultMark::Failures(self.failed);
            } else if self.succeeded == self.count {
                line.result = ResultMark::AllSucceeded(self.count);
            }
        }
        line.count = self.count;
        line
    }
}

/// The directory most of a turn's commands change into first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Workdir(String);

/// `cd <dir> && …` or `cd <dir>; …` at the start of a command.
#[derive(Debug, PartialEq, Eq)]
struct ChangeDirectory<'a> {
    dir: &'a str,
    rest: &'a str,
}

impl<'a> ChangeDirectory<'a> {
    fn parse(command: &'a str) -> Option<Self> {
        let after = command.strip_prefix("cd")?;
        let trimmed = after.trim_start();
        if trimmed.len() == after.len() {
            return None;
        }
        let (dir, rest) = match trimmed.chars().next()? {
            quote @ ('"' | '\'') => {
                let inner = &trimmed[1..];
                let end = inner.find(quote)?;
                (&inner[..end], &inner[end + 1..])
            }
            _ => {
                let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
                let token = &trimmed[..end];
                // `cd dir&&…` or `cd dir;…` without a space.
                match token.find("&&").or_else(|| token.find(';')) {
                    Some(at) => (&token[..at], &trimmed[at..]),
                    None => (token, &trimmed[end..]),
                }
            }
        };
        if dir.is_empty() {
            return None;
        }
        let rest = rest.trim_start();
        let rest = rest
            .strip_prefix("&&")
            .or_else(|| rest.strip_prefix(';'))?
            .trim_start();
        Some(Self { dir, rest })
    }
}

impl Workdir {
    fn of(turn: &TurnActivity) -> Option<Self> {
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for command in &turn.commands {
            let Some(cd) = ChangeDirectory::parse(command) else {
                continue;
            };
            match counts.iter_mut().find(|(dir, _)| *dir == cd.dir) {
                Some((_, count)) => *count += 1,
                None => counts.push((cd.dir, 1)),
            }
        }
        // The first of the most frequent, as the order they appeared.
        let mut best: Option<(&str, usize)> = None;
        for (dir, count) in counts {
            if best.is_none_or(|(_, most)| count > most) {
                best = Some((dir, count));
            }
        }
        best.map(|(dir, _)| Self(dir.to_owned()))
    }

    /// A target without a `cd` into the workdir, with paths relative to it.
    fn shorten(&self, target: &str) -> String {
        let text = match ChangeDirectory::parse(target) {
            Some(cd) if cd.dir == self.0 => cd.rest,
            Some(_) | None => target,
        };
        let root = self.0.trim_end_matches('/');
        if root.is_empty() {
            return text.to_owned();
        }
        text.replace(&format!("{root}/"), "").replace(root, ".")
    }
}

#[cfg(test)]
mod tests {
    use super::ChangeDirectory;

    #[test]
    fn a_leading_cd_is_read_with_or_without_quotes() {
        let cd = |text| ChangeDirectory::parse(text).map(|cd| (cd.dir, cd.rest));
        assert_eq!(
            cd("cd /w/app && cargo test"),
            Some(("/w/app", "cargo test"))
        );
        assert_eq!(cd("cd \"/w/my app\"; ls"), Some(("/w/my app", "ls")));
        assert_eq!(cd("cd '/w/a' && ls"), Some(("/w/a", "ls")));
        assert_eq!(cd("cd /w/app&&ls"), Some(("/w/app", "ls")));
        assert_eq!(cd("cd /w/app"), None);
        assert_eq!(cd("cdx /w && ls"), None);
        assert_eq!(cd("ls && cd /w"), None);
    }
}
