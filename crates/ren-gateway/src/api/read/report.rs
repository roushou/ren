//! A recording's report: its totals, its calls and turns, and its findings.
//! Each type converts once from the type it reports (`From`), naming every
//! field.

use ren_protocol::label::Stored;
use ren_recorder::id::CallId;
use ren_recorder::id::RecordingId;
use ren_recorder::id::TurnId;
use ren_recorder::secrets::SecretKind;
use ren_recorder::time::UnixMillis;
use ren_store::search::SearchField;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One call's numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CallMetrics {
    /// The prompt the model saw, in tokens: new input plus what was read
    /// from and written to the cache.
    pub context_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_written_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub request_bytes: Option<u64>,
    pub duration_ms: Option<u64>,
}

impl From<ren_store::activity::report::CallMetrics> for CallMetrics {
    fn from(value: ren_store::activity::report::CallMetrics) -> Self {
        let ren_store::activity::report::CallMetrics {
            context_tokens,
            cache_read_tokens,
            cache_written_tokens,
            output_tokens,
            request_bytes,
            duration_ms,
        } = value;
        Self {
            context_tokens,
            cache_read_tokens,
            cache_written_tokens,
            output_tokens,
            request_bytes,
            duration_ms,
        }
    }
}

/// A call read less than half of the earlier peak context from the cache
/// and wrote at least half of it, and at least `FindingKind::REBUILD_TOKENS`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CacheRebuild {
    #[ts(as = "String")]
    pub call: CallId,
    #[ts(as = "Option<String>")]
    pub turn: Option<TurnId>,
    pub cache_read_tokens: u64,
    pub cache_written_tokens: u64,
    /// The largest context of the recording's earlier calls.
    pub earlier_peak_tokens: u64,
    /// Time since the recording's previous call ended.
    pub idle_before_ms: Option<u64>,
}

impl From<ren_store::activity::report::CacheRebuild> for CacheRebuild {
    fn from(value: ren_store::activity::report::CacheRebuild) -> Self {
        let ren_store::activity::report::CacheRebuild {
            call,
            turn,
            cache_read_tokens,
            cache_written_tokens,
            earlier_peak_tokens,
            idle_before_ms,
        } = value;
        Self {
            call,
            turn,
            cache_read_tokens,
            cache_written_tokens,
            earlier_peak_tokens,
            idle_before_ms,
        }
    }
}

/// A call's context passed the recording's earlier peak by at least
/// `FindingKind::GROWTH_TOKENS`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ContextGrowth {
    #[ts(as = "String")]
    pub call: CallId,
    #[ts(as = "Option<String>")]
    pub turn: Option<TurnId>,
    pub earlier_peak_tokens: u64,
    pub context_tokens: u64,
}

impl From<ren_store::activity::report::ContextGrowth> for ContextGrowth {
    fn from(value: ren_store::activity::report::ContextGrowth) -> Self {
        let ren_store::activity::report::ContextGrowth {
            call,
            turn,
            earlier_peak_tokens,
            context_tokens,
        } = value;
        Self {
            call,
            turn,
            earlier_peak_tokens,
            context_tokens,
        }
    }
}

/// A tool result of at least `FindingKind::LARGE_OUTPUT_CHARS`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LargeToolOutput {
    /// The call whose response requested the tool.
    #[ts(as = "String")]
    pub call: CallId,
    pub tool: String,
    pub tool_call_id: Option<String>,
    /// The tool's target, when its input names one.
    pub target: Option<String>,
    pub output_chars: u64,
}

impl From<ren_store::activity::report::LargeToolOutput> for LargeToolOutput {
    fn from(value: ren_store::activity::report::LargeToolOutput) -> Self {
        let ren_store::activity::report::LargeToolOutput {
            call,
            tool,
            tool_call_id,
            target,
            output_chars,
        } = value;
        Self {
            call,
            tool,
            tool_call_id,
            target,
            output_chars,
        }
    }
}

/// At least `FindingKind::REPEATED_FAILURES` reported failures of one tool on
/// one target in a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RepeatedFailure {
    pub tool: String,
    pub target: Option<String>,
    pub failures: u64,
    /// The calls that requested the failed tool calls, in order.
    #[ts(as = "Vec<String>")]
    pub calls: Vec<CallId>,
}

impl From<ren_store::activity::report::RepeatedFailure> for RepeatedFailure {
    fn from(value: ren_store::activity::report::RepeatedFailure) -> Self {
        let ren_store::activity::report::RepeatedFailure {
            tool,
            target,
            failures,
            calls,
        } = value;
        Self {
            tool,
            target,
            failures,
            calls,
        }
    }
}

/// A credential a call carried: in the user's input, a tool's input or
/// output (all sent to the provider), or the model's reply. Never the
/// secret itself: its format, a masked hint, and a fingerprint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecretExposure {
    #[ts(as = "String")]
    pub call: CallId,
    #[ts(as = "Option<String>")]
    pub turn: Option<TurnId>,
    #[ts(as = "String")]
    pub field: Stored<SearchField>,
    #[ts(as = "String")]
    pub secret: Stored<SecretKind>,
    pub hint: String,
    pub fingerprint: String,
    pub count: u64,
}

