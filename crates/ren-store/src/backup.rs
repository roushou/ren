//! Online, incremental backup of a data directory into its `backup`
//! subdirectory, while the daemon runs.
//!
//! 1. The metadata is snapshotted with `VACUUM INTO` into a private staging
//!    directory: one read transaction, so the copy is consistent.
//! 2. The chunks that snapshot references and the backup lacks are copied
//!    into `backup/blobs.sqlite3`. Chunks never change, so this appends.
//! 3. Every referenced chunk must now be present, or the backup fails and the
//!    previous one is left as it was.
//! 4. The snapshot replaces `backup/ren.sqlite3` by rename.
//! 5. Chunks the new snapshot no longer references are removed.
//!
//! Stopping after any step leaves metadata (old or new) with a superset of
//! the chunks it references. The chunk sweep is held off while a backup
//! runs, so retention cannot remove a chunk the snapshot still needs.
//!
//! The location is fixed: callers cannot choose where the daemon writes.

use crate::{SweepGate, writer::Store};
use sqlx::{
    AssertSqlSafe, Connection, QueryBuilder, Row, Sqlite, SqliteConnection,
    sqlite::SqliteConnectOptions,
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// Why a backup did not complete. The previous backup is kept.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("a backup is already running")]
    Running,
    #[error("backup I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("backup database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Store(#[from] super::StoreError),
    #[error("data directory path is not UTF-8")]
    NonUtf8Path,
    #[error("{0} referenced chunks are missing from the history; the previous backup is kept")]
    Incomplete(u64),
}

/// What one backup wrote.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BackupReport {
    /// The backup, a data directory a reader can open.
    pub directory: PathBuf,
    pub metadata_bytes: u64,
    /// Chunks the backup holds now.
    pub chunks: u64,
    pub chunks_copied: u64,
    /// Stored (compressed) bytes copied.
    pub bytes_copied: u64,
    pub chunks_removed: u64,
}

/// Backs up one data directory. One backup runs at a time.
#[derive(Debug)]
pub struct Backup {
    data_dir: PathBuf,
    sweeps: SweepGate,
    running: tokio::sync::Mutex<()>,
}

impl Backup {
    pub const DIRECTORY: &'static str = "backup";
    const STAGING: &'static str = ".staging";
    /// Chunks fetched and inserted per transaction.
    const BATCH: usize = 64;

