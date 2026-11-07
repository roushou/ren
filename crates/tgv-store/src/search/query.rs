//! What a search or exposure listing asks for, and what it returns.

use super::SearchField;
use crate::query::{PageLimit, TimeRange};
use serde::Serialize;
use std::fmt;
use tgv_harness::Harness;
use tgv_protocol::label::Stored;
use tgv_recorder::secrets::SecretKind;
use tgv_recorder::{
    id::{CallId, SessionId, TurnId},
    time::UnixMillis,
};

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

/// Which exposures to list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExposureQuery {
    pub session: Option<SessionId>,
    pub time: TimeRange,
    pub limit: PageLimit,
}

/// A secret one call carried, as listed: never the secret itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExposureView {
    pub call: CallId,
    pub session: Option<SessionId>,
    pub turn: Option<TurnId>,
    pub client: Option<Stored<Harness>>,
    pub started_at: UnixMillis,
    pub field: Stored<SearchField>,
    pub kind: Stored<SecretKind>,
    pub hint: String,
    pub fingerprint: String,
    pub count: u64,
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
