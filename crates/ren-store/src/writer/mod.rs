//! The single writer: ownership, migrations, recovery, commits, and retention.
//! How each record of a batch is written is in `calls` (calls, channels,
//! attempts, bodies) and `runs` (operator, sessions, runs, history).

mod calls;
mod runs;

use super::chunk::{ChunkHash, EncodedChunk};
use crate::Reader;
use crate::millis::SqlMillis;
use crate::{
    StoreError,
    view::{DiskUsage, FileUsage},
};
use bytes::Bytes;
use ren_protocol::{
    label::{Labeled, Stored},
    route::Transport,
};
use ren_recorder::digest::{RederivePass, StoredExchange};
use ren_recorder::{Committed, Observation, Pruned, RecordSink};
use ren_recorder::{
    batch::{Batch, ObservationKind, Op, Stamp},
    id::AttemptId,
    retention::RetentionPolicy,
};
use ren_recorder::{
    capture::{BodyEnding, CallState, ChannelEnd, RunEnd},
    id::{CallId, ChannelId, IdSource, RunId},
    routing::OperatorRevision,
    time::{Clock, Moment, UnixMillis},
};
use sqlx::{
    AssertSqlSafe, Connection, QueryBuilder, Row, Sqlite, SqliteConnection, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    future::Future,
    path::Path,
    sync::Arc,
    time::Duration,
};

/// Owns the data directory for one daemon run.
pub struct Store {
    connection: SqliteConnection,
    ids: Arc<dyn IdSource>,
    sweeps: SweepGate,
    /// Reads stored bodies back, for digests derived again.
    bodies: Reader,
    _lock: File,
}

/// Holds off the chunk sweep while a backup copies chunks: retention may
/// remove calls, but not the chunks a backup's snapshot still references.
#[derive(Debug, Clone, Default)]
pub struct SweepGate(Arc<tokio::sync::RwLock<()>>);

/// While held, the sweep skips its pass; skipped chunks go on a later pass.
#[must_use = "the sweep resumes when the hold is dropped"]
pub struct SweepHold(
    #[expect(dead_code, reason = "held for its drop")] tokio::sync::OwnedRwLockReadGuard<()>,
);

impl SweepGate {
    /// Wait for a sweep in progress to finish, then hold off the next ones.
    pub async fn hold(&self) -> SweepHold {
        SweepHold(self.0.clone().read_owned().await)
    }
}

/// What startup found from the previous run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recovery {
    /// Calls left open by a daemon that stopped; now marked recovered.
    pub calls: u64,
    /// Agent runs a stopped daemon never saw end; now marked recovered.
    pub runs: u64,
    /// The operator state the last recorded command made, as stored; `None`
    /// in a new data directory.
    pub operator: Option<StoredOperatorState>,
}

/// The stored operator snapshot: the revision recorded beside it and its
/// document, unparsed. The gateway owns the document's format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOperatorState {
    pub revision: OperatorRevision,
    pub document: Box<str>,
}

/// The payload of a run's recovery observation.
#[derive(serde::Serialize)]
struct RecoveredRun {
    run: RunId,
}

const PRUNE_BATCH: i64 = 16;
const OPERATOR_ACTIONS_KEPT: i64 = 10_000;
const SWEEP_BATCH: i64 = 512;
const JOURNAL_SIZE_LIMIT: i64 = 64 * 1024 * 1024;

impl Store {
    pub(super) const LOCK_FILE: &'static str = "daemon.lock";
    pub const METADATA_FILE: &'static str = "ren.sqlite3";
    pub(super) const BLOBS_FILE: &'static str = "blobs.sqlite3";
    /// The chunk table's definition, shared with backups.
    pub(super) const CHUNKS_COLUMNS: &'static str = "(hash BLOB PRIMARY KEY, codec TEXT NOT NULL, raw_len INTEGER NOT NULL, data BLOB NOT NULL) WITHOUT ROWID";
    pub(super) const PAGE_SIZE_BLOBS: u32 = 16_384;

