//! Reads assembled from stored digests and bodies: turns, a recording's
//! report, a run's conversation, history targets, and captured prefixes.

use super::{PrefixLimit, Reader};
use crate::activity::{
    ActivityBudget, CallActivity, Excerpt, ReadCall, TurnActivity, TurnHead, TurnSummary,
    report::RecordingReport,
};
use crate::millis::SqlMillis;
use crate::search::ExposureQuery;
use crate::{
    StoreError,
    query::{CallQuery, PageLimit, TimeRange},
    view::{
        BodyPrefix, CallView, Columns, ConversationView, HistoryTarget, RunEntryView, TokenTotals,
    },
};
use ren_protocol::encoding::{ResponseEncoding, ResponseFormat};
use ren_protocol::{direction::Direction, label::Labeled};
use ren_recorder::batch::{BodyKey, HistorySubject};
use ren_recorder::digest::CallDigest;
use ren_recorder::{
    capture::{BodyEnding, CallState},
    digest::{Captured, RederivePass, StoredBody, StoredExchange, StoredResponse},
    id::{CallId, RecordingId, RunId, TurnId},
};
use sqlx::{AssertSqlSafe, QueryBuilder, Row, Sqlite};
use std::collections::HashMap;

/// Agent activity, read from stored bodies when asked (`crate::activity`).
impl Reader {
    /// The most of one body read to derive a digest again: enough for a
    /// long conversation, which clients resend on every call.
    const ACTIVITY_READ: PrefixLimit = PrefixLimit {
        bytes: 16 * 1024 * 1024,
        segments: 4096,
    };
    /// The most calls one turn's activity reads.
    const MAX_TURN_CALLS: i64 = 100;
    /// The most calls a recording report reads, oldest first.
    const MAX_REPORT_CALLS: i64 = 5_000;

    /// One turn: its prompt, calls, tool calls, and effects, from its
    /// calls' digests.
    pub async fn turn_activity(
        &self,
        turn: TurnId,
        budget: ActivityBudget,
    ) -> Result<TurnActivity, StoreError> {
        let row = sqlx::query("SELECT id, recording, native_id FROM turns WHERE id = ?")
            .bind(turn.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StoreError::NotFound)?;
        let columns = Columns(&row);
        let head = TurnHead {
            turn: columns.id("id")?,
            recording: columns.id("recording")?,
            native_id: columns.text("native_id")?.unwrap_or_default(),
        };
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM calls WHERE turn = ?")
            .bind(turn.to_string())
            .fetch_one(&self.pool)
            .await?;
        let views = self.turn_calls(turn, Self::MAX_TURN_CALLS).await?;
        let read = i64::try_from(views.len()).unwrap_or(i64::MAX);
        let calls = self.digested(views).await?;
        let omitted = u64::try_from(total.saturating_sub(read)).unwrap_or(0);
        Ok(TurnActivity::assemble(head, calls, omitted, budget))
    }

