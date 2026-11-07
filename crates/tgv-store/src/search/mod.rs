//! Full-text search over what calls show: the user's prompts, the model's
//! replies, and tool inputs and outputs.
//!
//! The index is derived data in its own file, `search.sqlite3`, beside the
//! store's two. It is never the source of truth: [`Indexer`] fills it from
//! stored bodies through the activity reader, backfills retained history on
//! first start, and removes what retention deleted. A missing file, or one of
//! another [`SearchIndex::VERSION`], is rebuilt. It has its own single
//! writer, so indexing never contends with capture, and it is not backed up.

use crate::activity::{Excerpt, ReadCall};
use crate::query::{PageLimit, TimeRange};
use crate::view::Columns;
use crate::{Reader, Store, StoreError};
use serde::Serialize;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use sqlx::{
    QueryBuilder, Row, Sqlite, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{collections::HashSet, fmt, path::Path, time::Duration};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use tgv_harness::Harness;
use tgv_protocol::label::{Labeled, Stored};
use tgv_recorder::{
    id::{CallId, SessionId, TurnId},
    time::UnixMillis,
};

/// Which part of a call a document holds.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum SearchField {
    /// The user's own words, without the context clients add.
    Prompt,
    /// The model's text output.
    Reply,
    /// Tool calls the model requested: name, targets, and input.
    ToolInput,
    /// Tool results the client sent back.
    ToolOutput,
}

/// One searchable text of a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub field: SearchField,
    pub text: String,
}

/// A call's documents, with what search filters on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallDocuments {
    pub call: CallId,
    pub session: Option<SessionId>,
    pub turn: Option<TurnId>,
    pub client: Option<Harness>,
    pub started_at: UnixMillis,
    pub documents: Vec<Document>,
}

impl ReadCall {
    /// The call's documents, each cut to `max_chars`.
    pub fn documents(&self, max_chars: usize) -> CallDocuments {
        let harness = self.harness();
        let mut documents = Vec::new();
        let mut push = |field, text: String| {
            if !text.trim().is_empty() {
                documents.push(Document {
                    field,
                    text: Excerpt::cut(&text, max_chars).text,
                });
            }
        };
        push(SearchField::Prompt, self.prompt().unwrap_or_default());
        push(SearchField::Reply, self.response.reply.clone());
        push(
            SearchField::ToolInput,
            self.response
                .tool_uses
                .iter()
                .map(|tool| {
                    let targets: Vec<String> = harness
                        .map(|harness| harness.effects(tool))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|effect| effect.target)
                        .collect();
                    let input = match &tool.input {
                        tgv_protocol::activity::ToolInput::Json(value) => value.to_string(),
                        tgv_protocol::activity::ToolInput::Text(text) => text.clone(),
                    };
                    format!("{} {} {}", tool.name, targets.join(" "), input)
                })
                .collect::<Vec<_>>()
                .join("\n"),
        );
        push(
            SearchField::ToolOutput,
            self.request
                .tool_results
                .iter()
                .map(|result| result.output.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        CallDocuments {
            call: self.view.id,
            session: self.view.session,
            turn: self.view.turn,
            client: harness,
            started_at: self.view.started_at,
            documents,
        }
    }
}

/// Why search text was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    #[error(
        "search text must have 1 to {} bytes and at least one word",
        SearchTerms::MAX_BYTES
    )]
    Terms,
}

/// What to look for: words, all of which must match, and "quoted phrases".
/// Every term becomes a quoted FTS5 string, so input can never inject query
/// syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchTerms {
    raw: String,
    terms: Vec<String>,
}

impl SearchTerms {
    pub const MAX_BYTES: usize = 256;

    pub fn parse(text: &str) -> Result<Self, SearchError> {
        if text.len() > Self::MAX_BYTES {
            return Err(SearchError::Terms);
        }
        let mut terms = Vec::new();
        for (index, part) in text.split('"').enumerate() {
            // Odd parts were inside quotes: phrases. Even ones are words.
            if index % 2 == 1 {
                if !part.trim().is_empty() {
                    terms.push(part.trim().to_owned());
                }
            } else {
                terms.extend(part.split_whitespace().map(str::to_owned));
            }
        }
        if terms.is_empty() {
            return Err(SearchError::Terms);
        }
        Ok(Self {
            raw: text.trim().to_owned(),
            terms,
        })
    }

    /// The FTS5 expression: each term a quoted string, all required.
    pub fn expression(&self) -> String {
        self.terms
            .iter()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ")
    }
}

/// The text as given, which parses back to the same terms.
impl fmt::Display for SearchTerms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// One search, newest matches first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    pub terms: SearchTerms,
    pub session: Option<SessionId>,
    pub client: Option<Harness>,
    pub field: Option<SearchField>,
    pub time: TimeRange,
    pub limit: PageLimit,
}

/// One matching document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    pub call: CallId,
    pub session: Option<SessionId>,
    pub turn: Option<TurnId>,
    pub client: Option<Stored<Harness>>,
    pub started_at: UnixMillis,
    pub field: Stored<SearchField>,
    /// The matching text around the match, terms marked «like this».
    pub snippet: String,
}

/// Search results, and how much history the index covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    /// Calls indexed so far; fewer than retained while backfilling.
    pub indexed_calls: u64,
}

