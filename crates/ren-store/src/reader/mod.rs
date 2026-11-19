//! Read-only access through a pool of query-only connections. Each query uses
//! its own short transaction; readers never block the writer.
//!
//! The history listing's SQL is built in `calls`; the reads assembled from
//! stored digests (turns, reports, conversations) are in `activity`.

mod activity;
mod calls;

use super::chunk::{ChunkHash, Codec, EncodedChunk};
use crate::millis::SqlMillis;
use crate::{
    Store, StoreError,
    query::{CallQuery, Cursor, PageLimit, Shelf, TimeRange},
    view::{
        AttachmentView, AttemptView, BodyPrefix, BodyView, CallDetail, CallPage, CallView,
        ChannelView, Columns, DiskUsage, MessageView, OperatorActionView, ProjectView,
        RecordingView, RunView, SessionView, StorageView, UsageGroup, UsageView, WorkItemView,
    },
    writer::BlobAccess,
};
use ren_depot::toolset::Generation;
use ren_protocol::{
    direction::Direction,
    label::{Labeled, Stored},
};
use ren_recorder::Observation;
use ren_recorder::batch::BodyKey;
use ren_recorder::{
    capture::CallState,
    id::{CallId, RecordingId, RunId, SessionId, WorkItemId},
    project::{ProjectName, RepositoryPath},
};
use sqlx::{
    AssertSqlSafe, QueryBuilder, Row, Sqlite, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::collections::HashMap;
use std::{path::Path, sync::Arc, time::Duration};

/// Bounds on a body prefix read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefixLimit {
    pub bytes: usize,
    pub segments: u32,
}

