//! Read-only access through a pool of query-only connections. Each query uses
//! its own short transaction; readers never block the writer.

use super::chunk::{ChunkHash, Codec, EncodedChunk};
use crate::{
    Store, StoreError,
    query::{CallQuery, Cursor, PageLimit, StatusFilter, ToolFilter},
    view::{
        AttemptView, BodyPrefix, BodyView, CallDetail, CallPage, CallView, ChannelView, Columns,
        DiskUsage, MessageView, OperatorActionView, SessionView, StorageView,
    },
    writer::BlobAccess,
};
use sqlx::{
    AssertSqlSafe, QueryBuilder, Row, Sqlite, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{path::Path, sync::Arc, time::Duration};
use tgv_protocol::inspect::{InputAccounting, ModelOutcome};
use tgv_protocol::{
    direction::Direction,
    label::{Labeled, Stored},
};
use tgv_recorder::Observation;
use tgv_recorder::batch::BodyKey;
use tgv_recorder::{
    capture::{AttemptTerminal, CallState, TerminalClass},
    id::{CallId, SessionId},
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
        let pool = SqlitePoolOptions::new()
            .max_connections(connections.max(1))
            .acquire_timeout(Duration::from_secs(5))
            .after_connect(move |connection, _| {
                let dir = dir.clone();
                Box::pin(
                    async move { Store::attach_blobs(connection, &dir, BlobAccess::Read).await },
                )
            })
            .connect_with(options)
            .await?;
        Ok(Self { pool })
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
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?)")
            .bind(session.to_string())
            .fetch_one(&self.pool)
            .await?;
        if !exists {
            return Err(StoreError::NotFound);
        }
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
            "SELECT s.id, s.provider, s.source, s.native_id, s.first_seen, s.last_seen, (SELECT COUNT(*) FROM calls WHERE session = s.id) AS call_count, (SELECT COUNT(*) FROM turns WHERE session = s.id) AS turn_count, (SELECT COUNT(*) FROM calls c WHERE c.session = s.id",
        );
        // The same predicate as the `errors` status filter.
        sql.push(" AND (c.status >= 400 OR c.terminal IN (");
        CallQuery::push_terminals(&mut sql, TerminalClass::Failed);
        sql.push(") OR c.outcome = ")
            .push_bind(ModelOutcome::Failed.label())
            .push(")) AS error_count, ");
        // SUM over no reported usage is NULL, kept as "not reported".
        sql.push("(SELECT SUM(COALESCE(json_extract(c.usage, '$.input_tokens'), 0) + CASE WHEN json_extract(c.usage, '$.accounting') = ")
            .push_bind(InputAccounting::ExcludesCache.label())
            .push(" THEN COALESCE(json_extract(c.usage, '$.cache_read_tokens'), 0) + COALESCE(json_extract(c.usage, '$.cache_creation_tokens'), 0) ELSE 0 END) FROM calls c WHERE c.session = s.id AND c.usage IS NOT NULL) AS input_tokens, ")
            .push("(SELECT SUM(json_extract(c.usage, '$.output_tokens')) FROM calls c WHERE c.session = s.id AND c.usage IS NOT NULL) AS output_tokens ")
            .push("FROM sessions s ORDER BY s.last_seen DESC, s.id DESC LIMIT ")
            .push_bind(i64::from(limit.get()));
        sql.build()
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(SessionView::from_row)
            .collect()
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
        let disk = DiskUsage::measure(&mut connection).await?;
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
        if let Some(from) = filter.time.from() {
            sql.push(" AND c.started_at >= ")
                .push_bind(i64::try_from(from.as_millis()).unwrap_or(i64::MAX));
        }
        if let Some(until) = filter.time.until() {
            sql.push(" AND c.started_at <= ")
                .push_bind(i64::try_from(until.as_millis()).unwrap_or(i64::MAX));
        }
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