    /// A recording's turns, newest first, each with the start of its prompt:
    /// the first one among its calls' digests.
    pub async fn recording_turns(
        &self,
        recording: RecordingId,
        limit: PageLimit,
    ) -> Result<Vec<TurnSummary>, StoreError> {
        self.require_recording(recording).await?;
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT t.id, t.native_id, MIN(c.started_at) AS started_at, \
             CASE WHEN COUNT(c.ended_at) = COUNT(*) THEN MAX(c.ended_at) END AS ended_at, ",
        );
        CallQuery::push_usage_columns(&mut sql);
        sql.push(" FROM turns t JOIN calls c ON c.turn = t.id WHERE t.recording = ")
            .push_bind(recording.to_string())
            .push(" GROUP BY t.id ORDER BY started_at DESC, t.id DESC LIMIT ")
            .push_bind(i64::from(limit.get()));
        let rows = sql.build().fetch_all(&self.pool).await?;
        let ids = rows
            .iter()
            .map(|row| Columns(row).id::<TurnId>("id"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut prompts = self.turn_prompts(&ids).await?;
        let mut turns = Vec::with_capacity(rows.len());
        for (row, turn) in rows.iter().zip(ids) {
            let columns = Columns(row);
            turns.push(TurnSummary {
                turn,
                native_id: columns.text("native_id")?.unwrap_or_default(),
                started_at: columns.time("started_at")?,
                ended_at: columns.optional_time("ended_at")?,
                call_count: columns.count("calls")?,
                error_count: columns.count("errors")?,
                tokens: TokenTotals::from_row(row)?,
                prompt: prompts
                    .remove(&turn)
                    .map(|prompt| prompt.narrow(TurnSummary::PROMPT_CHARS)),
            });
        }
        Ok(turns)
    }

    /// Each turn's prompt: the first one, by call start, among its calls'
    /// digests.
    async fn turn_prompts(&self, turns: &[TurnId]) -> Result<HashMap<TurnId, Excerpt>, StoreError> {
        let mut prompts = HashMap::new();
        if turns.is_empty() {
            return Ok(prompts);
        }
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT c.turn, d.prompt, d.prompt_omitted FROM calls c JOIN call_digests d ON d.call = c.id WHERE d.prompt IS NOT NULL AND c.turn IN (",
        );
        let mut list = sql.separated(", ");
        for turn in turns {
            list.push_bind(turn.to_string());
        }
        sql.push(") ORDER BY c.started_at, c.id");
        for row in sql.build().fetch_all(&self.pool).await? {
            let columns = Columns(&row);
            let turn: TurnId = columns.id("turn")?;
            if prompts.contains_key(&turn) {
                continue;
            }
            prompts.insert(
                turn,
                Excerpt {
                    text: columns.text("prompt")?.unwrap_or_default(),
                    omitted_chars: columns.count("prompt_omitted")?,
                },
            );
        }
        Ok(prompts)
    }

    /// A recording's context and cost measurements, from call records alone,
    /// with the secrets its calls carried.
    pub async fn recording_report(
        &self,
        recording: RecordingId,
    ) -> Result<RecordingReport, StoreError> {
        self.require_recording(recording).await?;
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM calls WHERE recording = ?")
            .bind(recording.to_string())
            .fetch_one(&self.pool)
            .await?;
        let calls: Vec<CallView> = sqlx::query(AssertSqlSafe(format!(
            "{} WHERE c.recording = ? ORDER BY c.started_at, c.id LIMIT ?",
            CallView::SELECT
        )))
        .bind(recording.to_string())
        .bind(Self::MAX_REPORT_CALLS)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(CallView::from_row)
        .collect::<Result<_, _>>()?;
        let read = i64::try_from(calls.len()).unwrap_or(i64::MAX);
        let omitted = u64::try_from(total.saturating_sub(read)).unwrap_or(0);
        let mut report = RecordingReport::build(recording, &calls, omitted);
        report.add_exposures(
            self.exposures(&ExposureQuery {
                recording: Some(recording),
                project: None,
                time: TimeRange::default(),
                limit: PageLimit::LARGEST,
            })
            .await?,
        );
        Ok(report)
    }

    /// One call's activity, with results matched only within the call.
    pub async fn call_activity(
        &self,
        call: CallId,
        budget: ActivityBudget,
    ) -> Result<CallActivity, StoreError> {
        let view = self.call_view(call).await?;
        let mut calls = self.digested(vec![view]).await?;
        calls
            .pop()
            .map(|call| call.activity(budget))
            .ok_or(StoreError::NotFound)
    }

    /// Calls with their digests, in the order given.
    async fn digested(&self, views: Vec<CallView>) -> Result<Vec<ReadCall>, StoreError> {
        let mut digests: HashMap<CallId, CallDigest> = HashMap::new();
        if !views.is_empty() {
            let mut sql = QueryBuilder::<Sqlite>::new(
                "SELECT call, digest FROM call_digests WHERE call IN (",
            );
            let mut list = sql.separated(", ");
            for view in &views {
                list.push_bind(view.id.to_string());
            }
            sql.push(")");
            for row in sql.build().fetch_all(&self.pool).await? {
                let columns = Columns(&row);
                // A digest this build cannot read is derived again; until
                // then the call shows as not yet derived.
                if let Ok(digest) =
                    serde_json::from_str::<CallDigest>(&columns.text("digest")?.unwrap_or_default())
                {
                    digests.insert(columns.id("call")?, digest);
                }
            }
        }
        Ok(views
            .into_iter()
            .map(|view| ReadCall {
                digest: digests.remove(&view.id),
                view,
            })
            .collect())
    }

    /// `NotFound` unless the store has `recording`.
    /// The newest `limit` entries of `run`'s conversation, oldest first:
    /// its own, and those of the runs before it on the same client thread
    /// (a conversation resumed in a new run reads as one). `None` when the
    /// run was never recorded.
    pub async fn conversation(
        &self,
        run: RunId,
        limit: u32,
    ) -> Result<Option<ConversationView>, StoreError> {
        let id = run.to_string();
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs WHERE id = ?)")
            .bind(&id)
            .fetch_one(&self.pool)
            .await?;
        if !exists {
            return Ok(None);
        }
        // The runs that make the conversation, in the order they started:
        // those before it on its thread, and its recording's earlier
        // processes, which a new one resumes before it says which thread it
        // opened.
        let chain = "SELECT r.id, r.started_at FROM runs r, runs this WHERE this.id = ?1 AND (r.id = this.id OR (r.started_at <= this.started_at AND ((this.thread IS NOT NULL AND r.thread = this.thread) OR (this.operator_session IS NOT NULL AND r.operator_session = this.operator_session))))";
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT * FROM (SELECT e.run, e.seq, e.at, e.started_at AS entry_started_at, e.turn, e.event, chain.started_at FROM run_entries e JOIN ({chain}) chain ON chain.id = e.run ORDER BY chain.started_at DESC, e.run DESC, e.seq DESC LIMIT ?2) ORDER BY started_at, run, seq"
        )))
        .bind(&id)
        .bind(i64::from(limit) + 1)
        .fetch_all(&self.pool)
        .await?;
        let earlier = rows.len() > usize::try_from(limit).unwrap_or(usize::MAX);
        let entries = rows
            .iter()
            .skip(usize::from(earlier))
            .map(|row| {
                let columns = Columns(row);
                let event: String = row.try_get("event")?;
                Ok(RunEntryView {
                    run: columns.id("run")?,
                    seq: columns.count("seq")?,
                    at: columns.time("at")?,
                    started_at: columns.time("entry_started_at")?,
                    turn: columns.text("turn")?,
                    event: serde_json::from_str(&event)
                        .map_err(|_| StoreError::Corrupt("run entry"))?,
                })
            })
            .collect::<Result<_, StoreError>>()?;
        Ok(Some(ConversationView { entries, earlier }))
    }

    /// What an edit of `subject` would touch, or `None` when it was never
    /// recorded: the open calls of its recordings and its runs still running.
    /// A recording's runs are those whose thread it is; a run's recordings those
    /// its thread names.
    pub async fn history_target(
        &self,
        subject: HistorySubject,
    ) -> Result<Option<HistoryTarget>, StoreError> {
        // The recordings a subject names (by their native ids), and its runs.
        let (id, exists, threads, runs) = match subject {
            HistorySubject::Recording(recording) => (
                recording.to_string(),
                "SELECT EXISTS(SELECT 1 FROM recordings WHERE id = ?1)",
                "(SELECT native_id FROM recordings WHERE id = ?1)",
                "id = ?1 OR thread = (SELECT native_id FROM recordings WHERE id = ?1)",
            ),
            HistorySubject::Run(run) => (
                run.to_string(),
                "SELECT EXISTS(SELECT 1 FROM runs WHERE id = ?1)",
                "(SELECT thread FROM runs WHERE id = ?1)",
                "id = ?1 OR thread = (SELECT thread FROM runs WHERE id = ?1)",
            ),
            HistorySubject::OperatorSession(session) => (
                session.to_string(),
                "SELECT EXISTS(SELECT 1 FROM operator_sessions WHERE id = ?1)",
                "(SELECT thread FROM runs WHERE operator_session = ?1)",
                "operator_session = ?1",
            ),
        };
        let exists: bool = sqlx::query_scalar(AssertSqlSafe(exists))
            .bind(&id)
            .fetch_one(&self.pool)
            .await?;
        if !exists {
            return Ok(None);
        }
        let counts = sqlx::query(AssertSqlSafe(format!(
            "SELECT (SELECT COUNT(*) FROM calls WHERE state = ?2 AND recording IN (SELECT id FROM recordings WHERE native_id IN {threads})) AS open_calls, (SELECT COUNT(*) FROM runs WHERE ended_at IS NULL AND ({runs})) AS running_runs"
        )))
        .bind(&id)
        .bind(CallState::Open.label())
        .fetch_one(&self.pool)
        .await?;
        let columns = Columns(&counts);
        Ok(Some(HistoryTarget {
            open_calls: columns.count("open_calls")?,
            running_runs: columns.count("running_runs")?,
        }))
    }

    pub(super) async fn require_recording(&self, recording: RecordingId) -> Result<(), StoreError> {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recordings WHERE id = ?)")
                .bind(recording.to_string())
                .fetch_one(&self.pool)
                .await?;
        if exists {
            Ok(())
        } else {
            Err(StoreError::NotFound)
        }
    }

    async fn call_view(&self, call: CallId) -> Result<CallView, StoreError> {
        let row = sqlx::query(AssertSqlSafe(format!(
            "{} WHERE c.id = ?",
            CallView::SELECT
        )))
        .bind(call.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound)?;
        CallView::from_row(&row)
    }

    async fn turn_calls(&self, turn: TurnId, limit: i64) -> Result<Vec<CallView>, StoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "{} WHERE c.turn = ? ORDER BY c.started_at, c.id LIMIT ?",
            CallView::SELECT
        )))
        .bind(turn.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(CallView::from_row)
        .collect()
    }

    /// Up to `limit` finished calls whose digest is missing or older than
    /// `version`, newest first, with their stored bodies.
    pub(crate) async fn undigested(
        &self,
        version: u32,
        pass: RederivePass,
    ) -> Result<Vec<StoredExchange>, StoreError> {
        let calls: Vec<String> = sqlx::query_scalar(
            "SELECT c.id FROM calls c LEFT JOIN call_digests d ON d.call = c.id WHERE c.state <> 'open' AND (d.call IS NULL OR d.version < ?) ORDER BY c.started_at DESC, c.id DESC LIMIT ?",
        )
        .bind(i64::from(version))
        .bind(i64::try_from(pass.calls).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await?;
        let mut exchanges = Vec::with_capacity(calls.len());
        for call in calls {
            let call: CallId = call
                .parse()
                .map_err(|_| StoreError::Corrupt("identifier"))?;
            let view = match self.call_view(call).await {
                Ok(view) => view,
                // Pruned since it was listed.
                Err(StoreError::NotFound) => continue,
                Err(error) => return Err(error),
            };
            let request = self
                .captured(BodyKey::Request(call))
                .await?
                .map(CapturedPrefix::into_stored);
            let response = match self.body_key(call, Direction::Response).await {
                Ok(key) => {
                    let format: Option<String> = sqlx::query_scalar(
                        "SELECT a.response_format FROM attempts a JOIN calls c ON c.final_attempt = a.id WHERE c.id = ?",
                    )
                    .bind(call.to_string())
                    .fetch_optional(&self.pool)
                    .await?
                    .flatten();
                    self.captured(key).await?.map(|prefix| StoredResponse {
                        format: format
                            .and_then(|label| ResponseFormat::from_label(&label))
                            .unwrap_or(ResponseFormat::Other),
                        encoding: prefix.encoding.unwrap_or(ResponseEncoding::Identity),
                        body: prefix.into_stored(),
                    })
                }
                Err(StoreError::NotFound) => None,
                Err(error) => return Err(error),
            };
            exchanges.push(StoredExchange {
                call,
                endpoint: view.endpoint.known(),
                harness: view
                    .attribution
                    .hint
                    .as_ref()
                    .and_then(|hint| hint.source.known())
                    .map(|source| source.harness()),
                request,
                response,
            });
            if pass.filled(&exchanges) {
                break;
            }
        }
        Ok(exchanges)
    }

    /// A body's stored prefix with its message boundaries; `None` when the
    /// body was never recorded.
    async fn captured(&self, key: BodyKey) -> Result<Option<CapturedPrefix>, StoreError> {
        let prefix = match self.body_prefix(key, Self::ACTIVITY_READ).await {
            Ok(prefix) => prefix,
            Err(StoreError::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        let messages: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT byte_offset, len FROM messages WHERE owner = ? AND direction = ? ORDER BY byte_offset LIMIT ?",
        )
        .bind(key.owner())
        .bind(key.direction().label())
        .bind(Self::MAX_MESSAGES)
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(CapturedPrefix::new(prefix, &messages)))
    }
}

