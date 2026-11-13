//! Context and cost measurements over a session's calls, and the findings
//! they support: where the provider's prompt cache was rebuilt, where
//! context grew most, which tool outputs were largest, and which tool calls
//! kept failing.
//!
//! Every number is provider-reported or recorded by Ren; none is cost or
//! subscription allowance. Findings state measurements against fixed,
//! documented thresholds, never causes: Ren cannot see the provider's cache.

use crate::search::{ExposureView, SearchField};
use crate::view::CallView;
use ren_protocol::inspect::InputAccounting;
use ren_protocol::label::Stored;
use ren_recorder::secrets::SecretKind;
use ren_recorder::{
    id::{CallId, RecordingId, TurnId},
    time::UnixMillis,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// One call's numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl CallMetrics {
    pub fn of(call: &CallView) -> Self {
        let usage = call.usage.as_ref();
        let context_tokens = usage.and_then(|usage| {
            let input = usage.input_tokens?;
            match usage.accounting.known()? {
                InputAccounting::IncludesCache => Some(input),
                InputAccounting::ExcludesCache => Some(
                    input
                        .saturating_add(usage.cache_read_tokens.unwrap_or(0))
                        .saturating_add(usage.cache_creation_tokens.unwrap_or(0)),
                ),
            }
        });
        Self {
            context_tokens,
            cache_read_tokens: usage.and_then(|usage| usage.cache_read_tokens),
            cache_written_tokens: usage.and_then(|usage| usage.cache_creation_tokens),
            output_tokens: usage.and_then(|usage| usage.output_tokens),
            request_bytes: call.request.as_ref().map(|body| body.stored_bytes),
            duration_ms: call
                .ended_at
                .map(|end| end.as_millis().saturating_sub(call.started_at.as_millis())),
        }
    }
}

/// What a finding reports.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum FindingKind {
    /// A call wrote most of the context to the cache again.
    CacheRebuild,
    /// Context grew past its earlier peak by a large step.
    ContextGrowth,
    /// A tool returned a large output, which every later call carries.
    LargeToolOutput,
    /// The same tool on the same target failed again and again in a turn.
    RepeatedFailure,
    /// A credential in a well-known format was in what a call carried.
    SecretExposure,
}

/// A call read less than half of the earlier peak context from the cache
/// and wrote at least half of it, and at least [`FindingKind::REBUILD_TOKENS`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheRebuild {
    pub call: CallId,
    pub turn: Option<TurnId>,
    pub cache_read_tokens: u64,
    pub cache_written_tokens: u64,
    /// The largest context of the session's earlier calls.
    pub earlier_peak_tokens: u64,
    /// Time since the session's previous call ended.
    pub idle_before_ms: Option<u64>,
}

/// A call's context passed the session's earlier peak by at least
/// [`FindingKind::GROWTH_TOKENS`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextGrowth {
    pub call: CallId,
    pub turn: Option<TurnId>,
    pub earlier_peak_tokens: u64,
    pub context_tokens: u64,
}

/// A tool result of at least [`FindingKind::LARGE_OUTPUT_CHARS`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeToolOutput {
    /// The call whose response requested the tool.
    pub call: CallId,
    pub tool: String,
    pub tool_call_id: Option<String>,
    /// The tool's target, when its input names one.
    pub target: Option<String>,
    pub output_chars: u64,
}

/// At least [`FindingKind::REPEATED_FAILURES`] reported failures of one tool on
/// one target in a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepeatedFailure {
    pub tool: String,
    pub target: Option<String>,
    pub failures: u64,
    /// The calls that requested the failed tool calls, in order.
    pub calls: Vec<CallId>,
}

/// A credential a call carried: in the user's input, a tool's input or
/// output (all sent to the provider), or the model's reply. Never the
/// secret itself: its format, a masked hint, and a fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretExposure {
    pub call: CallId,
    pub turn: Option<TurnId>,
    pub field: Stored<SearchField>,
    pub secret: Stored<SecretKind>,
    pub hint: String,
    pub fingerprint: String,
    pub count: u64,
}

