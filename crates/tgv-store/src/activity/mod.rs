//! Agent activity, read from stored bodies when asked: for a turn, the
//! user's prompt, each call's tool calls with what they touched and how
//! their results came back, and the files and commands of the whole turn.
//!
//! Derived from captured bytes at read time, never stored, and never part of
//! forwarding. A tool call is what the model requested; a result's status is
//! what the client reported; an effect is what the tool's input names.
//! Turns are the client's own groupings, never inferred.

pub mod report;

use crate::search::SearchIndex;
use crate::secrets::SecretScanner;
use crate::view::{CallView, TokenTotals};
use report::{CallMetrics, Finding, FindingKind, LargeToolOutput, RepeatedFailure, SecretExposure};
use serde::Serialize;
use std::collections::HashSet;
use tgv_harness::{
    Harness,
    tools::{EffectKind, ToolEffect},
};
use tgv_protocol::{
    activity::{
        ActivityLimitation, RequestActivity, ResponseActivity, ResultStatus, ToolInput, ToolResult,
    },
    label::Stored,
};
use tgv_recorder::{
    id::{CallId, SessionId, TurnId},
    time::UnixMillis,
};

/// Text cut to a budget, saying how much was left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Excerpt {
    pub text: String,
    pub omitted_chars: u64,
}

impl Excerpt {
    /// `text`, cut to `max_chars` characters.
    pub fn cut(text: &str, max_chars: usize) -> Self {
        let total = text.chars().count();
        Self {
            text: text.chars().take(max_chars).collect(),
            omitted_chars: u64::try_from(total.saturating_sub(max_chars)).unwrap_or(u64::MAX),
        }
    }

    fn of_input(input: &ToolInput, max_chars: usize) -> Self {
        match input {
            ToolInput::Json(value) => Self::cut(&value.to_string(), max_chars),
            ToolInput::Text(text) => Self::cut(text, max_chars),
        }
    }
}

/// How much text each part of an activity view keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityBudget {
    pub prompt_chars: usize,
    pub reply_chars: usize,
    pub input_chars: usize,
    pub output_chars: usize,
}

impl ActivityBudget {
    /// Enough to see what happened, compact enough to hand to a model.
    pub const DEFAULT: Self = Self {
        prompt_chars: 4_000,
        reply_chars: 1_000,
        input_chars: 400,
        output_chars: 400,
    };
    /// For turn listings: the start of each prompt.
    pub const SUMMARY: Self = Self {
        prompt_chars: 300,
        reply_chars: 0,
        input_chars: 0,
        output_chars: 0,
    };
}

/// A tool result, as the client reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResultView {
    pub status: ResultStatus,
    pub output: Excerpt,
}

/// A tool call a response requested, and its result when a later request in
/// the turn carried one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolCallView {
    pub id: Option<String>,
    pub name: String,
    pub effects: Vec<ToolEffect>,
    pub input: Excerpt,
    /// `None` when no request in the turn answered it.
    pub result: Option<ResultView>,
}

/// What one call shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallActivity {
    pub call: CallId,
    pub started_at: UnixMillis,
    pub ended_at: Option<UnixMillis>,
    pub model: Option<String>,
    pub status: Option<u16>,
    pub outcome: Option<Stored<tgv_protocol::inspect::ModelOutcome>>,
    pub metrics: CallMetrics,
    /// The user's own words, when this request starts with them.
    pub prompt: Option<Excerpt>,
    pub tool_calls: Vec<ToolCallView>,
    /// Results this request carried for tool calls outside the turn.
    pub earlier_results: u64,
    pub reply: Option<Excerpt>,
    pub limitations: Vec<ActivityLimitation>,
}

/// One turn's activity.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TurnActivity {
    pub turn: TurnId,
    pub session: SessionId,
    /// The client's turn identifier.
    pub native_id: String,
    pub client: Option<Harness>,
    pub started_at: Option<UnixMillis>,
    pub ended_at: Option<UnixMillis>,
    pub prompt: Option<Excerpt>,
    pub files_read: Vec<String>,
    pub files_written: Vec<String>,
    pub commands: Vec<String>,
    pub tool_calls: u64,
    /// Tool calls whose result the client reported as failed.
    pub failed_tool_calls: u64,
    /// Tool calls no request in the turn answered.
    pub unanswered_tool_calls: u64,
    /// Large tool outputs and repeated failures in the turn.
    pub findings: Vec<Finding>,
    pub calls: Vec<CallActivity>,
    /// Calls beyond the read limit, left out.
    pub omitted_calls: u64,
}

