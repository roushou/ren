//! A call's and a turn's activity: tool calls with their results and effects,
//! and a turn's summary and findings. Each type converts once from the type it
//! reports (`From`), naming every field.

use super::calls::TokenTotals;
use super::report::{CallMetrics, Finding};
use ren_harness::Harness;
use ren_harness::tools::EffectKind;
use ren_protocol::activity::ActivityLimitation;
use ren_protocol::activity::ResultStatus;
use ren_protocol::label::Stored;
use ren_recorder::id::CallId;
use ren_recorder::id::RecordingId;
use ren_recorder::id::TurnId;
use ren_recorder::time::UnixMillis;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A tool result, as the client reported it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResultView {
    #[ts(as = "String")]
    pub status: Stored<ResultStatus>,
    pub output: Excerpt,
}

impl From<ren_store::activity::ResultView> for ResultView {
    fn from(value: ren_store::activity::ResultView) -> Self {
        let ren_store::activity::ResultView { status, output } = value;
        Self {
            status,
            output: output.into(),
        }
    }
}

/// What a tool's input names, as a view: a known kind of effect, or the
/// label of one this binary does not know.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EffectView {
    #[ts(as = "String")]
    pub kind: Stored<EffectKind>,
    pub target: String,
}

impl From<ren_store::activity::EffectView> for EffectView {
    fn from(value: ren_store::activity::EffectView) -> Self {
        let ren_store::activity::EffectView { kind, target } = value;
        Self { kind, target }
    }
}

/// A tool call a response requested, and its result when a later request in
/// the turn carried one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolCallView {
    pub id: Option<String>,
    pub name: String,
    pub effects: Vec<EffectView>,
    pub input: Excerpt,
    /// `None` when no request in the turn answered it.
    pub result: Option<ResultView>,
}

impl From<ren_store::activity::ToolCallView> for ToolCallView {
    fn from(value: ren_store::activity::ToolCallView) -> Self {
        let ren_store::activity::ToolCallView {
            id,
            name,
            effects,
            input,
            result,
        } = value;
        Self {
            id,
            name,
            effects: effects.into_iter().map(Into::into).collect(),
            input: input.into(),
            result: result.map(Into::into),
        }
    }
}

/// What one call shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CallActivity {
    #[ts(as = "String")]
    pub call: CallId,
    #[ts(as = "u64")]
    pub started_at: UnixMillis,
    #[ts(as = "Option<u64>")]
    pub ended_at: Option<UnixMillis>,
    pub model: Option<String>,
    pub status: Option<u16>,
    #[ts(as = "Option<String>")]
    pub outcome: Option<Stored<ren_protocol::inspect::ModelOutcome>>,
    pub metrics: CallMetrics,
    /// The user's own words, when this request starts with them.
    pub prompt: Option<Excerpt>,
    pub tool_calls: Vec<ToolCallView>,
    /// Results this request carried for tool calls outside the turn.
    pub earlier_results: u64,
    pub reply: Option<Excerpt>,
    #[ts(as = "Vec<String>")]
    pub limitations: Vec<Stored<ActivityLimitation>>,
}

impl From<ren_store::activity::CallActivity> for CallActivity {
    fn from(value: ren_store::activity::CallActivity) -> Self {
        let ren_store::activity::CallActivity {
            call,
            started_at,
            ended_at,
            model,
            status,
            outcome,
            metrics,
            prompt,
            tool_calls,
            earlier_results,
            reply,
            limitations,
        } = value;
        Self {
            call,
            started_at,
            ended_at,
            model,
            status,
            outcome,
            metrics: metrics.into(),
            prompt: prompt.map(Into::into),
            tool_calls: tool_calls.into_iter().map(Into::into).collect(),
            earlier_results,
            reply: reply.map(Into::into),
            limitations,
        }
    }
}

/// One turn's activity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnActivity {
    #[ts(as = "String")]
    pub turn: TurnId,
    #[ts(as = "String")]
    pub recording: RecordingId,
    /// The client's turn identifier.
    pub native_id: String,
    #[ts(as = "Option<String>")]
    pub client: Option<Stored<Harness>>,
    #[ts(as = "Option<u64>")]
    pub started_at: Option<UnixMillis>,
    #[ts(as = "Option<u64>")]
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

impl From<ren_store::activity::TurnActivity> for TurnActivity {
    fn from(value: ren_store::activity::TurnActivity) -> Self {
        let ren_store::activity::TurnActivity {
            turn,
            recording,
            native_id,
            client,
            started_at,
            ended_at,
            prompt,
            files_read,
            files_written,
            commands,
            tool_calls,
            failed_tool_calls,
            unanswered_tool_calls,
            findings,
            calls,
            omitted_calls,
        } = value;
        Self {
            turn,
            recording,
            native_id,
            client,
            started_at,
            ended_at,
            prompt: prompt.map(Into::into),
            files_read,
            files_written,
            commands,
            tool_calls,
            failed_tool_calls,
            unanswered_tool_calls,
            findings: findings.into_iter().map(Into::into).collect(),
            calls: calls.into_iter().map(Into::into).collect(),
            omitted_calls,
        }
    }
}

/// One turn, as a recording lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnSummary {
    #[ts(as = "String")]
    pub turn: TurnId,
    pub native_id: String,
    #[ts(as = "u64")]
    pub started_at: UnixMillis,
    #[ts(as = "Option<u64>")]
    pub ended_at: Option<UnixMillis>,
    pub call_count: u64,
    pub error_count: u64,
    pub tokens: TokenTotals,
    pub prompt: Option<Excerpt>,
}

impl From<ren_store::activity::TurnSummary> for TurnSummary {
    fn from(value: ren_store::activity::TurnSummary) -> Self {
        let ren_store::activity::TurnSummary {
            turn,
            native_id,
            started_at,
            ended_at,
            call_count,
            error_count,
            tokens,
            prompt,
        } = value;
        Self {
            turn,
            native_id,
            started_at,
            ended_at,
            call_count,
            error_count,
            tokens: tokens.into(),
            prompt: prompt.map(Into::into),
        }
    }
}

/// Text cut to a budget, saying how much was left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Excerpt {
    pub text: String,
    pub omitted_chars: u64,
}

impl From<ren_recorder::digest::Excerpt> for Excerpt {
    fn from(value: ren_recorder::digest::Excerpt) -> Self {
        let ren_recorder::digest::Excerpt {
            text,
            omitted_chars,
        } = value;
        Self {
            text,
            omitted_chars,
        }
    }
}
