//! Full-text search over what calls show: the user's prompts, the model's
//! replies, and tool inputs and outputs.
//!
//! The index is derived data in its own file, `search.sqlite3`, beside the
//! store's two. It is never the source of truth: [`Indexer`] fills it from
//! stored bodies through the activity reader, backfills retained history on
//! first start, and removes what retention deleted. A missing file, or one of
//! another [`SearchIndex::VERSION`], is rebuilt. It has its own single
//! writer, so indexing never contends with capture, and it is not backed up.

mod document;
mod indexer;
mod query;

pub use document::{CallDocuments, Document, Exposure, SearchField};
pub(crate) use indexer::IndexCursor;
pub use indexer::{Backlog, Indexer, PassReport};
pub use query::{
    ExposureQuery, ExposureView, SearchError, SearchHit, SearchPage, SearchQuery, SearchTerms,
};

use crate::activity::Excerpt;
use crate::view::{Columns, FileUsage, IndexUsage};
use crate::{Store, StoreError};
use sqlx::{
    Connection, QueryBuilder, Row, Sqlite, SqliteConnection, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::Duration,
};
use tgv_protocol::label::{Labeled, Stored};
use tgv_recorder::{
    id::{CallId, TurnId},
    time::UnixMillis,
};

/// What the index knows of one turn's prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedPrompt {
    /// When the turn's earliest indexed call started: when it is the turn's
    /// first call, a missing prompt means the turn has none.
    pub earliest_call: UnixMillis,
    /// The first prompt among the turn's indexed calls.
    pub prompt: Option<Excerpt>,
}

/// The search index: a pool on `search.sqlite3`. Clones share it.
#[derive(Clone)]
pub struct SearchIndex {
    pool: SqlitePool,
}

impl SearchIndex {
    pub const FILE: &str = "search.sqlite3";
    /// The index format. Another version on disk is dropped and rebuilt.
    pub const VERSION: i64 = 3;
    /// The most characters of one field of one call that are indexed.
    pub const FIELD_CHARS: usize = 32 * 1024;
    const SCHEMA: [&str; 6] = [
        "CREATE TABLE IF NOT EXISTS exposures (call TEXT NOT NULL, field TEXT NOT NULL, kind TEXT NOT NULL, hint TEXT NOT NULL, fingerprint TEXT NOT NULL, count INTEGER NOT NULL)",
        "CREATE INDEX IF NOT EXISTS exposures_call ON exposures(call)",
        "CREATE TABLE IF NOT EXISTS indexed_calls (call TEXT PRIMARY KEY, session TEXT, turn TEXT, client TEXT, started_at INTEGER NOT NULL, prompt TEXT, prompt_omitted INTEGER) WITHOUT ROWID",
        "CREATE INDEX IF NOT EXISTS indexed_calls_started ON indexed_calls(started_at)",
        "CREATE INDEX IF NOT EXISTS indexed_calls_turn ON indexed_calls(turn, started_at)",
        "CREATE VIRTUAL TABLE IF NOT EXISTS documents USING fts5(text, call UNINDEXED, field UNINDEXED, tokenize = 'unicode61 remove_diacritics 2')",
    ];