/// One turn, as a session lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TurnSummary {
    pub turn: TurnId,
    pub native_id: String,
    pub started_at: UnixMillis,
    pub ended_at: Option<UnixMillis>,
    pub call_count: u64,
    pub error_count: u64,
    pub tokens: TokenTotals,
    pub prompt: Option<Excerpt>,
}

/// A turn's identity, before its calls are read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnHead {
    pub turn: TurnId,
    pub session: SessionId,
    pub native_id: String,
}

/// One call with what its bodies showed.
#[derive(Debug, Clone)]
pub struct ReadCall {
    pub view: CallView,
    pub request: RequestActivity,
    pub response: ResponseActivity,
}

impl ReadCall {
    /// The harness whose hints attributed the call.
    pub fn harness(&self) -> Option<Harness> {
        self.view
            .attribution
            .hint
            .as_ref()
            .and_then(|hint| hint.source.known())
            .map(|source| source.harness())
    }

    /// The user's own words in this call's request, if it starts with them.
    pub fn prompt(&self) -> Option<String> {
        self.harness().map_or_else(
            || {
                let text = self.request.user_text.join("\n\n");
                (!text.trim().is_empty()).then_some(text)
            },
            |harness| harness.prompt(&self.request.user_text),
        )
    }

    /// This call alone, with results matched only within it.
    pub fn activity(self, budget: ActivityBudget) -> CallActivity {
        let results = self.results(budget.output_chars);
        let mut answered = Vec::new();
        self.into_activity(&results, &mut answered, budget)
    }

    /// The tool results this request carried, with the status the client
    /// reported, by the tool call each answers.
    fn results(&self, output_chars: usize) -> Vec<(String, ResultView)> {
        let harness = self.harness();
        self.request
            .tool_results
            .iter()
            .map(|result: &ToolResult| {
                (
                    result.id.clone(),
                    ResultView {
                        status: harness
                            .map_or(result.status, |harness| harness.result_status(result)),
                        output: Excerpt::cut(&result.output, output_chars),
                    },
                )
            })
            .collect()
    }

    fn into_activity(
        self,
        results: &[(String, ResultView)],
        answered: &mut Vec<String>,
        budget: ActivityBudget,
    ) -> CallActivity {
        let harness = self.harness();
        let metrics = CallMetrics::of(&self.view);
        let prompt = self
            .prompt()
            .map(|prompt| Excerpt::cut(&prompt, budget.prompt_chars));
        let own_results: Vec<&str> = self
            .request
            .tool_results
            .iter()
            .map(|result| result.id.as_str())
            .collect();
        let earlier_results = own_results
            .iter()
            .filter(|id| !answered.iter().any(|known| known == *id))
            .count();
        let tool_calls = self
            .response
            .tool_uses
            .iter()
            .map(|tool| {
                if let Some(id) = &tool.id {
                    answered.push(id.clone());
                }
                ToolCallView {
                    id: tool.id.clone(),
                    name: tool.name.clone(),
                    effects: harness
                        .map(|harness| harness.effects(tool))
                        .unwrap_or_default(),
                    input: Excerpt::of_input(&tool.input, budget.input_chars),
                    result: tool.id.as_ref().and_then(|id| {
                        results
                            .iter()
                            .find(|(result, _)| result == id)
                            .map(|(_, view)| view.clone())
                    }),
                }
            })
            .collect();
        let reply = (!self.response.reply.trim().is_empty())
            .then(|| Excerpt::cut(&self.response.reply, budget.reply_chars));
        let mut limitations = self.request.limitations;
        for limitation in self.response.limitations {
            if !limitations.contains(&limitation) {
                limitations.push(limitation);
            }
        }
        CallActivity {
            call: self.view.id,
            started_at: self.view.started_at,
            ended_at: self.view.ended_at,
            model: self.view.model,
            status: self.view.status,
            outcome: self.view.outcome,
            metrics,
            prompt,
            tool_calls,
            earlier_results: u64::try_from(earlier_results).unwrap_or(u64::MAX),
            reply,
            limitations,
        }
    }
}