impl TimeRange {
    /// `AND` clauses for calls (aliased `c`) started within the range, both
    /// ends inclusive.
    pub(crate) fn push_started(self, sql: &mut QueryBuilder<Sqlite>) {
        if let Some(from) = self.from() {
            sql.push(" AND c.started_at >= ")
                .push_bind(SqlMillis::at(from));
        }
        if let Some(until) = self.until() {
            sql.push(" AND c.started_at <= ")
                .push_bind(SqlMillis::at(until));
        }
    }
}

/// A stored body prefix, ready to replay.
struct CapturedPrefix {
    bytes: Vec<u8>,
    encoding: Option<ResponseEncoding>,
    messages: Option<Vec<std::ops::Range<usize>>>,
    /// Observed bytes are missing from `bytes`: a gap, the read limit, or
    /// bytes never stored.
    missing: bool,
    ending: BodyEnding,
}

impl CapturedPrefix {
    fn new(prefix: BodyPrefix, messages: &[(i64, i64)]) -> Self {
        let len = prefix.bytes.len();
        // Messages past the prefix are left out; the extent already says so.
        let messages = (!messages.is_empty()).then(|| {
            messages
                .iter()
                .filter_map(|&(offset, size)| {
                    let start = usize::try_from(offset).ok()?;
                    let end = start.checked_add(usize::try_from(size).ok()?)?;
                    (end <= len).then_some(start..end)
                })
                .collect()
        });
        Self {
            encoding: prefix
                .encoding
                .map(|encoding| encoding.known().unwrap_or(ResponseEncoding::Unsupported)),
            missing: prefix.gap
                || prefix.truncated
                || prefix
                    .body
                    .observed_bytes
                    .is_some_and(|observed| prefix.body.stored_bytes < observed),
            ending: prefix.body.ending.known().unwrap_or(BodyEnding::Unfinished),
            bytes: prefix.bytes,
            messages,
        }
    }

    /// The prefix as the recorder replays it.
    fn into_stored(self) -> StoredBody {
        StoredBody {
            complete: if self.missing {
                Captured::Partial
            } else {
                Captured::Whole
            },
            ending: self.ending,
            messages: self.messages,
            bytes: self.bytes,
        }
    }
}
