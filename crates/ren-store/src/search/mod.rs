//! Full-text search over what calls show: the user's prompts, the model's
//! replies, and tool inputs and outputs, and the secrets calls carried.
//!
//! Both read what the recorder derived when each call finished: the FTS5
//! index `search` over `search_texts`, and `call_exposures`, in
//! `ren.sqlite3`. Retention removes a call's rows with it.

mod query;

pub use query::{
    ExposureQuery, ExposureView, SearchError, SearchHit, SearchPage, SearchQuery, SearchTerms,
};
pub use ren_recorder::digest::{Exposure, SearchField};

use crate::query::CallQuery;
use crate::view::Columns;
use crate::{Reader, StoreError};
use ren_harness::Harness;
use ren_harness::attribution::HintSource;
use ren_protocol::label::{Labeled, Stored};
use sqlx::{QueryBuilder, Sqlite};

impl Reader {
    /// Matching texts, newest call first.
    pub async fn search(&self, query: &SearchQuery) -> Result<SearchPage, StoreError> {
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT s.call, s.field, snippet(search, 0, '«', '»', '…', 24) AS snippet, c.session, c.turn, json_extract(c.attribution, '$.hint.source') AS source, c.started_at FROM search JOIN search_texts s ON s.id = search.rowid JOIN calls c ON c.id = s.call WHERE search MATCH ",
        );
        sql.push_bind(query.terms.expression());
        if let Some(field) = query.field {
            sql.push(" AND s.field = ").push_bind(field.label());
        }
        if let Some(session) = query.session {
            sql.push(" AND c.session = ").push_bind(session.to_string());
        }
        if let Some(client) = query.client {
            CallQuery::push_client(&mut sql, client);
        }
        query.time.push_started(&mut sql);
        sql.push(" ORDER BY c.started_at DESC, s.id DESC LIMIT ")
            .push_bind(i64::from(query.limit.get()));
        let rows = sql.build().fetch_all(&self.pool).await?;
        let mut hits = Vec::with_capacity(rows.len());
        for row in &rows {
            let columns = Columns(row);
            hits.push(SearchHit {
                call: columns.id("call")?,
                session: columns.optional_id("session")?,
                turn: columns.optional_id("turn")?,
                client: Self::client(&columns)?,
                started_at: columns.time("started_at")?,
                field: columns.label("field")?,
                snippet: columns.text("snippet")?.unwrap_or_default(),
            });
        }
        Ok(SearchPage {
            hits,
            indexed_calls: self.digested_count().await?,
        })
    }

    /// Secrets calls carried, newest call first.
    pub async fn exposures(&self, query: &ExposureQuery) -> Result<Vec<ExposureView>, StoreError> {
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT e.call, e.field, e.kind, e.hint, e.fingerprint, e.count, c.session, c.turn, json_extract(c.attribution, '$.hint.source') AS source, c.started_at FROM call_exposures e JOIN calls c ON c.id = e.call WHERE 1 = 1",
        );
        if let Some(session) = query.session {
            sql.push(" AND c.session = ").push_bind(session.to_string());
        }
        query.time.push_started(&mut sql);
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
                    client: Self::client(&columns)?,
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

    /// Calls whose digest has been derived: all finished calls, except while
    /// history recorded before digests is being derived.
    pub async fn digested_count(&self) -> Result<u64, StoreError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM call_digests")
            .fetch_one(&self.pool)
            .await?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// The harness of the convention that attributed a row's call.
    fn client(columns: &Columns<'_>) -> Result<Option<Stored<Harness>>, StoreError> {
        Ok(columns
            .text("source")?
            .map(|label| match HintSource::from_label(&label) {
                Some(source) => Stored::Known(source.harness()),
                None => Stored::from_label(&label),
            }))
    }
}

#[cfg(test)]
mod tests;