impl TurnActivity {
    /// A turn from its calls, oldest first. Results are matched to the tool
    /// calls they answer anywhere in the turn.
    pub fn assemble(
        head: TurnHead,
        calls: Vec<ReadCall>,
        omitted_calls: u64,
        budget: ActivityBudget,
        scanner: &SecretScanner,
    ) -> Self {
        let client = calls.iter().find_map(ReadCall::harness);
        let exposures: Vec<Finding> = calls
            .iter()
            .flat_map(|call| {
                let documents = call.documents(SearchIndex::FIELD_CHARS, scanner);
                documents.exposures.into_iter().map(move |exposure| {
                    Finding::SecretExposure(SecretExposure {
                        call: documents.call,
                        turn: documents.turn,
                        field: exposure.field.into(),
                        secret: exposure.kind.into(),
                        hint: exposure.hint,
                        fingerprint: exposure.fingerprint,
                        count: exposure.count,
                    })
                })
            })
            .collect();
        let results: Vec<_> = calls
            .iter()
            .flat_map(|call| call.results(budget.output_chars))
            .collect();
        let mut answered = Vec::new();
        let calls: Vec<CallActivity> = calls
            .into_iter()
            .map(|call| call.into_activity(&results, &mut answered, budget))
            .collect();
        let mut turn = Self {
            turn: head.turn,
            session: head.session,
            native_id: head.native_id,
            client,
            started_at: calls.first().map(|call| call.started_at),
            // A turn has ended only once every call it made has.
            ended_at: calls
                .iter()
                .map(|call| call.ended_at)
                .collect::<Option<Vec<_>>>()
                .and_then(|ends| ends.into_iter().max()),
            prompt: calls.iter().find_map(|call| call.prompt.clone()),
            files_read: Vec::new(),
            files_written: Vec::new(),
            commands: Vec::new(),
            tool_calls: 0,
            failed_tool_calls: 0,
            unanswered_tool_calls: 0,
            findings: Self::tool_findings(&calls)
                .into_iter()
                .chain(exposures)
                .collect(),
            calls: Vec::new(),
            omitted_calls,
        };
        let mut seen: HashSet<(EffectKind, String)> = HashSet::new();
        for tool in calls.iter().flat_map(|call| &call.tool_calls) {
            turn.tool_calls = turn.tool_calls.saturating_add(1);
            match tool.result.as_ref().map(|result| result.status) {
                Some(ResultStatus::Failed) => {
                    turn.failed_tool_calls = turn.failed_tool_calls.saturating_add(1);
                }
                None => {
                    turn.unanswered_tool_calls = turn.unanswered_tool_calls.saturating_add(1);
                }
                Some(ResultStatus::Succeeded | ResultStatus::Unreported) => {}
            }
            for effect in &tool.effects {
                if !seen.insert((effect.kind, effect.target.clone())) {
                    continue;
                }
                let list = match effect.kind {
                    EffectKind::ReadFile => &mut turn.files_read,
                    EffectKind::WriteFile => &mut turn.files_written,
                    EffectKind::RunCommand => &mut turn.commands,
                    EffectKind::Search | EffectKind::Delegate | EffectKind::Fetch => continue,
                };
                list.push(effect.target.clone());
            }
        }
        turn.calls = calls;
        turn
    }

    /// Tool outputs of at least [`FindingKind::LARGE_OUTPUT_CHARS`], and
    /// tools that failed [`FindingKind::REPEATED_FAILURES`] times or more
    /// on one target, in the order they first appeared.
    fn tool_findings(calls: &[CallActivity]) -> Vec<Finding> {
        let mut findings = Vec::new();
        let mut failures: Vec<(String, Option<String>, Vec<CallId>)> = Vec::new();
        for call in calls {
            for tool in &call.tool_calls {
                let Some(result) = &tool.result else {
                    continue;
                };
                let target = tool.effects.first().map(|effect| effect.target.clone());
                let chars = u64::try_from(result.output.text.chars().count())
                    .unwrap_or(u64::MAX)
                    .saturating_add(result.output.omitted_chars);
                if chars >= FindingKind::LARGE_OUTPUT_CHARS {
                    findings.push(Finding::LargeToolOutput(LargeToolOutput {
                        call: call.call,
                        tool: tool.name.clone(),
                        tool_call_id: tool.id.clone(),
                        target: target.clone(),
                        output_chars: chars,
                    }));
                }
                if result.status == ResultStatus::Failed {
                    match failures
                        .iter_mut()
                        .find(|(name, known, _)| *name == tool.name && *known == target)
                    {
                        Some((_, _, calls)) => calls.push(call.call),
                        None => failures.push((tool.name.clone(), target, vec![call.call])),
                    }
                }
            }
        }
        findings.extend(failures.into_iter().filter_map(|(tool, target, calls)| {
            let failures = u64::try_from(calls.len()).unwrap_or(u64::MAX);
            (failures >= FindingKind::REPEATED_FAILURES).then_some(Finding::RepeatedFailure(
                RepeatedFailure {
                    tool,
                    target,
                    failures,
                    calls,
                },
            ))
        }));
        findings
    }
}

#[cfg(test)]
mod tests;