/// The search index: a pool on `search.sqlite3`. Clones share it.
#[derive(Clone)]
pub struct SearchIndex {
    pool: SqlitePool,
}

impl SearchIndex {
    pub const FILE: &str = "search.sqlite3";
    /// The index format. Another version on disk is dropped and rebuilt.
    pub const VERSION: i64 = 1;
    /// The most characters of one field of one call that are indexed.
    pub const FIELD_CHARS: usize = 32 * 1024;
    const SCHEMA: [&str; 3] = [
        "CREATE TABLE IF NOT EXISTS indexed_calls (call TEXT PRIMARY KEY, session TEXT, turn TEXT, client TEXT, started_at INTEGER NOT NULL) WITHOUT ROWID",
        "CREATE INDEX IF NOT EXISTS indexed_calls_started ON indexed_calls(started_at)",
        "CREATE VIRTUAL TABLE IF NOT EXISTS documents USING fts5(text, call UNINDEXED, field UNINDEXED, tokenize = 'unicode61 remove_diacritics 2')",
    ];

    /// Open the index for writing and reading, creating or rebuilding it.
    pub async fn open(dir: &Path) -> Result<Self, StoreError> {
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
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await?;
        if version != Self::VERSION {
            let mut tx = pool.begin().await?;
            for drop in [
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

    /// Add calls' documents, one transaction for all of them.
    pub async fn add(&self, calls: &[CallDocuments]) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        for call in calls {
            let id = call.call.to_string();
            sqlx::query("INSERT OR REPLACE INTO indexed_calls(call, session, turn, client, started_at) VALUES (?, ?, ?, ?, ?)")
                .bind(&id)
                .bind(call.session.map(|session| session.to_string()))
                .bind(call.turn.map(|turn| turn.to_string()))
                .bind(call.client.map(Labeled::label))
                .bind(i64::try_from(call.started_at.as_millis()).unwrap_or(i64::MAX))
                .execute(&mut *tx)
                .await?;
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
        for table in ["documents", "indexed_calls"] {
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

/// What one indexing pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub struct PassReport {
    pub indexed: u64,
    pub removed: u64,
    /// More finished calls wait: run the next pass soon.
    pub more: bool,
}

/// Fills the index from finished calls, oldest end first, and removes what
/// retention deleted. Each pass is bounded; the caller paces them.
pub struct Indexer {
    index: SearchIndex,
    reader: Reader,
    /// The last finished call indexed or skipped, by end time and ID.
    cursor: Option<(UnixMillis, CallId)>,
    /// Where the next retention sweep resumes.
    sweep_after: Option<CallId>,
}

impl Indexer {
    /// Calls read from the store per pass.
    const PAGE: i64 = 100;
    /// Calls whose bodies are read and indexed per pass.
    const BATCH: usize = 20;
    /// Indexed IDs checked against the store per pass.
    const SWEEP: i64 = 500;
    /// How far back a caught-up cursor looks again: capture commits a call's
    /// end shortly after its end time, so a later commit can sort earlier.
    const SLACK_MS: u64 = 60_000;

    pub fn new(index: SearchIndex, reader: Reader) -> Self {
        Self {
            index,
            reader,
            cursor: None,
            sweep_after: None,
        }
    }

    pub async fn pass(&mut self) -> Result<PassReport, StoreError> {
        let mut report = PassReport::default();
        let page = self.reader.finished_calls(self.cursor, Self::PAGE).await?;
        let ids: Vec<CallId> = page.iter().map(|(_, call)| *call).collect();
        let known = self.index.indexed(&ids).await?;
        let mut batch = Vec::new();
        let mut last = None;
        for (ended, call) in &page {
            if batch.len() >= Self::BATCH {
                report.more = true;
                break;
            }
            last = Some((*ended, *call));
            if known.contains(call) {
                continue;
            }
            match self
                .reader
                .read_documents(*call, SearchIndex::FIELD_CHARS)
                .await
            {
                Ok(documents) => batch.push(documents),
                // Pruned between the listing and the read.
                Err(StoreError::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        self.index.add(&batch).await?;
        report.indexed = u64::try_from(batch.len()).unwrap_or(u64::MAX);
        let full = i64::try_from(page.len()).unwrap_or(i64::MAX) >= Self::PAGE;
        report.more |= full;
        self.cursor = match (last, report.more) {
            (Some(last), true) => Some(last),
            // Caught up: look back a little next time, and dedupe by ID.
            (Some((ended, _)), false) => Some((
                UnixMillis::from_millis(ended.as_millis().saturating_sub(Self::SLACK_MS)),
                CallId::restore(uuid::Uuid::nil()),
            )),
            (None, _) => self.cursor,
        };
        report.removed = self.sweep().await?;
        Ok(report)
    }

    /// Remove indexed calls the store no longer has.
    async fn sweep(&mut self) -> Result<u64, StoreError> {
        let page = self
            .index
            .indexed_page(self.sweep_after, Self::SWEEP)
            .await?;
        self.sweep_after = if i64::try_from(page.len()).unwrap_or(i64::MAX) < Self::SWEEP {
            None
        } else {
            page.last().copied()
        };
        let existing = self.reader.existing_calls(&page).await?;
        let gone: Vec<CallId> = page
            .into_iter()
            .filter(|call| !existing.contains(call))
            .collect();
        self.index.remove(&gone).await?;
        Ok(u64::try_from(gone.len()).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests;