impl From<ren_store::activity::report::SecretExposure> for SecretExposure {
    fn from(value: ren_store::activity::report::SecretExposure) -> Self {
        let ren_store::activity::report::SecretExposure {
            call,
            turn,
            field,
            secret,
            hint,
            fingerprint,
            count,
        } = value;
        Self {
            call,
            turn,
            field,
            secret,
            hint,
            fingerprint,
            count,
        }
    }
}

/// One turn's row in a recording report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnMetrics {
    #[ts(as = "String")]
    pub turn: TurnId,
    #[ts(as = "u64")]
    pub started_at: UnixMillis,
    #[ts(as = "Option<u64>")]
    pub ended_at: Option<UnixMillis>,
    pub call_count: u64,
    /// Time between the recording's previous call ending and this turn.
    pub idle_before_ms: Option<u64>,
    pub context_start_tokens: Option<u64>,
    pub context_end_tokens: Option<u64>,
    /// Cache reads over context, in percent, over calls reporting both.
    pub cache_hit_percent: Option<u64>,
    pub cache_written_tokens: u64,
    pub output_tokens: u64,
}

impl From<ren_store::activity::report::TurnMetrics> for TurnMetrics {
    fn from(value: ren_store::activity::report::TurnMetrics) -> Self {
        let ren_store::activity::report::TurnMetrics {
            turn,
            started_at,
            ended_at,
            call_count,
            idle_before_ms,
            context_start_tokens,
            context_end_tokens,
            cache_hit_percent,
            cache_written_tokens,
            output_tokens,
        } = value;
        Self {
            turn,
            started_at,
            ended_at,
            call_count,
            idle_before_ms,
            context_start_tokens,
            context_end_tokens,
            cache_hit_percent,
            cache_written_tokens,
            output_tokens,
        }
    }
}

/// Totals over a recording's calls that reported usage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReportTotals {
    pub calls: u64,
    pub calls_with_usage: u64,
    pub peak_context_tokens: u64,
    pub context_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_written_tokens: u64,
    pub output_tokens: u64,
    pub cache_hit_percent: Option<u64>,
}

impl From<ren_store::activity::report::ReportTotals> for ReportTotals {
    fn from(value: ren_store::activity::report::ReportTotals) -> Self {
        let ren_store::activity::report::ReportTotals {
            calls,
            calls_with_usage,
            peak_context_tokens,
            context_tokens,
            cache_read_tokens,
            cache_written_tokens,
            output_tokens,
            cache_hit_percent,
        } = value;
        Self {
            calls,
            calls_with_usage,
            peak_context_tokens,
            context_tokens,
            cache_read_tokens,
            cache_written_tokens,
            output_tokens,
            cache_hit_percent,
        }
    }
}

/// A recording's context and cost measurements, from its call records alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RecordingReport {
    #[ts(as = "String")]
    pub recording: RecordingId,
    pub totals: ReportTotals,
    pub turns: Vec<TurnMetrics>,
    pub findings: Vec<Finding>,
    /// Calls beyond the read limit, left out.
    pub omitted_calls: u64,
}

impl From<ren_store::activity::report::RecordingReport> for RecordingReport {
    fn from(value: ren_store::activity::report::RecordingReport) -> Self {
        let ren_store::activity::report::RecordingReport {
            recording,
            totals,
            turns,
            findings,
            omitted_calls,
        } = value;
        Self {
            recording,
            totals: totals.into(),
            turns: turns.into_iter().map(Into::into).collect(),
            findings: findings.into_iter().map(Into::into).collect(),
            omitted_calls,
        }
    }
}

/// One finding: its `kind` label, then its fields. A reader of the API keeps
/// a finding of a kind it does not know as its fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum Finding {
    Known(KnownFinding),
    /// A kind this binary does not know: its `kind` and its other fields.
    #[ts(skip)]
    Unknown(serde_json::Map<String, serde_json::Value>),
}

/// A finding of a kind this binary knows, by its `FindingKind` label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum KnownFinding {
    CacheRebuild(CacheRebuild),
    ContextGrowth(ContextGrowth),
    LargeToolOutput(LargeToolOutput),
    RepeatedFailure(RepeatedFailure),
    SecretExposure(SecretExposure),
}

impl From<ren_store::activity::report::Finding> for Finding {
    fn from(finding: ren_store::activity::report::Finding) -> Self {
        use ren_store::activity::report::Finding as Source;
        match finding {
            Source::CacheRebuild(detail) => Self::Known(KnownFinding::CacheRebuild(detail.into())),
            Source::ContextGrowth(detail) => {
                Self::Known(KnownFinding::ContextGrowth(detail.into()))
            }
            Source::LargeToolOutput(detail) => {
                Self::Known(KnownFinding::LargeToolOutput(detail.into()))
            }
            Source::RepeatedFailure(detail) => {
                Self::Known(KnownFinding::RepeatedFailure(detail.into()))
            }
            Source::SecretExposure(detail) => {
                Self::Known(KnownFinding::SecretExposure(detail.into()))
            }
            Source::Unknown(raw) => {
                let mut fields = serde_json::Map::new();
                fields.insert("kind".to_owned(), raw.kind().label().into());
                fields.extend(raw.detail().clone());
                Self::Unknown(fields)
            }
        }
    }
}
