//! Read-only access through a pool of query-only connections. Each query uses
//! its own short transaction; readers never block the writer.

use super::chunk::{ChunkHash, Codec, EncodedChunk};
use crate::activity::{
    ActivityBudget, CallActivity, Excerpt, ReadCall, TurnActivity, TurnHead, TurnSummary,
    report::SessionReport,
};
use crate::search::{CallDocuments, IndexCursor, IndexedPrompt, SearchIndex};
use crate::{
    Store, StoreError,
    query::{CallQuery, Cursor, KindFilter, PageLimit, StatusFilter, TimeRange, ToolFilter},
    view::{
        AttemptView, BodyPrefix, BodyView, CallDetail, CallPage, CallView, ChannelView, Columns,
        DiskUsage, MessageView, ModelUsage, OperatorActionView, SessionView, StorageView,
        TokenTotals, UsageBucket, UsageCounts, UsageView,
    },
    writer::BlobAccess,
};
use sqlx::{
    AssertSqlSafe, QueryBuilder, Row, Sqlite, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::collections::{HashMap, HashSet};
use std::{path::Path, sync::Arc, time::Duration};
use tgv_harness::attribution::HintSource;
use tgv_protocol::activity::{
    ActivityLimitation, ActivityLimits, ActivityReader, BodyExtent, CapturedBody, RequestActivity,
    ResponseActivity,
};
use tgv_protocol::encoding::ResponseEncoding;
use tgv_protocol::inspect::pipeline::Protocol;
use tgv_protocol::inspect::{InputAccounting, ModelOutcome};
use tgv_protocol::{
    direction::Direction,
    label::{Labeled, Stored},
    route::{Endpoint, Method},
};
use tgv_recorder::Observation;
use tgv_recorder::batch::BodyKey;
use tgv_recorder::secrets::SecretScanner;
use tgv_recorder::time::UnixMillis;
use tgv_recorder::{
    capture::{AttemptTerminal, CallState, TerminalClass},
    id::{CallId, SessionId, TurnId},
};

/// Bounds on a body prefix read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefixLimit {
    pub bytes: usize,
    pub segments: u32,
}

#[derive(Clone)]
pub struct Reader {
    pool: SqlitePool,
    /// Finds credentials in turns' bodies, for their findings.
    scanner: Arc<SecretScanner>,
    /// The data directory, for measuring the search index beside it.
    dir: Arc<Path>,
}

impl Reader {
    /// Message boundaries returned with one call's detail.
    const MAX_MESSAGES: i64 = 10_000;