    /// Open the index for writing and reading, creating or rebuilding it.
    /// The index is derived, so a file that cannot be opened or fails its
    /// integrity check is deleted and built again rather than stopping the
    /// daemon.
    pub async fn open(dir: &Path) -> Result<Self, StoreError> {
        match Self::connect(dir).await {
            Ok(index) => Ok(index),
            Err(error) => {
                tracing::warn!(%error, "search index unusable; rebuilding it");
                for suffix in ["", "-wal", "-shm"] {
                    match std::fs::remove_file(dir.join(format!("{}{suffix}", Self::FILE))) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                Self::connect(dir).await
            }
        }
    }

    async fn connect(dir: &Path) -> Result<Self, StoreError> {
        let path = dir.join(Self::FILE);
        Store::private_file(&path)?;
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await?;
        let check: String = sqlx::query_scalar("PRAGMA quick_check")
            .fetch_one(&pool)
            .await?;
        if check != "ok" {
            pool.close().await;
            return Err(StoreError::Corrupt(
                "search index failed its integrity check",
            ));
        }
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await?;
        if version != Self::VERSION {
            let mut tx = pool.begin().await?;
            for drop in [
                "DROP TABLE IF EXISTS exposures",
                "DROP TABLE IF EXISTS documents",
                "DROP TABLE IF EXISTS indexed_calls",
            ] {
                sqlx::query(drop).execute(&mut *tx).await?;
            }
            for statement in Self::SCHEMA {
                sqlx::query(statement).execute(&mut *tx).await?;
            }
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "PRAGMA user_version = {}",
                Self::VERSION
            )))
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }
        Ok(Self { pool })
    }

    /// Open an index a daemon built, read-only. Missing or of another
    /// version, it is [`StoreError::NotFound`].
    pub async fn open_read(dir: &Path) -> Result<Self, StoreError> {
        let path = dir.join(Self::FILE);
        if !path.is_file() {
            return Err(StoreError::NotFound);
        }
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .read_only(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await?;
        if version != Self::VERSION {
            return Err(StoreError::NotFound);
        }
        Ok(Self { pool })
    }

    /// The index file's disk use, measured through a short read-only
    /// connection that also reads the store's calls beside it. `None` when
    /// no index has been built.
    pub async fn usage(dir: &Path) -> Result<Option<IndexUsage>, StoreError> {
        let path = dir.join(Self::FILE);
        if !path.is_file() {
            return Ok(None);
        }
        let metadata = dir.join(Store::METADATA_FILE);
        let metadata = metadata.to_str().ok_or(StoreError::NonUtf8Path)?;
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .read_only(true)
            .busy_timeout(Duration::from_secs(5));
        let mut connection = SqliteConnection::connect_with(&options).await?;
        let file = FileUsage::measure(&mut connection, "main").await?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut connection)
            .await?;
        let (indexed_calls, stale_calls) = if version == Self::VERSION {
            sqlx::query("ATTACH DATABASE ? AS meta")
                .bind(metadata)
                .execute(&mut connection)
                .await?;
            let row = sqlx::query("SELECT COUNT(*) AS indexed, COALESCE(SUM(call NOT IN (SELECT id FROM meta.calls)), 0) AS stale FROM indexed_calls")
                .fetch_one(&mut connection)
                .await?;
            (
                row.try_get::<u64, _>("indexed")?,
                row.try_get::<u64, _>("stale")?,
            )
        } else {
            // About to be rebuilt: its rows describe nothing retained.
            (0, 0)
        };
        connection.close().await?;
        Ok(Some(IndexUsage {
            file,
            indexed_calls,
            stale_calls,
        }))
    }

    /// Add calls' documents, one transaction for all of them.
    pub async fn add(&self, calls: &[CallDocuments]) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        for call in calls {
            let id = call.call.to_string();
            sqlx::query("INSERT OR REPLACE INTO indexed_calls(call, session, turn, client, started_at, prompt, prompt_omitted) VALUES (?, ?, ?, ?, ?, ?, ?)")
                .bind(&id)
                .bind(call.session.map(|session| session.to_string()))
                .bind(call.turn.map(|turn| turn.to_string()))
                .bind(call.client.map(Labeled::label))
                .bind(i64::try_from(call.started_at.as_millis()).unwrap_or(i64::MAX))
                .bind(call.prompt.as_ref().map(|prompt| prompt.text.as_str()))
                .bind(call.prompt.as_ref().map(|prompt| i64::try_from(prompt.omitted_chars).unwrap_or(i64::MAX)))
                .execute(&mut *tx)
                .await?;
            for exposure in &call.exposures {
                sqlx::query("INSERT INTO exposures(call, field, kind, hint, fingerprint, count) VALUES (?, ?, ?, ?, ?, ?)")
                    .bind(&id)
                    .bind(exposure.field.label())
                    .bind(exposure.kind.label())
                    .bind(&exposure.hint)
                    .bind(&exposure.fingerprint)
                    .bind(i64::try_from(exposure.count).unwrap_or(i64::MAX))
                    .execute(&mut *tx)
                    .await?;
            }
            for document in &call.documents {
                sqlx::query("INSERT INTO documents(text, call, field) VALUES (?, ?, ?)")
                    .bind(&document.text)
                    .bind(&id)
                    .bind(document.field.label())
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// What the index knows of each of `turns`' prompts: when its earliest
    /// indexed call started, and the first prompt among its indexed calls.
    /// A turn none of whose calls is indexed is absent.
    pub async fn turn_prompts(
        &self,
        turns: &[TurnId],
    ) -> Result<HashMap<TurnId, IndexedPrompt>, StoreError> {
        let mut prompts: HashMap<TurnId, IndexedPrompt> = HashMap::new();
        if turns.is_empty() {
            return Ok(prompts);
        }
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT turn, started_at, prompt, prompt_omitted FROM indexed_calls WHERE turn IN (",
        );
        let mut list = sql.separated(", ");
        for turn in turns {
            list.push_bind(turn.to_string());
        }
        sql.push(") ORDER BY started_at, call");
        for row in sql.build().fetch_all(&self.pool).await? {
            let columns = Columns(&row);
            let Some(turn) = columns.optional_id::<TurnId>("turn")? else {
                continue;
            };
            let prompt = match columns.text("prompt")? {
                Some(text) => Some(Excerpt {
                    text,
                    omitted_chars: columns.count("prompt_omitted")?,
                }),
                None => None,
            };
            let indexed = prompts.entry(turn).or_insert(IndexedPrompt {
                earliest_call: columns.time("started_at")?,
                prompt: None,
            });
            if indexed.prompt.is_none() {
                indexed.prompt = prompt;
            }
        }
        Ok(prompts)
    }

    /// Which of `calls` are indexed.
    pub async fn indexed(&self, calls: &[CallId]) -> Result<HashSet<CallId>, StoreError> {
        if calls.is_empty() {
            return Ok(HashSet::new());
        }
        let mut sql = QueryBuilder::<Sqlite>::new("SELECT call FROM indexed_calls WHERE call IN (");
        let mut list = sql.separated(", ");
        for call in calls {
            list.push_bind(call.to_string());
        }
        sql.push(")");
        sql.build()
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(Self::id)
            .collect()
    }

    /// A page of indexed call IDs, in ID order after `after`.
    pub async fn indexed_page(
        &self,
        after: Option<CallId>,
        limit: i64,
    ) -> Result<Vec<CallId>, StoreError> {
        sqlx::query("SELECT call FROM indexed_calls WHERE call > ? ORDER BY call LIMIT ?")
            .bind(after.map(|call| call.to_string()).unwrap_or_default())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(Self::id)
            .collect()
    }

    /// Remove calls, which retention deleted, and their documents.
    pub async fn remove(&self, calls: &[CallId]) -> Result<(), StoreError> {
        if calls.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for table in ["exposures", "documents", "indexed_calls"] {
            let mut sql =
                QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table} WHERE call IN ("));
            let mut list = sql.separated(", ");
            for call in calls {
                list.push_bind(call.to_string());
            }
            sql.push(")");
            sql.build().execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Matching documents, newest call first.
    pub async fn search(&self, query: &SearchQuery) -> Result<SearchPage, StoreError> {
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT documents.call, documents.field, snippet(documents, 0, '«', '»', '…', 24) AS snippet, c.session, c.turn, c.client, c.started_at FROM documents JOIN indexed_calls c ON c.call = documents.call WHERE documents MATCH ",
        );
        sql.push_bind(query.terms.expression());
        if let Some(field) = query.field {
            sql.push(" AND documents.field = ").push_bind(field.label());
        }
        if let Some(session) = query.session {
            sql.push(" AND c.session = ").push_bind(session.to_string());
        }
        if let Some(client) = query.client {
            sql.push(" AND c.client = ").push_bind(client.label());
        }
        if let Some(from) = query.time.from() {
            sql.push(" AND c.started_at >= ")
                .push_bind(i64::try_from(from.as_millis()).unwrap_or(i64::MAX));
        }
        if let Some(until) = query.time.until() {
            sql.push(" AND c.started_at <= ")
                .push_bind(i64::try_from(until.as_millis()).unwrap_or(i64::MAX));
        }
        sql.push(" ORDER BY c.started_at DESC, documents.rowid DESC LIMIT ")
            .push_bind(i64::from(query.limit.get()));
        let rows = sql.build().fetch_all(&self.pool).await?;
        let mut hits = Vec::with_capacity(rows.len());
        for row in &rows {
            let columns = Columns(row);
            hits.push(SearchHit {
                call: columns.id("call")?,
                session: columns.optional_id("session")?,
                turn: columns.optional_id("turn")?,
                client: columns
                    .text("client")?
                    .map(|label| Stored::from_label(&label)),
                started_at: columns.time("started_at")?,
                field: columns.label("field")?,
                snippet: columns.text("snippet")?.unwrap_or_default(),
            });
        }
        Ok(SearchPage {
            hits,
            indexed_calls: self.indexed_count().await?,
        })
    }

    /// Secrets calls carried, newest call first.
    pub async fn exposures(&self, query: &ExposureQuery) -> Result<Vec<ExposureView>, StoreError> {
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT e.call, e.field, e.kind, e.hint, e.fingerprint, e.count, c.session, c.turn, c.client, c.started_at FROM exposures e JOIN indexed_calls c ON c.call = e.call WHERE 1 = 1",
        );
        if let Some(session) = query.session {
            sql.push(" AND c.session = ").push_bind(session.to_string());
        }
        if let Some(from) = query.time.from() {
            sql.push(" AND c.started_at >= ")
                .push_bind(i64::try_from(from.as_millis()).unwrap_or(i64::MAX));
        }
        if let Some(until) = query.time.until() {
            sql.push(" AND c.started_at <= ")
                .push_bind(i64::try_from(until.as_millis()).unwrap_or(i64::MAX));
        }
        sql.push(" ORDER BY c.started_at DESC, e.rowid LIMIT ")
            .push_bind(i64::from(query.limit.get()));
        sql.build()
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|row| {
                let columns = Columns(row);
                Ok(ExposureView {
                    call: columns.id("call")?,
                    session: columns.optional_id("session")?,
                    turn: columns.optional_id("turn")?,
                    client: columns
                        .text("client")?
                        .map(|label| Stored::from_label(&label)),
                    started_at: columns.time("started_at")?,
                    field: columns.label("field")?,
                    kind: columns.label("kind")?,
                    hint: columns.text("hint")?.unwrap_or_default(),
                    fingerprint: columns.text("fingerprint")?.unwrap_or_default(),
                    count: columns.count("count")?,
                })
            })
            .collect()
    }

    pub async fn indexed_count(&self) -> Result<u64, StoreError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM indexed_calls")
            .fetch_one(&self.pool)
            .await?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    fn id(row: &sqlx::sqlite::SqliteRow) -> Result<CallId, StoreError> {
        row.try_get::<String, _>("call")?
            .parse()
            .map_err(|_| StoreError::Corrupt("search call"))
    }
}

#[cfg(test)]
mod tests;