/// One finding, serialized with its `kind` label first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    CacheRebuild(CacheRebuild),
    ContextGrowth(ContextGrowth),
    LargeToolOutput(LargeToolOutput),
    RepeatedFailure(RepeatedFailure),
    SecretExposure(SecretExposure),
    /// A finding of a kind this binary does not know, read from a newer
    /// one: kept as read.
    Unknown(RawFinding),
}

/// A finding of an unknown kind: its label and the rest of its fields,
/// verbatim. Only reading one makes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFinding {
    kind: Stored<FindingKind>,
    detail: serde_json::Map<String, serde_json::Value>,
}

impl RawFinding {
    pub fn kind(&self) -> &Stored<FindingKind> {
        &self.kind
    }

    /// Its fields other than `kind`.
    pub fn detail(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.detail
    }
}

impl FindingKind {
    /// An earlier peak below this is the session's start, such as a small
    /// side call before the first real one: nothing to rebuild or grow from.
    pub const MIN_PEAK_TOKENS: u64 = 10_000;
    /// Fewer tokens written are not reported as a rebuild.
    pub const REBUILD_TOKENS: u64 = 10_000;
    /// Context steps smaller than this are not reported.
    pub const GROWTH_TOKENS: u64 = 10_000;
    /// The most context-growth findings a report lists, largest first.
    pub const MAX_GROWTH: usize = 5;
    /// Tool outputs of at least this many characters are reported.
    pub const LARGE_OUTPUT_CHARS: u64 = 20_000;
    /// Failures of one tool on one target reported as repeated.
    pub const REPEATED_FAILURES: u64 = 3;
}

impl Finding {
    pub fn kind(&self) -> Stored<FindingKind> {
        match self {
            Self::CacheRebuild(_) => FindingKind::CacheRebuild.into(),
            Self::ContextGrowth(_) => FindingKind::ContextGrowth.into(),
            Self::LargeToolOutput(_) => FindingKind::LargeToolOutput.into(),
            Self::RepeatedFailure(_) => FindingKind::RepeatedFailure.into(),
            Self::SecretExposure(_) => FindingKind::SecretExposure.into(),
            Self::Unknown(raw) => raw.kind.clone(),
        }
    }
}

#[derive(Serialize)]
struct Tagged<'a, T> {
    kind: &'a Stored<FindingKind>,
    #[serde(flatten)]
    detail: &'a T,
}

impl Serialize for Finding {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let kind = &self.kind();
        match self {
            Self::CacheRebuild(detail) => Tagged { kind, detail }.serialize(serializer),
            Self::ContextGrowth(detail) => Tagged { kind, detail }.serialize(serializer),
            Self::LargeToolOutput(detail) => Tagged { kind, detail }.serialize(serializer),
            Self::RepeatedFailure(detail) => Tagged { kind, detail }.serialize(serializer),
            Self::SecretExposure(detail) => Tagged { kind, detail }.serialize(serializer),
            Self::Unknown(raw) => Tagged {
                kind,
                detail: &raw.detail,
            }
            .serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Finding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut detail = serde_json::Map::deserialize(deserializer)?;
        let kind = match detail.remove("kind") {
            Some(serde_json::Value::String(label)) => Stored::<FindingKind>::from_label(&label),
            Some(_) | None => return Err(D::Error::missing_field("kind")),
        };
        let known = match kind {
            Stored::Known(known) => known,
            Stored::Unknown(_) => return Ok(Self::Unknown(RawFinding { kind, detail })),
        };
        let fields = serde_json::Value::Object(detail);
        let finding = match known {
            FindingKind::CacheRebuild => serde_json::from_value(fields).map(Self::CacheRebuild),
            FindingKind::ContextGrowth => serde_json::from_value(fields).map(Self::ContextGrowth),
            FindingKind::LargeToolOutput => {
                serde_json::from_value(fields).map(Self::LargeToolOutput)
            }
            FindingKind::RepeatedFailure => {
                serde_json::from_value(fields).map(Self::RepeatedFailure)
            }
            FindingKind::SecretExposure => serde_json::from_value(fields).map(Self::SecretExposure),
        };
        finding.map_err(D::Error::custom)
    }
}