#[derive(Clone)]
pub struct Reader {
    pub(crate) pool: SqlitePool,
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
        Ok(Self { pool })
    }

    /// Newest calls first, filtered.
    pub async fn calls(&self, query: &CallQuery) -> Result<CallPage, StoreError> {
        let mut sql = QueryBuilder::<Sqlite>::new(CallView::SELECT);
        sql.push(" WHERE 1 = 1");
        if let Some(cursor) = query.after {
            sql.push(" AND (c.started_at, c.id) < (")
                .push_bind(SqlMillis::at(cursor.started_at))
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

    /// A recording's calls in chronological order.
    pub async fn recording_calls(
        &self,
        recording: RecordingId,
        limit: PageLimit,
        after: Option<Cursor>,
    ) -> Result<CallPage, StoreError> {
        self.require_recording(recording).await?;
        let (time, call) = after.map_or((SqlMillis::BEFORE_EPOCH, String::new()), |cursor| {
            (SqlMillis::at(cursor.started_at), cursor.call.to_string())
        });
        let rows = sqlx::query(AssertSqlSafe(format!(
            "{} WHERE c.recording = ? AND (c.started_at, c.id) > (?, ?) ORDER BY c.started_at, c.id LIMIT ?",
            CallView::SELECT
        )))
        .bind(recording.to_string())
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

    /// The `limit` most recently active recordings on `shelf`.
    pub async fn recordings(
        &self,
        limit: PageLimit,
        shelf: Shelf,
    ) -> Result<Vec<RecordingView>, StoreError> {
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT s.id, s.provider, s.source, s.native_id, s.first_seen, s.last_seen, CASE WHEN s.archived_at >= s.last_seen THEN s.archived_at END AS archived_at, (SELECT COUNT(*) FROM turns WHERE recording = s.id) AS turn_count, ",
        );
        CallQuery::push_usage_columns(&mut sql);
        sql.push(" FROM recordings s LEFT JOIN calls c ON c.recording = s.id WHERE ")
            .push(shelf.recordings())
            .push(" GROUP BY s.id ORDER BY s.last_seen DESC, s.id DESC LIMIT ")
            .push_bind(i64::from(limit.get()));
        sql.build()
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(RecordingView::from_row)
            .collect()
    }

    /// Calls, errors, and tokens of the calls started in `range`: in total,
    /// by the hour, by model, and by user.
    pub async fn usage(&self, range: TimeRange) -> Result<UsageView, StoreError> {
        // One scan, grouped by hour, model, and user; the totals and each
        // breakdown are summed from its groups.
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT (c.started_at / 3600000) * 3600000 AS start, c.provider, c.model, c.user, ",
        );
        CallQuery::push_usage_columns(&mut sql);
        sql.push(" FROM calls c WHERE 1 = 1");
        range.push_started(&mut sql);
        sql.push(" GROUP BY start, c.provider, c.model, c.user");
        let groups = sql
            .build()
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(UsageGroup::from_row)
            .collect::<Result<_, StoreError>>()?;
        Ok(UsageView::fold(groups))
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

    /// The toolset generations runs without an end were started with:
    /// generations a collection must keep. A run left by a daemon that
    /// stopped counts until the next startup marks it recovered.
    pub async fn live_toolsets(&self) -> Result<Vec<Generation>, StoreError> {
        let numbers: Vec<i64> = sqlx::query_scalar(
            "SELECT DISTINCT toolset_generation FROM runs WHERE ended_at IS NULL AND toolset_generation IS NOT NULL ORDER BY toolset_generation",
        )
        .fetch_all(&self.pool)
        .await?;
        numbers
            .into_iter()
            .map(|number| {
                u64::try_from(number)
                    .map(Generation::from)
                    .map_err(|_| StoreError::Corrupt("run toolset"))
            })
            .collect()
    }

    /// The newest `limit` runs on `shelf`, newest first.
    pub async fn runs(&self, limit: PageLimit, shelf: Shelf) -> Result<Vec<RunView>, StoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "{} WHERE {} ORDER BY started_at DESC, id DESC LIMIT ?",
            RunView::SELECT,
            shelf.runs(),
        )))
        .bind(i64::from(limit.get()))
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(RunView::from_row)
        .collect()
    }

    /// Operator sessions on `shelf`, most recently active first (by their
    /// newest run, else their creation), each with its runs.
    pub async fn sessions(
        &self,
        limit: PageLimit,
        shelf: Shelf,
    ) -> Result<Vec<SessionView>, StoreError> {
        let rows = sqlx::query(AssertSqlSafe(format!(
            "{} WHERE {} ORDER BY COALESCE((SELECT MAX(started_at) FROM runs WHERE runs.operator_session = operator_sessions.id), created_at) DESC, id DESC LIMIT ?",
            SessionView::select(),
            shelf.sessions(),
        )))
        .bind(i64::from(limit.get()))
        .fetch_all(&self.pool)
        .await?;
        let ids = rows
            .iter()
            .map(|row| row.try_get::<String, _>("id"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut runs = self.sessions_runs(&ids).await?;
        rows.iter()
            .zip(&ids)
            .map(|(row, id)| SessionView::from_row(row, runs.remove(id).unwrap_or_default()))
            .collect()
    }

    /// One operator session, with its runs, if it was recorded.
    pub async fn session(&self, id: SessionId) -> Result<Option<SessionView>, StoreError> {
        let id = id.to_string();
        let Some(row) = sqlx::query(AssertSqlSafe(format!(
            "{} WHERE id = ?",
            SessionView::select()
        )))
        .bind(&id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        SessionView::from_row(&row, self.session_runs(&id).await?).map(Some)
    }

    /// The attachments uploaded to `session`, oldest first.
    pub async fn attachments(&self, session: SessionId) -> Result<Vec<AttachmentView>, StoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "{} WHERE operator_session = ? ORDER BY added_at, id",
            AttachmentView::SELECT,
        )))
        .bind(session.to_string())
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(AttachmentView::from_row)
        .collect()
    }

    /// A session's runs, newest first.
    /// The runs of each of `sessions`, newest first, in one query.
    async fn sessions_runs(
        &self,
        sessions: &[String],
    ) -> Result<HashMap<String, Vec<RunView>>, StoreError> {
        let mut grouped = HashMap::<String, Vec<RunView>>::new();
        if sessions.is_empty() {
            return Ok(grouped);
        }
        let mut sql = QueryBuilder::<Sqlite>::new(RunView::SELECT);
        sql.push(" WHERE operator_session IN (");
        let mut ids = sql.separated(", ");
        for session in sessions {
            ids.push_bind(session);
        }
        sql.push(") ORDER BY started_at DESC, id DESC");
        for row in sql.build().fetch_all(&self.pool).await? {
            let session: String = row.try_get("operator_session")?;
            grouped
                .entry(session)
                .or_default()
                .push(RunView::from_row(&row)?);
        }
        Ok(grouped)
    }

    async fn session_runs(&self, session: &str) -> Result<Vec<RunView>, StoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "{} WHERE operator_session = ? ORDER BY started_at DESC, id DESC",
            RunView::SELECT,
        )))
        .bind(session)
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(RunView::from_row)
        .collect()
    }

    /// One run, if it was recorded.
    /// Work items on `shelf`, most recently changed first.
    pub async fn work_items(
        &self,
        limit: PageLimit,
        shelf: Shelf,
    ) -> Result<Vec<WorkItemView>, StoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "{} WHERE {} ORDER BY updated_at DESC, id DESC LIMIT ?",
            WorkItemView::SELECT,
            shelf.work(),
        )))
        .bind(i64::from(limit.get()))
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(WorkItemView::from_row)
        .collect()
    }

    /// One work item, if it was recorded.
    pub async fn work_item(&self, id: WorkItemId) -> Result<Option<WorkItemView>, StoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "{} WHERE id = ?",
            WorkItemView::SELECT
        )))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(WorkItemView::from_row)
        .transpose()
    }

    /// Every registered project, archived ones too, by name.
    pub async fn projects(&self) -> Result<Vec<ProjectView>, StoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "{} ORDER BY name",
            ProjectView::SELECT
        )))
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(ProjectView::from_row)
        .collect()
    }

    /// The project named `name`, if one is registered.
    pub async fn project_named(
        &self,
        name: &ProjectName,
    ) -> Result<Option<ProjectView>, StoreError> {
        self.project_where("name", name.as_str()).await
    }

    /// The project whose repository is `repository`, if one is registered.
    pub async fn project_at(
        &self,
        repository: &RepositoryPath,
    ) -> Result<Option<ProjectView>, StoreError> {
        self.project_where("repository", &repository.to_string())
            .await
    }

    async fn project_where(
        &self,
        column: &'static str,
        value: &str,
    ) -> Result<Option<ProjectView>, StoreError> {
        sqlx::query(AssertSqlSafe(format!(
            "{} WHERE {column} = ?",
            ProjectView::SELECT
        )))
        .bind(value)
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(ProjectView::from_row)
        .transpose()
    }

    pub async fn run(&self, id: RunId) -> Result<Option<RunView>, StoreError> {
        sqlx::query(AssertSqlSafe(format!("{} WHERE id = ?", RunView::SELECT)))
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(RunView::from_row)
            .transpose()
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

    pub async fn storage(&self) -> Result<StorageView, StoreError> {
        // Before the connection below is held: with every connection of the
        // pool held so, this would wait for one that is never released.
        let digested_calls = self.digested_count().await?;
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
            digested_calls,
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