    /// Open a read pool on a data directory a [`Store`] has initialised.
    pub async fn open(dir: &Path, connections: u32) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(dir.join(Store::METADATA_FILE))
            .read_only(true)
            .pragma("query_only", "ON")
            .busy_timeout(Duration::from_secs(5));
        let dir: Arc<Path> = dir.into();
        let attach = dir.clone();
        let pool = SqlitePoolOptions::new()
            .max_connections(connections.max(1))
            .acquire_timeout(Duration::from_secs(5))
            .after_connect(move |connection, _| {
                let dir = attach.clone();
                Box::pin(
                    async move { Store::attach_blobs(connection, &dir, BlobAccess::Read).await },
                )
            })
            .connect_with(options)
            .await?;
        Ok(Self {
            pool,
            dir,
            scanner: Arc::new(SecretScanner::new()?),
        })
    }

    /// Newest calls first, filtered.
    pub async fn calls(&self, query: &CallQuery) -> Result<CallPage, StoreError> {
        let mut sql = QueryBuilder::<Sqlite>::new(CallView::SELECT);
        sql.push(" WHERE 1 = 1");
        if let Some(cursor) = query.after {
            sql.push(" AND (c.started_at, c.id) < (")
                .push_bind(i64::try_from(cursor.started_at.as_millis()).unwrap_or(i64::MAX))
                .push(", ")
                .push_bind(cursor.call.to_string())
                .push(")");
        }
        query.push_filters(&mut sql);
        sql.push(" ORDER BY c.started_at DESC, c.id DESC LIMIT ")
            .push_bind(i64::from(query.limit.get()).saturating_add(1));
        let rows = sql.build().fetch_all(&self.pool).await?;
        CallPage::from_rows(&rows, query.limit)
    }

    /// A session's calls in chronological order.
    pub async fn session_calls(
        &self,
        session: SessionId,
        limit: PageLimit,
        after: Option<Cursor>,
    ) -> Result<CallPage, StoreError> {
        self.require_session(session).await?;
        let (time, call) = after.map_or((-1, String::new()), |cursor| {
            (
                i64::try_from(cursor.started_at.as_millis()).unwrap_or(i64::MAX),
                cursor.call.to_string(),
            )
        });
        let rows = sqlx::query(AssertSqlSafe(format!(
            "{} WHERE c.session = ? AND (c.started_at, c.id) > (?, ?) ORDER BY c.started_at, c.id LIMIT ?",
            CallView::SELECT
        )))
        .bind(session.to_string())
        .bind(time)
        .bind(call)
        .bind(i64::from(limit.get()).saturating_add(1))
        .fetch_all(&self.pool)
        .await?;
        CallPage::from_rows(&rows, limit)
    }

    pub async fn call(&self, id: CallId) -> Result<CallDetail, StoreError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(AssertSqlSafe(format!(
            "{} WHERE c.id = ?",
            CallView::SELECT
        )))
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
        let call = CallView::from_row(&row)?;
        let channel = sqlx::query(AssertSqlSafe(format!(
            "{} WHERE id = ?",
            ChannelView::SELECT
        )))
        .bind(call.channel.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(StoreError::from)
        .and_then(|row| ChannelView::from_row(&row))?;
        let attempts: Vec<AttemptView> = sqlx::query(AssertSqlSafe(format!(
            "{} WHERE a.call = ? ORDER BY a.ordinal",
            AttemptView::SELECT
        )))
        .bind(id.to_string())
        .fetch_all(&mut *tx)
        .await?
        .iter()
        .map(AttemptView::from_row)
        .collect::<Result<_, _>>()?;
        let observations = sqlx::query(AssertSqlSafe(format!(
            "{} WHERE call = ? ORDER BY seq LIMIT 1000",
            Columns::OBSERVATION_SELECT
        )))
        .bind(id.to_string())
        .fetch_all(&mut *tx)
        .await?
        .iter()
        .map(|row| Columns(row).observation())
        .collect::<Result<_, _>>()?;
        let messages = sqlx::query(MessageView::SELECT_FOR_CALL)
            .bind(id.to_string())
            .bind(Direction::Request.label())
            .bind(Direction::Response.label())
            .bind(
                attempts
                    .last()
                    .map(|attempt: &AttemptView| attempt.id.to_string()),
            )
            .bind(Self::MAX_MESSAGES)
            .fetch_all(&mut *tx)
            .await?
            .iter()
            .map(MessageView::from_row)
            .collect::<Result<_, _>>()?;
        tx.commit().await?;
        Ok(CallDetail {
            call,
            transport: channel.transport.clone(),
            channel,
            attempts,
            observations,
            messages,
        })
    }

    pub async fn sessions(&self, limit: PageLimit) -> Result<Vec<SessionView>, StoreError> {
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT s.id, s.provider, s.source, s.native_id, s.first_seen, s.last_seen, (SELECT COUNT(*) FROM turns WHERE session = s.id) AS turn_count, ",
        );
        CallQuery::push_usage_columns(&mut sql);
        sql.push(" FROM sessions s LEFT JOIN calls c ON c.session = s.id GROUP BY s.id ORDER BY s.last_seen DESC, s.id DESC LIMIT ")
            .push_bind(i64::from(limit.get()));
        sql.build()
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(SessionView::from_row)
            .collect()
    }

    /// Calls, errors, and tokens of the calls started in `range`: in total,
    /// by the hour, and by model.
    pub async fn usage(&self, range: TimeRange) -> Result<UsageView, StoreError> {
        let select = |prefix: &str| {
            let mut sql = QueryBuilder::<Sqlite>::new(prefix);
            CallQuery::push_usage_columns(&mut sql);
            sql.push(" FROM calls c WHERE 1 = 1");
            range.push_started(&mut sql);
            sql
        };
        let mut totals = select("SELECT ");
        let totals = totals.build().fetch_one(&self.pool).await?;
        let mut hours = select("SELECT (c.started_at / 3600000) * 3600000 AS start, ");
        hours.push(" GROUP BY start ORDER BY start");
        let hours = hours
            .build()
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|row| {
                Ok(UsageBucket {
                    start: Columns(row).time("start")?,
                    usage: UsageCounts::from_row(row)?,
                })
            })
            .collect::<Result<_, StoreError>>()?;
        let mut models = select("SELECT c.provider, c.model, ");
        models.push(
            " GROUP BY c.provider, c.model ORDER BY input_tokens DESC NULLS LAST, calls DESC",
        );
        let models = models
            .build()
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|row| {
                let columns = Columns(row);
                Ok(ModelUsage {
                    provider: columns.label("provider")?,
                    model: columns.text("model")?,
                    usage: UsageCounts::from_row(row)?,
                })
            })
            .collect::<Result<_, StoreError>>()?;
        Ok(UsageView {
            totals: UsageCounts::from_row(&totals)?,
            hours,
            models,
        })
    }

    /// The newest `limit` recorded operator commands, oldest first.
    pub async fn operator_actions(
        &self,
        limit: PageLimit,
    ) -> Result<Vec<OperatorActionView>, StoreError> {
        sqlx::query("SELECT seq, at_wall, revision, actor, command FROM (SELECT * FROM operator_actions ORDER BY seq DESC LIMIT ?) ORDER BY seq")
            .bind(i64::from(limit.get()))
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(OperatorActionView::from_row)
            .collect()
    }

    /// The body of `call` in `direction`: the request, or the final attempt's response.
    pub async fn body_key(
        &self,
        call: CallId,
        direction: Direction,
    ) -> Result<BodyKey, StoreError> {
        match direction {
            Direction::Request => {
                let exists: bool =
                    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM calls WHERE id = ?)")
                        .bind(call.to_string())
                        .fetch_one(&self.pool)
                        .await?;
                exists
                    .then_some(BodyKey::Request(call))
                    .ok_or(StoreError::NotFound)
            }
            Direction::Response => {
                let attempt: Option<String> =
                    sqlx::query_scalar("SELECT final_attempt FROM calls WHERE id = ?")
                        .bind(call.to_string())
                        .fetch_optional(&self.pool)
                        .await?
                        .flatten();
                let attempt = attempt.ok_or(StoreError::NotFound)?;
                Ok(BodyKey::Response(
                    attempt
                        .parse()
                        .map_err(|_| StoreError::Corrupt("identifier"))?,
                ))
            }
        }
    }

    /// A bounded, contiguous prefix of a body. Reading stops at a gap: bytes
    /// are never joined across missing capture.
    pub async fn body_prefix(
        &self,
        body: BodyKey,
        limit: PrefixLimit,
    ) -> Result<BodyPrefix, StoreError> {
        let mut tx = self.pool.begin().await?;
        let view = Self::body_view(&mut tx, body).await?;
        let rows = sqlx::query("SELECT s.byte_offset, s.len, c.hash, c.codec, c.raw_len, c.data FROM segments s LEFT JOIN blobs.chunks c ON c.hash = s.chunk WHERE s.owner = ? AND s.direction = ? ORDER BY s.byte_offset LIMIT ?")
            .bind(body.owner())
            .bind(body.direction().label())
            .bind(i64::from(limit.segments).saturating_add(1))
            .fetch_all(&mut *tx)
            .await?;
        let encoding = match body {
            BodyKey::Response(attempt) => sqlx::query_scalar::<_, Option<String>>(
                "SELECT response_encoding FROM attempts WHERE id = ?",
            )
            .bind(attempt.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .flatten()
            .map(|label| Stored::from_label(&label)),
            BodyKey::Request(_) | BodyKey::AttemptRequest(_) => None,
        };
        tx.commit().await?;
        let mut prefix = BodyPrefix {
            body: view,
            bytes: Vec::new(),
            gap: false,
            truncated: false,
            encoding,
        };
        let mut next = 0_u64;
        for (index, row) in rows.iter().enumerate() {
            if index >= usize::try_from(limit.segments).unwrap_or(usize::MAX)
                || prefix.bytes.len() >= limit.bytes
            {
                prefix.truncated = true;
                break;
            }
            let offset: u64 = row.try_get("byte_offset")?;
            if offset != next {
                prefix.gap = true;
                break;
            }
            let raw = EncodedChunk::read_segment(row)?;
            next = next.saturating_add(u64::try_from(raw.len()).unwrap_or(u64::MAX));
            let room = limit.bytes.saturating_sub(prefix.bytes.len());
            prefix.truncated = raw.len() > room;
            prefix
                .bytes
                .extend_from_slice(raw.get(..room.min(raw.len())).unwrap_or_default());
        }
        if !prefix.truncated && !prefix.gap && next < prefix.body.stored_bytes {
            prefix.truncated = true;
        }
        if !prefix.truncated
            && prefix
                .body
                .observed_bytes
                .is_some_and(|observed| next < observed)
        {
            prefix.gap = true;
        }
        Ok(prefix)
    }

    /// Stream a complete body, in order, a page of segments per transaction.
    /// Fails with [`StoreError::IncompleteBody`] unless every observed byte is
    /// stored and the end was seen.
    pub async fn export(
        &self,
        body: BodyKey,
        mut write: impl FnMut(&[u8]) -> std::io::Result<()>,
    ) -> Result<u64, StoreError> {
        let view = {
            let mut tx = self.pool.begin().await?;
            let view = Self::body_view(&mut tx, body).await?;
            tx.commit().await?;
            view
        };
        if !view.complete() {
            return Err(StoreError::IncompleteBody);
        }
        let mut next = 0_u64;
        loop {
            let rows = sqlx::query("SELECT s.byte_offset, s.len, c.hash, c.codec, c.raw_len, c.data FROM segments s LEFT JOIN blobs.chunks c ON c.hash = s.chunk WHERE s.owner = ? AND s.direction = ? AND s.byte_offset >= ? ORDER BY s.byte_offset LIMIT 64")
                .bind(body.owner())
                .bind(body.direction().label())
                .bind(i64::try_from(next).unwrap_or(i64::MAX))
                .fetch_all(&self.pool)
                .await?;
            if rows.is_empty() {
                break;
            }
            for row in &rows {
                let offset: u64 = row.try_get("byte_offset")?;
                if offset != next {
                    return Err(StoreError::IncompleteBody);
                }
                let raw = EncodedChunk::read_segment(row)?;
                next = next.saturating_add(u64::try_from(raw.len()).unwrap_or(u64::MAX));
                write(&raw)?;
            }
        }
        if Some(next) == view.observed_bytes {
            Ok(next)
        } else {
            Err(StoreError::IncompleteBody)
        }
    }

    async fn body_view(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        body: BodyKey,
    ) -> Result<BodyView, StoreError> {
        let row = sqlx::query("SELECT observed_bytes AS b_observed, stored_bytes AS b_stored, ending AS b_ending FROM bodies WHERE owner = ? AND direction = ?")
            .bind(body.owner())
            .bind(body.direction().label())
            .fetch_optional(&mut **tx)
            .await?
            .ok_or(StoreError::NotFound)?;
        BodyView::from_columns(&Columns(&row), "b")?.ok_or(StoreError::NotFound)
    }

    /// Committed observations after `seq`, oldest first.
    pub async fn observations_after(
        &self,
        seq: u64,
        limit: PageLimit,
    ) -> Result<Vec<Observation>, StoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "{} WHERE seq > ? ORDER BY seq LIMIT ?",
            Columns::OBSERVATION_SELECT
        )))
        .bind(i64::try_from(seq).unwrap_or(i64::MAX))
        .bind(i64::from(limit.get()))
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(|row| Columns(row).observation())
        .collect()
    }

    /// The oldest retained observation, if any.
    pub async fn oldest_seq(&self) -> Result<Option<u64>, StoreError> {
        Ok(sqlx::query_scalar("SELECT MIN(seq) FROM observations")
            .fetch_one(&self.pool)
            .await?)
    }

    /// The newest committed observation, if any.
    pub async fn latest_seq(&self) -> Result<Option<u64>, StoreError> {
        Ok(sqlx::query_scalar("SELECT MAX(seq) FROM observations")
            .fetch_one(&self.pool)
            .await?)
    }

    pub async fn storage(&self) -> Result<StorageView, StoreError> {
        let mut connection = self.pool.acquire().await?;
        let disk = DiskUsage::measure(&mut connection, &self.dir).await?;
        let calls =
            sqlx::query("SELECT COUNT(*) AS calls, COALESCE(SUM(state = ?), 0) AS open FROM calls")
                .bind(CallState::Open.label())
                .fetch_one(&mut *connection)
                .await?;
        let chunks = sqlx::query("SELECT COUNT(*) AS chunks, COALESCE(SUM(raw_len), 0) AS unique_bytes, COALESCE(SUM(length(data)), 0) AS compressed FROM blobs.chunks")
            .fetch_one(&mut *connection)
            .await?;
        let referenced: u64 = sqlx::query_scalar("SELECT COALESCE(SUM(len), 0) FROM segments")
            .fetch_one(&mut *connection)
            .await?;
        Ok(StorageView {
            disk,
            calls: calls.try_get("calls")?,
            open_calls: calls.try_get("open")?,
            chunks: chunks.try_get("chunks")?,
            referenced_bytes: referenced,
            unique_bytes: chunks.try_get("unique_bytes")?,
            compressed_bytes: chunks.try_get("compressed")?,
        })
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

impl EncodedChunk {
    /// The raw bytes of a segment row joined to its chunk, verified.
    fn read_segment(row: &sqlx::sqlite::SqliteRow) -> Result<Vec<u8>, StoreError> {
        let hash: Option<Vec<u8>> = row.try_get("hash")?;
        let hash = hash
            .as_deref()
            .and_then(ChunkHash::restore)
            .ok_or(StoreError::Corrupt("missing chunk"))?;
        let codec: String = row.try_get("codec")?;
        let codec = Codec::from_label(&codec).ok_or(StoreError::Corrupt("chunk codec"))?;
        let raw_len: u64 = row.try_get("raw_len")?;
        let len: u64 = row.try_get("len")?;
        if raw_len != len {
            return Err(StoreError::Corrupt("segment length"));
        }
        EncodedChunk::decode(
            hash,
            codec,
            usize::try_from(raw_len).map_err(|_| StoreError::Corrupt("chunk length"))?,
            row.try_get("data")?,
        )
        .ok_or(StoreError::Corrupt("chunk content"))
    }
}

impl CallQuery {
    /// `calls`, `errors`, `input_tokens`, `output_tokens`, and
    /// `cache_read_tokens` aggregated over the rows of `calls c`. Errors use
    /// the same predicate as the `errors` status filter. Input adds cached
    /// context where the provider reported it separately. A token SUM over
    /// calls that reported no usage is NULL: "not reported", not zero.
    fn push_usage_columns(sql: &mut QueryBuilder<Sqlite>) {
        sql.push("COUNT(c.id) AS calls, COALESCE(SUM(CASE WHEN c.status >= 400 OR c.terminal IN (");
        Self::push_terminals(sql, TerminalClass::Failed);
        sql.push(") OR c.outcome = ")
            .push_bind(ModelOutcome::Failed.label())
            .push(" THEN 1 ELSE 0 END), 0) AS errors, ")
            .push("SUM(CASE WHEN c.usage IS NULL THEN NULL ELSE COALESCE(json_extract(c.usage, '$.input_tokens'), 0) + CASE WHEN json_extract(c.usage, '$.accounting') = ")
            .push_bind(InputAccounting::ExcludesCache.label())
            .push(" THEN COALESCE(json_extract(c.usage, '$.cache_read_tokens'), 0) + COALESCE(json_extract(c.usage, '$.cache_creation_tokens'), 0) ELSE 0 END END) AS input_tokens, ")
            .push("SUM(json_extract(c.usage, '$.output_tokens')) AS output_tokens, ")
            .push("SUM(json_extract(c.usage, '$.cache_read_tokens')) AS cache_read_tokens");
    }

    /// Append this query's filters as WHERE clauses.
    /// Bind every terminal label of one class, comma-separated.
    fn push_terminals(sql: &mut QueryBuilder<Sqlite>, class: TerminalClass) {
        let mut labels = sql.separated(", ");
        for terminal in AttemptTerminal::ALL
            .iter()
            .filter(|terminal| terminal.class() == class)
        {
            labels.push_bind(terminal.label());
        }
    }

    fn push_filters(&self, sql: &mut QueryBuilder<Sqlite>) {
        let filter = &self.filter;
        filter.time.push_started(sql);
        if let Some(provider) = filter.provider {
            sql.push(" AND c.provider = ").push_bind(provider.label());
        }
        if let Some(model) = &filter.model {
            sql.push(" AND instr(lower(COALESCE(c.model, '')), lower(")
                .push_bind(model.as_str().to_owned())
                .push(")) > 0");
        }
        if let Some(tool) = &filter.tool {
            sql.push(" AND EXISTS (SELECT 1 FROM json_each(c.tool_names) t WHERE instr(lower(t.value), lower(")
                .push_bind(tool.as_str().to_owned())
                .push(")) > 0)");
        }
        if let Some(text) = &filter.text {
            let text = text.as_str().to_owned();
            sql.push(" AND (instr(lower(c.id), lower(")
                .push_bind(text.clone())
                .push(")) > 0 OR instr(lower(COALESCE(c.model, '')), lower(")
                .push_bind(text.clone())
                .push(")) > 0 OR EXISTS (SELECT 1 FROM json_each(c.tool_names) t WHERE instr(lower(t.value), lower(")
                .push_bind(text)
                .push(")) > 0))");
        }
        if let Some(session) = filter.session {
            sql.push(" AND c.session = ").push_bind(session.to_string());
        }
        if let Some(client) = filter.client {
            // The harness is known from the convention that attributed the call.
            sql.push(" AND json_extract(c.attribution, '$.hint.source') IN (");
            let mut sources = sql.separated(", ");
            for source in HintSource::ALL
                .iter()
                .filter(|source| source.harness() == client)
            {
                sources.push_bind(source.label());
            }
            sql.push(")");
        }
        if let Some(kind) = filter.kind {
            // A WebSocket handshake is recorded as a GET on a model endpoint.
            sql.push(match kind {
                KindFilter::Model => " AND c.method <> ",
                KindFilter::Other => " AND NOT (c.method <> ",
            })
            .push_bind(Method::Get.label())
            .push(" AND c.endpoint IN (");
            let mut endpoints = sql.separated(", ");
            for endpoint in Endpoint::ALL.iter().filter(|endpoint| endpoint.generates()) {
                endpoints.push_bind(endpoint.label());
            }
            sql.push(match kind {
                KindFilter::Model => ")",
                KindFilter::Other => "))",
            });
        }
        if let Some(tools) = filter.tools {
            sql.push(match tools {
                ToolFilter::WithTools => " AND c.tool_count > 0",
                ToolFilter::WithoutTools => " AND c.tool_count = 0 AND c.state <> 'open'",
            });
        }
        if let Some(status) = filter.status {
            match status {
                StatusFilter::Active => {
                    sql.push(" AND c.state = ")
                        .push_bind(CallState::Open.label());
                }
                StatusFilter::Finished => {
                    sql.push(" AND c.state = ")
                        .push_bind(CallState::Finished.label());
                }
                StatusFilter::Interrupted => {
                    sql.push(" AND (c.state = ")
                        .push_bind(CallState::Recovered.label())
                        .push(" OR c.terminal IN (");
                    Self::push_terminals(sql, TerminalClass::Stopped);
                    sql.push("))");
                }
                StatusFilter::Errors => {
                    sql.push(" AND (c.status >= 400 OR c.terminal IN (");
                    Self::push_terminals(sql, TerminalClass::Failed);
                    sql.push(") OR c.outcome = ")
                        .push_bind(ModelOutcome::Failed.label())
                        .push(")");
                }
            }
        }
    }
}

impl CallPage {
    fn from_rows(rows: &[sqlx::sqlite::SqliteRow], limit: PageLimit) -> Result<Self, StoreError> {
        let mut calls = rows
            .iter()
            .take(limit.rows())
            .map(CallView::from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let next = (rows.len() > limit.rows())
            .then(|| calls.last().map(CallView::cursor))
            .flatten();
        calls.shrink_to_fit();
        Ok(Self { calls, next })
    }
}

/// Agent activity, read from stored bodies when asked (`crate::activity`).
impl Reader {
    /// The most of one body read for activity: enough for a long
    /// conversation, which clients resend on every call.
    const ACTIVITY_READ: PrefixLimit = PrefixLimit {
        bytes: 16 * 1024 * 1024,
        segments: 4096,
    };
    /// The most calls one turn's activity reads.
    const MAX_TURN_CALLS: i64 = 100;
    /// The most calls a session report reads, oldest first.
    const MAX_REPORT_CALLS: i64 = 5_000;
    /// Calls read for a turn listing's prompt.
    const PROMPT_CALLS: i64 = 3;

    /// One turn: its prompt, calls, tool calls, and effects.
    pub async fn turn_activity(
        &self,
        turn: TurnId,
        budget: ActivityBudget,
    ) -> Result<TurnActivity, StoreError> {
        let row = sqlx::query("SELECT id, session, native_id FROM turns WHERE id = ?")
            .bind(turn.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StoreError::NotFound)?;
        let columns = Columns(&row);
        let head = TurnHead {
            turn: columns.id("id")?,
            session: columns.id("session")?,
            native_id: columns.text("native_id")?.unwrap_or_default(),
        };
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM calls WHERE turn = ?")
            .bind(turn.to_string())
            .fetch_one(&self.pool)
            .await?;
        let views = self.turn_calls(turn, Self::MAX_TURN_CALLS).await?;
        let read = i64::try_from(views.len()).unwrap_or(i64::MAX);
        let mut calls = Vec::with_capacity(views.len());
        for view in views {
            calls.push(self.read_call(view, Reach::Exchange).await?);
        }
        let omitted = u64::try_from(total.saturating_sub(read)).unwrap_or(0);
        Ok(TurnActivity::assemble(
            head,
            calls,
            omitted,
            budget,
            &self.scanner,
        ))
    }

    /// A session's turns, newest first, each with the start of its prompt.
    /// Prompts come from `index` where it has the turn's first call, and are
    /// read from request bodies only for turns it has not indexed yet.
    pub async fn session_turns(
        &self,
        session: SessionId,
        limit: PageLimit,
        index: Option<&SearchIndex>,
    ) -> Result<Vec<TurnSummary>, StoreError> {
        self.require_session(session).await?;
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT t.id, t.native_id, MIN(c.started_at) AS started_at, \
             CASE WHEN COUNT(c.ended_at) = COUNT(*) THEN MAX(c.ended_at) END AS ended_at, ",
        );
        CallQuery::push_usage_columns(&mut sql);
        sql.push(" FROM turns t JOIN calls c ON c.turn = t.id WHERE t.session = ")
            .push_bind(session.to_string())
            .push(" GROUP BY t.id ORDER BY started_at DESC, t.id DESC LIMIT ")
            .push_bind(i64::from(limit.get()));
        let rows = sql.build().fetch_all(&self.pool).await?;
        let ids = rows
            .iter()
            .map(|row| Columns(row).id::<TurnId>("id"))
            .collect::<Result<Vec<_>, _>>()?;
        let indexed = match index {
            Some(index) => index.turn_prompts(&ids).await?,
            None => HashMap::new(),
        };
        let mut turns = Vec::with_capacity(rows.len());
        for (row, turn) in rows.iter().zip(ids) {
            let columns = Columns(row);
            let started_at = columns.time("started_at")?;
            let prompt = match indexed.get(&turn) {
                Some(IndexedPrompt {
                    prompt: Some(prompt),
                    ..
                }) => Some(prompt.clone()),
                Some(IndexedPrompt {
                    earliest_call,
                    prompt: None,
                }) if *earliest_call == started_at => None,
                Some(_) | None => self.read_prompt(turn).await?,
            };
            turns.push(TurnSummary {
                turn,
                native_id: columns.text("native_id")?.unwrap_or_default(),
                started_at,
                ended_at: columns.optional_time("ended_at")?,
                call_count: columns.count("calls")?,
                error_count: columns.count("errors")?,
                tokens: TokenTotals::from_row(row)?,
                prompt,
            });
        }
        Ok(turns)
    }

    /// A turn's prompt read from the request bodies of its first calls, for
    /// a turn the search index has not reached.
    async fn read_prompt(&self, turn: TurnId) -> Result<Option<Excerpt>, StoreError> {
        for view in self.turn_calls(turn, Self::PROMPT_CALLS).await? {
            let call = self.read_call(view, Reach::Request).await?;
            if let Some(text) = call.prompt() {
                return Ok(Some(Excerpt::cut(
                    &text,
                    ActivityBudget::SUMMARY.prompt_chars,
                )));
            }
        }
        Ok(None)
    }

    /// A session's context and cost measurements, from call records alone:
    /// no body is read, so it stays fast for long sessions.
    pub async fn session_report(&self, session: SessionId) -> Result<SessionReport, StoreError> {
        self.require_session(session).await?;
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM calls WHERE session = ?")
            .bind(session.to_string())
            .fetch_one(&self.pool)
            .await?;
        let calls: Vec<CallView> = sqlx::query(AssertSqlSafe(format!(
            "{} WHERE c.session = ? ORDER BY c.started_at, c.id LIMIT ?",
            CallView::SELECT
        )))
        .bind(session.to_string())
        .bind(Self::MAX_REPORT_CALLS)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(CallView::from_row)
        .collect::<Result<_, _>>()?;
        let read = i64::try_from(calls.len()).unwrap_or(i64::MAX);
        let omitted = u64::try_from(total.saturating_sub(read)).unwrap_or(0);
        Ok(SessionReport::build(session, &calls, omitted))
    }

    /// One call's activity, with results matched only within the call.
    pub async fn call_activity(
        &self,
        call: CallId,
        budget: ActivityBudget,
    ) -> Result<CallActivity, StoreError> {
        let view = self.call_view(call).await?;
        Ok(self
            .read_call(view, Reach::Exchange)
            .await?
            .activity(budget))
    }

    /// Finished calls from `cursor` on, in end-time and ID order.
    pub(crate) async fn finished_calls(
        &self,
        cursor: IndexCursor,
        limit: i64,
    ) -> Result<Vec<(UnixMillis, CallId)>, StoreError> {
        let millis = |time: UnixMillis| i64::try_from(time.as_millis()).unwrap_or(i64::MAX);
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT id, ended_at FROM calls WHERE ended_at IS NOT NULL",
        );
        match cursor {
            IndexCursor::Start => {}
            IndexCursor::After(ended, call) => {
                sql.push(" AND (ended_at, id) > (")
                    .push_bind(millis(ended))
                    .push(", ")
                    .push_bind(call.to_string())
                    .push(")");
            }
            IndexCursor::Since(ended) => {
                sql.push(" AND ended_at >= ").push_bind(millis(ended));
            }
        }
        sql.push(" ORDER BY ended_at, id LIMIT ").push_bind(limit);
        sql.build()
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|row| {
                let columns = Columns(row);
                Ok((columns.time("ended_at")?, columns.id("id")?))
            })
            .collect()
    }

    /// Which of `calls` the store still has.
    pub(crate) async fn existing_calls(
        &self,
        calls: &[CallId],
    ) -> Result<HashSet<CallId>, StoreError> {
        let mut existing = HashSet::new();
        for chunk in calls.chunks(500) {
            let mut sql = QueryBuilder::<Sqlite>::new("SELECT id FROM calls WHERE id IN (");
            let mut list = sql.separated(", ");
            for call in chunk {
                list.push_bind(call.to_string());
            }
            sql.push(")");
            for row in sql.build().fetch_all(&self.pool).await? {
                existing.insert(Columns(&row).id("id")?);
            }
        }
        Ok(existing)
    }

    /// A call's searchable documents, read from its bodies.
    pub(crate) async fn read_documents(
        &self,
        call: CallId,
        max_chars: usize,
    ) -> Result<CallDocuments, StoreError> {
        let view = self.call_view(call).await?;
        Ok(self
            .read_call(view, Reach::Exchange)
            .await?
            .documents(max_chars, &self.scanner))
    }

    /// `NotFound` unless the store has `session`.
    async fn require_session(&self, session: SessionId) -> Result<(), StoreError> {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?)")
            .bind(session.to_string())
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

    /// A call with its bodies read, as far as `reach`. A body the call's
    /// view says was never stored is not looked up.
    async fn read_call(&self, view: CallView, reach: Reach) -> Result<ReadCall, StoreError> {
        let Some(protocol) = view.endpoint.known().and_then(Protocol::for_endpoint) else {
            let unsupported = vec![ActivityLimitation::UnsupportedEndpoint];
            return Ok(ReadCall {
                view,
                request: RequestActivity {
                    limitations: unsupported,
                    ..RequestActivity::default()
                },
                response: ResponseActivity::default(),
            });
        };
        let reader = ActivityReader::new(protocol, ActivityLimits::default());
        let request = match view.request {
            Some(_) => match self.captured(BodyKey::Request(view.id)).await? {
                Some(body) => reader.request(body.as_captured()),
                None => RequestActivity::default(),
            },
            None => RequestActivity::default(),
        };
        let response = match (reach, &view.response) {
            (Reach::Request, _) | (Reach::Exchange, None) => ResponseActivity::default(),
            (Reach::Exchange, Some(_)) => match self.body_key(view.id, Direction::Response).await {
                Ok(key) => match self.captured(key).await? {
                    Some(body) => reader.response(body.as_captured()),
                    None => ResponseActivity::default(),
                },
                Err(StoreError::NotFound) => ResponseActivity::default(),
                Err(error) => return Err(error),
            },
        };
        Ok(ReadCall {
            view,
            request,
            response,
        })
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
                .push_bind(i64::try_from(from.as_millis()).unwrap_or(i64::MAX));
        }
        if let Some(until) = self.until() {
            sql.push(" AND c.started_at <= ")
                .push_bind(i64::try_from(until.as_millis()).unwrap_or(i64::MAX));
        }
    }
}

/// How much of a call's exchange to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// Only the request: what the user asked.
    Request,
    /// The request and the final attempt's response.
    Exchange,
}

/// A stored body prefix, ready for the activity reader.
struct CapturedPrefix {
    bytes: Vec<u8>,
    extent: BodyExtent,
    encoding: Option<ResponseEncoding>,
    messages: Option<Vec<std::ops::Range<usize>>>,
}

impl CapturedPrefix {
    fn new(prefix: BodyPrefix, messages: &[(i64, i64)]) -> Self {
        let extent = if prefix.gap {
            BodyExtent::Gap
        } else if prefix.truncated || !prefix.body.complete() {
            BodyExtent::Truncated
        } else {
            BodyExtent::Complete
        };
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
            bytes: prefix.bytes,
            extent,
            messages,
        }
    }

    fn as_captured(&self) -> CapturedBody<'_> {
        CapturedBody {
            bytes: &self.bytes,
            extent: self.extent,
            encoding: self.encoding,
            messages: self.messages.as_deref(),
        }
    }
}