/// One turn's row in a session report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TurnMetrics {
    pub turn: TurnId,
    pub started_at: UnixMillis,
    pub ended_at: Option<UnixMillis>,
    pub call_count: u64,
    /// Time between the session's previous call ending and this turn.
    pub idle_before_ms: Option<u64>,
    pub context_start_tokens: Option<u64>,
    pub context_end_tokens: Option<u64>,
    /// Cache reads over context, in percent, over calls reporting both.
    pub cache_hit_percent: Option<u64>,
    pub cache_written_tokens: u64,
    pub output_tokens: u64,
}

/// Totals over a session's calls that reported usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
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

/// A session's context and cost measurements, from its call records alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordingReport {
    pub session: RecordingId,
    pub totals: ReportTotals,
    pub turns: Vec<TurnMetrics>,
    pub findings: Vec<Finding>,
    /// Calls beyond the read limit, left out.
    pub omitted_calls: u64,
}

/// A turn's row while its calls are added.
struct TurnRow {
    metrics: TurnMetrics,
    read: u64,
    context: u64,
    end: TurnEnd,
}

/// Where a turn's calls have got to: a turn has ended only once every call
/// it made has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnEnd {
    /// No call added yet.
    Empty,
    /// Every call so far has ended; the latest end.
    Ended(UnixMillis),
    /// A call has no end yet, so neither has the turn.
    Open,
}

impl TurnEnd {
    fn add(self, call_end: Option<UnixMillis>) -> Self {
        match (self, call_end) {
            (Self::Open, _) | (_, None) => Self::Open,
            (Self::Empty, Some(end)) => Self::Ended(end),
            (Self::Ended(earlier), Some(end)) => Self::Ended(earlier.max(end)),
        }
    }

    fn at(self) -> Option<UnixMillis> {
        match self {
            Self::Ended(end) => Some(end),
            Self::Empty | Self::Open => None,
        }
    }
}

impl TurnRow {
    fn add(&mut self, call: &CallView, metrics: &CallMetrics) {
        let row = &mut self.metrics;
        row.call_count = row.call_count.saturating_add(1);
        self.end = self.end.add(call.ended_at);
        row.ended_at = self.end.at();
        if let Some(context) = metrics.context_tokens {
            row.context_start_tokens.get_or_insert(context);
            row.context_end_tokens = Some(context);
            if let Some(read) = metrics.cache_read_tokens {
                self.read = self.read.saturating_add(read);
                self.context = self.context.saturating_add(context);
                row.cache_hit_percent = Self::percent(self.read, self.context);
            }
        }
        row.cache_written_tokens = row
            .cache_written_tokens
            .saturating_add(metrics.cache_written_tokens.unwrap_or(0));
        row.output_tokens = row
            .output_tokens
            .saturating_add(metrics.output_tokens.unwrap_or(0));
    }

    fn percent(part: u64, whole: u64) -> Option<u64> {
        part.saturating_mul(100).checked_div(whole)
    }
}

impl RecordingReport {
    /// Add the secrets the session's calls carried, from their digests'
    /// exposures, as findings in call order.
    pub fn add_exposures(&mut self, exposures: Vec<ExposureView>) {
        let mut timed: Vec<(UnixMillis, Finding)> = exposures
            .into_iter()
            .map(|exposure| {
                (
                    exposure.started_at,
                    Finding::SecretExposure(SecretExposure {
                        call: exposure.call,
                        turn: exposure.turn,
                        field: exposure.field,
                        secret: exposure.kind,
                        hint: exposure.hint,
                        fingerprint: exposure.fingerprint,
                        count: exposure.count,
                    }),
                )
            })
            .collect();
        timed.sort_by_key(|(at, _)| *at);
        self.findings
            .extend(timed.into_iter().map(|(_, finding)| finding));
    }