    /// Take ownership of `dir`, migrate, and recover calls the previous run
    /// left open. Fails with [`StoreError::Locked`] if another daemon owns it.
    pub async fn open<C: Clock + ?Sized>(
        dir: &Path,
        clock: &C,
        ids: Arc<dyn IdSource>,
    ) -> Result<(Self, Recovery), StoreError> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(dir)?;
        let lock = Self::private_file(&dir.join(Self::LOCK_FILE))?;
        lock.try_lock().map_err(|_| StoreError::Locked)?;
        let mut recovery = Recovery::default();
        Self::private_file(&dir.join(Self::METADATA_FILE))?;
        Self::private_file(&dir.join(Self::BLOBS_FILE))?;
        let options = SqliteConnectOptions::new()
            .filename(dir.join(Self::METADATA_FILE))
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .pragma("journal_size_limit", JOURNAL_SIZE_LIMIT.to_string())
            .busy_timeout(Duration::from_secs(5));
        let mut connection = SqliteConnection::connect_with(&options).await?;
        sqlx::migrate!("./migrations").run(&mut connection).await?;
        Self::attach_blobs(&mut connection, dir, BlobAccess::Write).await?;
        let mut store = Self {
            connection,
            ids,
            sweeps: SweepGate::default(),
            bodies: Reader::open(dir, 1).await?,
            _lock: lock,
        };
        recovery.calls = store.recover(clock.now()).await?;
        recovery.runs = store.recover_runs(clock.now()).await?;
        recovery.operator = store.operator_state().await?;
        Ok((store, recovery))
    }

    /// The stored operator snapshot, as stored.
    async fn operator_state(&mut self) -> Result<Option<StoredOperatorState>, StoreError> {
        let stored: Option<(i64, String)> =
            sqlx::query_as("SELECT revision, state FROM operator_state WHERE id = 1")
                .fetch_optional(&mut self.connection)
                .await?;
        stored
            .map(|(revision, document)| {
                Ok(StoredOperatorState {
                    revision: OperatorRevision::restore(
                        u64::try_from(revision)
                            .map_err(|_| StoreError::Corrupt("negative operator revision"))?,
                    ),
                    document: document.into(),
                })
            })
            .transpose()
    }

    pub(super) fn private_file(path: &Path) -> Result<File, StoreError> {
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        Ok(options.open(path)?)
    }

    pub(super) async fn attach_blobs(
        connection: &mut SqliteConnection,
        dir: &Path,
        access: BlobAccess,
    ) -> Result<(), sqlx::Error> {
        let path = dir.join(Self::BLOBS_FILE);
        let path = path
            .to_str()
            .ok_or_else(|| sqlx::Error::Configuration(StoreError::NonUtf8Path.into()))?;
        sqlx::query("ATTACH DATABASE ? AS blobs")
            .bind(path)
            .execute(&mut *connection)
            .await?;
        match access {
            BlobAccess::Read => {}
            BlobAccess::Write => {
                // Page size only applies before the first table exists.
                for pragma in [
                    format!("PRAGMA blobs.page_size = {}", Self::PAGE_SIZE_BLOBS),
                    "PRAGMA blobs.journal_mode = WAL".to_owned(),
                    "PRAGMA blobs.synchronous = NORMAL".to_owned(),
                    format!("PRAGMA blobs.journal_size_limit = {JOURNAL_SIZE_LIMIT}"),
                    format!(
                        "CREATE TABLE IF NOT EXISTS blobs.chunks {}",
                        Self::CHUNKS_COLUMNS
                    ),
                ] {
                    sqlx::query(AssertSqlSafe(pragma))
                        .execute(&mut *connection)
                        .await?;
                }
            }
        }
        Ok(())
    }

    /// Calls the previous run never finished keep an unknown outcome: they are
    /// marked recovered, with unfinished bodies, and get an observation.
    async fn recover(&mut self, now: Moment) -> Result<u64, StoreError> {
        let mut tx = MetadataTx {
            tx: self.connection.begin().await?,
            ids: self.ids.as_ref(),
        };
        let open = sqlx::query("SELECT id, channel FROM calls WHERE state = ?")
            .bind(CallState::Open.label())
            .fetch_all(&mut *tx.tx)
            .await?;
        sqlx::query("UPDATE bodies SET ending = ? WHERE ending = ?")
            .bind(BodyEnding::Unfinished.label())
            .bind(BodyEnding::Open.label())
            .execute(&mut *tx.tx)
            .await?;
        let channels =
            sqlx::query("SELECT id FROM channels WHERE transport = ? AND ending IS NULL")
                .bind(Transport::WebSocket.label())
                .fetch_all(&mut *tx.tx)
                .await?;
        for row in &channels {
            let channel: String = row.try_get("id")?;
            sqlx::query("UPDATE channels SET ending = ? WHERE id = ?")
                .bind(ChannelEnd::Recovered.label())
                .bind(&channel)
                .execute(&mut *tx.tx)
                .await?;
        }
        for row in &open {
            let call: String = row.try_get("id")?;
            let channel: String = row.try_get("channel")?;
            let stamp = Stamp {
                at: now,
                channel: channel
                    .parse()
                    .map_err(|_| StoreError::Corrupt("identifier"))?,
                call: call
                    .parse()
                    .map_err(|_| StoreError::Corrupt("identifier"))?,
            };
            sqlx::query("UPDATE calls SET state = ?, capture_complete = 0 WHERE id = ?")
                .bind(CallState::Recovered.label())
                .bind(&call)
                .execute(&mut *tx.tx)
                .await?;
            tx.append(&stamp, None, ObservationKind::CallRecovered, &())
                .await?;
        }
        tx.commit().await?;
        Ok(u64::try_from(open.len()).unwrap_or(u64::MAX))
    }

    /// Runs the previous daemon never saw end keep an unknown outcome: they
    /// are marked recovered, and get an observation.
    async fn recover_runs(&mut self, now: Moment) -> Result<u64, StoreError> {
        let mut tx = MetadataTx {
            tx: self.connection.begin().await?,
            ids: self.ids.as_ref(),
        };
        let open: Vec<String> = sqlx::query_scalar("SELECT id FROM runs WHERE run_end IS NULL")
            .fetch_all(&mut *tx.tx)
            .await?;
        for run in &open {
            sqlx::query("UPDATE runs SET run_end = ?, ended_at = ? WHERE id = ?")
                .bind(RunEnd::Recovered.label())
                .bind(SqlMillis::at(now.wall))
                .bind(run)
                .execute(&mut *tx.tx)
                .await?;
            let run: RunId = run.parse().map_err(|_| StoreError::Corrupt("identifier"))?;
            tx.log(
                now,
                ChannelId::restore(uuid::Uuid::nil()),
                None,
                None,
                ObservationKind::RunRecovered,
                &RecoveredRun { run },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(u64::try_from(open.len()).unwrap_or(u64::MAX))
    }

    /// Commit a batch: new chunks first, then every operation and its
    /// observation in one metadata transaction. A crash between the two leaves
    /// only unreferenced chunks, which the next sweep removes.
    pub async fn commit(&mut self, batch: &Batch) -> Result<Committed, StoreError> {
        let mut committed = Committed::default();
        let hashes = self
            .store_chunks(batch.chunks().to_vec(), &mut committed)
            .await?;
        let mut tx = MetadataTx {
            tx: self.connection.begin().await?,
            ids: self.ids.as_ref(),
        };
        for op in batch.operations() {
            if let Some(observation) = tx.apply(op, &hashes).await? {
                committed.observations.push(observation);
            }
        }
        tx.commit().await?;
        Ok(committed)
    }

    async fn store_chunks(
        &mut self,
        chunks: Vec<Bytes>,
        committed: &mut Committed,
    ) -> Result<Vec<StoredChunk>, StoreError> {
        if chunks.is_empty() {
            return Ok(Vec::new());
        }
        let (chunks, hashes) = tokio::task::spawn_blocking(move || {
            let hashes: Vec<_> = chunks.iter().map(|chunk| ChunkHash::of(chunk)).collect();
            (chunks, hashes)
        })
        .await?;
        let mut existing = HashSet::new();
        let mut query =
            QueryBuilder::<Sqlite>::new("SELECT hash FROM blobs.chunks WHERE hash IN (");
        let mut list = query.separated(", ");
        for hash in &hashes {
            list.push_bind(hash.as_bytes().to_vec());
        }
        query.push(")");
        for row in query.build().fetch_all(&mut self.connection).await? {
            let bytes: Vec<u8> = row.try_get("hash")?;
            if let Some(hash) = ChunkHash::restore(&bytes) {
                existing.insert(hash);
            }
        }
        let mut missing = Vec::new();
        for (hash, chunk) in hashes.iter().zip(&chunks) {
            committed.referenced_bytes = committed
                .referenced_bytes
                .saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
            if existing.insert(*hash) {
                missing.push((*hash, chunk.clone()));
            }
        }
        let encoded = tokio::task::spawn_blocking(move || {
            missing
                .into_iter()
                .map(|(hash, raw)| EncodedChunk::encode(hash, &raw))
                .collect::<Vec<_>>()
        })
        .await?;
        let mut tx = self.connection.begin().await?;
        for chunk in &encoded {
            committed.new_bytes = committed
                .new_bytes
                .saturating_add(u64::try_from(chunk.raw_len).unwrap_or(u64::MAX));
            committed.compressed_bytes = committed
                .compressed_bytes
                .saturating_add(u64::try_from(chunk.data.len()).unwrap_or(u64::MAX));
            sqlx::query(
                "INSERT OR IGNORE INTO blobs.chunks(hash, codec, raw_len, data) VALUES (?, ?, ?, ?)",
            )
            .bind(chunk.hash.as_bytes())
            .bind(chunk.codec.label())
            .bind(i64::try_from(chunk.raw_len).unwrap_or(i64::MAX))
            .bind(chunk.data.as_slice())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(hashes
            .into_iter()
            .zip(&chunks)
            .map(|(hash, chunk)| StoredChunk {
                hash,
                len: u64::try_from(chunk.len()).unwrap_or(u64::MAX),
            })
            .collect())
    }

    /// One retention pass: delete the oldest finished calls while any limit is
    /// exceeded, then sweep chunks nothing references, and measure again.
    pub async fn prune(
        &mut self,
        policy: &RetentionPolicy,
        now: UnixMillis,
    ) -> Result<Pruned, StoreError> {
        let mut pruned = Pruned::default();
        if !policy.enabled {
            return Ok(pruned);
        }
        let cutoff = now.as_millis().saturating_sub(policy.max_age_millis());
        let retained: u64 = sqlx::query_scalar("SELECT COUNT(*) FROM calls")
            .fetch_one(&mut self.connection)
            .await?;
        let over_bytes = self.disk_usage().await?.live_bytes() > policy.max_storage_bytes;
        let over_calls = retained.saturating_sub(policy.max_calls);
        let candidates = sqlx::query(
            "SELECT id, channel, recording, started_at FROM calls WHERE state <> ? ORDER BY started_at, id LIMIT ?",
        )
        .bind(CallState::Open.label())
        .bind(PRUNE_BATCH)
        .fetch_all(&mut self.connection)
        .await?;
        let mut tx = MetadataTx {
            tx: self.connection.begin().await?,
            ids: self.ids.as_ref(),
        };
        for (index, row) in candidates.iter().enumerate() {
            let started: u64 = row.try_get("started_at")?;
            let rank = u64::try_from(index).unwrap_or(u64::MAX);
            if started >= cutoff && rank >= over_calls && !over_bytes {
                break;
            }
            let call: String = row.try_get("id")?;
            let channel: String = row.try_get("channel")?;
            let recording: Option<String> = row.try_get("recording")?;
            tx.retire(&call, &channel, recording.as_deref()).await?;
            pruned.calls = pruned.calls.saturating_add(1);
        }
        // Operator actions are few; keep the newest, with their log rows.
        for statement in [
            "DELETE FROM observations WHERE kind = ?1 AND call IS NULL AND seq < (SELECT COALESCE(MIN(seq), 0) FROM (SELECT seq FROM observations WHERE kind = ?1 ORDER BY seq DESC LIMIT ?2))",
            "DELETE FROM operator_actions WHERE seq <= (SELECT COALESCE(MAX(seq), 0) FROM operator_actions) - ?2 AND ?1 = ?1",
        ] {
            sqlx::query(statement)
                .bind(ObservationKind::OperatorAction.label())
                .bind(OPERATOR_ACTIONS_KEPT)
                .execute(&mut *tx.tx)
                .await?;
        }
        tx.commit().await?;
        pruned.chunks = match self.sweeps.0.clone().try_write_owned() {
            Ok(_sweeping) => self.sweep().await?,
            Err(_) => {
                tracing::debug!("a backup is running; chunk sweep deferred");
                0
            }
        };
        pruned.more = pruned.calls == u64::try_from(PRUNE_BATCH).unwrap_or(u64::MAX);
        Ok(pruned)
    }

    /// Mark and sweep: remove chunks no segment references. Runs between
    /// commits on the writer, so no committed chunk is waiting for segments.
    async fn sweep(&mut self) -> Result<u64, StoreError> {
        let mut total = 0;
        loop {
            let removed = sqlx::query("DELETE FROM blobs.chunks WHERE hash IN (SELECT c.hash FROM blobs.chunks c WHERE NOT EXISTS (SELECT 1 FROM main.segments s WHERE s.chunk = c.hash) LIMIT ?)")
                .bind(SWEEP_BATCH)
                .execute(&mut self.connection)
                .await?
                .rows_affected();
            total += removed;
            if removed < u64::try_from(SWEEP_BATCH).unwrap_or(u64::MAX) {
                return Ok(total);
            }
        }
    }

    /// The gate a [`Backup`](super::backup::Backup) holds while it copies.
    pub fn sweep_gate(&self) -> SweepGate {
        self.sweeps.clone()
    }

    pub async fn disk_usage(&mut self) -> Result<DiskUsage, StoreError> {
        DiskUsage::measure(&mut self.connection).await
    }

    /// Move WAL contents into the database files and restart the logs.
    pub async fn checkpoint(&mut self) -> Result<(), StoreError> {
        for pragma in [
            "PRAGMA main.wal_checkpoint(RESTART)",
            "PRAGMA blobs.wal_checkpoint(RESTART)",
        ] {
            sqlx::query(pragma).execute(&mut self.connection).await?;
        }
        Ok(())
    }

    /// Close SQLite before releasing the ownership lock.
    pub async fn close(self) -> Result<(), StoreError> {
        self.connection.close().await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum BlobAccess {
    Read,
    Write,
}

impl DiskUsage {
    /// Both store files, through `connection`.
    pub(super) async fn measure(connection: &mut SqliteConnection) -> Result<Self, StoreError> {
        Ok(Self {
            metadata: FileUsage::measure(connection, "main").await?,
            blobs: FileUsage::measure(connection, "blobs").await?,
        })
    }
}

impl FileUsage {
    pub(crate) async fn measure(
        connection: &mut SqliteConnection,
        schema: &str,
    ) -> Result<Self, StoreError> {
        let mut values = [0_u64; 3];
        for (value, pragma) in values
            .iter_mut()
            .zip(["page_size", "page_count", "freelist_count"])
        {
            *value = sqlx::query_scalar(AssertSqlSafe(format!("PRAGMA {schema}.{pragma}")))
                .fetch_one(&mut *connection)
                .await?;
        }
        let [page_size, page_count, freelist_count] = values;
        Ok(Self {
            page_size,
            page_count,
            freelist_count,
        })
    }
}

/// A chunk the metadata transaction can reference.
#[derive(Debug, Clone, Copy)]
struct StoredChunk {
    hash: ChunkHash,
    len: u64,
}

/// One metadata transaction: batch operations, their log rows, and retention
/// deletes. Observations and the query tables always change together here.
struct MetadataTx<'a> {
    tx: Transaction<'a, Sqlite>,
    ids: &'a dyn IdSource,
}

impl MetadataTx<'_> {
    const LOG_SCHEMA: i64 = 1;

    async fn commit(self) -> Result<(), StoreError> {
        self.tx.commit().await?;
        Ok(())
    }

    async fn append<P: serde::Serialize>(
        &mut self,
        stamp: &Stamp,
        attempt: Option<AttemptId>,
        kind: ObservationKind,
        payload: &P,
    ) -> Result<Observation, StoreError> {
        self.log(
            stamp.at,
            stamp.channel,
            Some(stamp.call),
            attempt,
            kind,
            payload,
        )
        .await
    }

    async fn log<P: serde::Serialize>(
        &mut self,
        at: Moment,
        channel: ChannelId,
        call: Option<CallId>,
        attempt: Option<AttemptId>,
        kind: ObservationKind,
        payload: &P,
    ) -> Result<Observation, StoreError> {
        let payload = serde_json::to_string(payload)?;
        let seq: u64 = sqlx::query_scalar("INSERT INTO observations(at_wall, at_mono, channel, call, attempt, kind, schema, payload) VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING seq")
            .bind(SqlMillis::at(at.wall))
            .bind(SqlMillis::span(at.monotonic.since_origin()))
            .bind(channel.to_string())
            .bind(call.map(|call| call.to_string()))
            .bind(attempt.map(|attempt| attempt.to_string()))
            .bind(kind.label())
            .bind(Self::LOG_SCHEMA)
            .bind(payload)
            .fetch_one(&mut *self.tx)
            .await?;
        Ok(Observation {
            seq,
            at: at.wall,
            kind: Stored::Known(kind),
            channel,
            call,
            attempt,
        })
    }

    async fn apply(
        &mut self,
        op: &Op,
        chunks: &[StoredChunk],
    ) -> Result<Option<Observation>, StoreError> {
        match op {
            Op::OpenCall(open) => self.open_call(open).await.map(Some),
            Op::StartAttempt(start) => self.start_attempt(start).await.map(Some),
            Op::ResponseHead(head) => self.response_head(head).await.map(Some),
            Op::Segments { body, segments } => {
                self.segments(*body, segments, chunks).await.map(|()| None)
            }
            Op::Message { body, mark } => self.message(*body, mark).await.map(|()| None),
            Op::FinishAttempt(finish) => self.finish_attempt(finish).await.map(Some),
            Op::FinishCall(finish) => self.finish_call(finish).await.map(Some),
            Op::CloseChannel(close) => self.close_channel(close).await.map(Some),
            Op::Operator(action) => self.operator(action).await.map(Some),
            Op::Session(record) => self.session(record).await.map(Some),
            Op::Run(record) => self.run(record).await.map(Some),
            Op::History(edit) => self.history(edit).await.map(Some),
            Op::RunEntry(entry) => self.run_entry(entry).await.map(|()| None),
            Op::Digest { call, digest } => self.digest(*call, digest).await.map(|()| None),
        }
    }
}

/// The store is the recorder's sink: each method is the inherent one.
impl RecordSink for Store {
    type Error = StoreError;

    fn commit(
        &mut self,
        batch: &Batch,
    ) -> impl Future<Output = Result<Committed, StoreError>> + Send {
        Store::commit(self, batch)
    }

    fn prune(
        &mut self,
        policy: &RetentionPolicy,
        now: UnixMillis,
    ) -> impl Future<Output = Result<Pruned, StoreError>> + Send {
        Store::prune(self, policy, now)
    }

    fn checkpoint(&mut self) -> impl Future<Output = Result<(), StoreError>> + Send {
        Store::checkpoint(self)
    }

    fn undigested(
        &mut self,
        version: u32,
        pass: RederivePass,
    ) -> impl Future<Output = Result<Vec<StoredExchange>, StoreError>> + Send {
        self.bodies.undigested(version, pass)
    }

    fn close(self) -> impl Future<Output = Result<(), StoreError>> + Send {
        Store::close(self)
    }
}