    pub fn new(data_dir: &Path, sweeps: SweepGate) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            sweeps,
            running: tokio::sync::Mutex::new(()),
        }
    }

    pub async fn run(&self) -> Result<BackupReport, BackupError> {
        let _one = self.running.try_lock().map_err(|_| BackupError::Running)?;
        let _hold = self.sweeps.hold().await;
        let target = self.data_dir.join(Self::DIRECTORY);
        let staging = target.join(Self::STAGING);
        Self::private_dir(&target)?;
        // A staging directory left by an interrupted backup is ours to replace.
        if staging.try_exists()? {
            std::fs::remove_dir_all(&staging)?;
        }
        Self::private_dir(&staging)?;
        let snapshot = staging.join(Store::METADATA_FILE);

        let mut source = self.source().await?;
        sqlx::query("VACUUM main INTO ?")
            .bind(Self::utf8(&snapshot)?)
            .execute(&mut source)
            .await?;
        Self::private(&snapshot)?;

        let blobs = target.join(Store::BLOBS_FILE);
        let mut backup = Self::destination(&blobs).await?;
        Self::attach(&mut backup, &snapshot).await?;
        let missing: Vec<Vec<u8>> = sqlx::query_scalar(
            "SELECT DISTINCT s.chunk FROM snap.segments s WHERE NOT EXISTS (SELECT 1 FROM main.chunks c WHERE c.hash = s.chunk)",
        )
        .fetch_all(&mut backup)
        .await?;
        let (mut chunks_copied, mut bytes_copied) = (0_u64, 0_u64);
        for hashes in missing.chunks(Self::BATCH) {
            let mut select = QueryBuilder::<Sqlite>::new(
                "SELECT hash, codec, raw_len, data FROM blobs.chunks WHERE hash IN (",
            );
            let mut list = select.separated(", ");
            for hash in hashes {
                list.push_bind(hash.as_slice());
            }
            select.push(")");
            let rows = select.build().fetch_all(&mut source).await?;
            let mut tx = backup.begin().await?;
            for row in rows {
                let data: Vec<u8> = row.try_get("data")?;
                bytes_copied =
                    bytes_copied.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
                sqlx::query("INSERT OR IGNORE INTO main.chunks(hash, codec, raw_len, data) VALUES (?, ?, ?, ?)")
                    .bind(row.try_get::<Vec<u8>, _>("hash")?)
                    .bind(row.try_get::<String, _>("codec")?)
                    .bind(row.try_get::<i64, _>("raw_len")?)
                    .bind(data)
                    .execute(&mut *tx)
                    .await?;
                chunks_copied = chunks_copied.saturating_add(1);
            }
            tx.commit().await?;
        }
        source.close().await?;
        let absent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM (SELECT DISTINCT s.chunk FROM snap.segments s WHERE NOT EXISTS (SELECT 1 FROM main.chunks c WHERE c.hash = s.chunk))",
        )
        .fetch_one(&mut backup)
        .await?;
        sqlx::query("DETACH DATABASE snap")
            .execute(&mut backup)
            .await?;
        if absent > 0 {
            backup.close().await?;
            std::fs::remove_dir_all(&staging)?;
            return Err(BackupError::Incomplete(
                u64::try_from(absent).unwrap_or(u64::MAX),
            ));
        }

        let metadata = target.join(Store::METADATA_FILE);
        std::fs::File::open(&snapshot)?.sync_all()?;
        std::fs::rename(&snapshot, &metadata)?;
        std::fs::remove_dir_all(&staging)?;
        Self::attach(&mut backup, &metadata).await?;
        let chunks_removed = sqlx::query(
            "DELETE FROM main.chunks WHERE NOT EXISTS (SELECT 1 FROM snap.segments s WHERE s.chunk = main.chunks.hash)",
        )
        .execute(&mut backup)
        .await?
        .rows_affected();
        let chunks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM main.chunks")
            .fetch_one(&mut backup)
            .await?;
        sqlx::query("DETACH DATABASE snap")
            .execute(&mut backup)
            .await?;
        backup.close().await?;
        Ok(BackupReport {
            metadata_bytes: std::fs::metadata(&metadata)?.len(),
            directory: target,
            chunks: u64::try_from(chunks).unwrap_or(0),
            chunks_copied,
            bytes_copied,
            chunks_removed,
        })
    }

    /// A read-only connection to the live history. `query_only` would refuse
    /// `VACUUM INTO`, so this is not a [`Reader`](super::Reader) connection.
    async fn source(&self) -> Result<SqliteConnection, BackupError> {
        let options = SqliteConnectOptions::new()
            .filename(self.data_dir.join(Store::METADATA_FILE))
            .read_only(true)
            .busy_timeout(Duration::from_secs(5));
        let mut connection = SqliteConnection::connect_with(&options).await?;
        Store::attach_blobs(
            &mut connection,
            &self.data_dir,
            super::writer::BlobAccess::Read,
        )
        .await?;
        Ok(connection)
    }

    /// The backup's chunk file, created on first use. It keeps the rollback
    /// journal, so a backup is two self-contained files.
    async fn destination(path: &Path) -> Result<SqliteConnection, BackupError> {
        Store::private_file(path)?;
        let options = SqliteConnectOptions::new()
            .filename(path)
            .busy_timeout(Duration::from_secs(5));
        let mut connection = SqliteConnection::connect_with(&options).await?;
        for statement in [
            format!("PRAGMA page_size = {}", Store::PAGE_SIZE_BLOBS),
            format!(
                "CREATE TABLE IF NOT EXISTS main.chunks {}",
                Store::CHUNKS_COLUMNS
            ),
        ] {
            sqlx::query(AssertSqlSafe(statement))
                .execute(&mut connection)
                .await?;
        }
        Ok(connection)
    }

    async fn attach(connection: &mut SqliteConnection, path: &Path) -> Result<(), BackupError> {
        sqlx::query("ATTACH DATABASE ? AS snap")
            .bind(Self::utf8(path)?)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    fn utf8(path: &Path) -> Result<&str, BackupError> {
        path.to_str().ok_or(BackupError::NonUtf8Path)
    }

    fn private_dir(path: &Path) -> Result<(), BackupError> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    fn private(path: &Path) -> Result<(), BackupError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(not(unix))]
        let _ = path;
        Ok(())
    }
}