    /// The report over `calls`, a session's calls in start order.
    pub fn build(session: RecordingId, calls: &[CallView], omitted_calls: u64) -> Self {
        let mut totals = ReportTotals::default();
        let mut rows: Vec<TurnRow> = Vec::new();
        let mut timed: Vec<(UnixMillis, Finding)> = Vec::new();
        let mut growth: Vec<(u64, UnixMillis, ContextGrowth)> = Vec::new();
        let mut peak: Option<u64> = None;
        let mut last_end: Option<UnixMillis> = None;
        for call in calls {
            let metrics = CallMetrics::of(call);
            let idle_before_ms =
                last_end.map(|end| call.started_at.as_millis().saturating_sub(end.as_millis()));
            totals.calls = totals.calls.saturating_add(1);
            if let Some(context) = metrics.context_tokens {
                let read = metrics.cache_read_tokens.unwrap_or(0);
                let written = metrics.cache_written_tokens.unwrap_or(0);
                totals.calls_with_usage = totals.calls_with_usage.saturating_add(1);
                totals.context_tokens = totals.context_tokens.saturating_add(context);
                totals.cache_read_tokens = totals.cache_read_tokens.saturating_add(read);
                totals.cache_written_tokens = totals.cache_written_tokens.saturating_add(written);
                totals.output_tokens = totals
                    .output_tokens
                    .saturating_add(metrics.output_tokens.unwrap_or(0));
                totals.peak_context_tokens = totals.peak_context_tokens.max(context);
                if let Some(earlier) = peak.filter(|peak| *peak >= FindingKind::MIN_PEAK_TOKENS) {
                    if written >= FindingKind::REBUILD_TOKENS
                        && read.saturating_mul(2) < earlier
                        && written.saturating_mul(2) >= earlier
                    {
                        timed.push((
                            call.started_at,
                            Finding::CacheRebuild(CacheRebuild {
                                call: call.id,
                                turn: call.turn,
                                cache_read_tokens: read,
                                cache_written_tokens: written,
                                earlier_peak_tokens: earlier,
                                idle_before_ms,
                            }),
                        ));
                    }
                    let step = context.saturating_sub(earlier);
                    if step >= FindingKind::GROWTH_TOKENS {
                        growth.push((
                            step,
                            call.started_at,
                            ContextGrowth {
                                call: call.id,
                                turn: call.turn,
                                earlier_peak_tokens: earlier,
                                context_tokens: context,
                            },
                        ));
                    }
                }
                peak = Some(peak.map_or(context, |earlier| earlier.max(context)));
            }
            if let Some(turn) = call.turn {
                let index = match rows.iter().position(|row| row.metrics.turn == turn) {
                    Some(index) => index,
                    None => {
                        rows.push(TurnRow {
                            metrics: TurnMetrics {
                                turn,
                                started_at: call.started_at,
                                ended_at: None,
                                call_count: 0,
                                idle_before_ms,
                                context_start_tokens: None,
                                context_end_tokens: None,
                                cache_hit_percent: None,
                                cache_written_tokens: 0,
                                output_tokens: 0,
                            },
                            read: 0,
                            context: 0,
                            end: TurnEnd::Empty,
                        });
                        rows.len().saturating_sub(1)
                    }
                };
                if let Some(row) = rows.get_mut(index) {
                    row.add(call, &metrics);
                }
            }
            last_end = last_end.max(call.ended_at);
        }
        totals.cache_hit_percent =
            TurnRow::percent(totals.cache_read_tokens, totals.context_tokens);
        growth.sort_by_key(|(step, _, _)| std::cmp::Reverse(*step));
        timed.extend(
            growth
                .into_iter()
                .take(FindingKind::MAX_GROWTH)
                .map(|(_, at, finding)| (at, Finding::ContextGrowth(finding))),
        );
        timed.sort_by_key(|(at, _)| *at);
        Self {
            session,
            totals,
            turns: rows.into_iter().map(|row| row.metrics).collect(),
            findings: timed.into_iter().map(|(_, finding)| finding).collect(),
            omitted_calls,
        }
    }
}
