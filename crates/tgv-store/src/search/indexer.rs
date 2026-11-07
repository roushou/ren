//! The index's only writer: bounded passes over finished calls.

use super::SearchIndex;
use crate::{Reader, StoreError};
use std::time::Duration;
use tgv_recorder::{id::CallId, time::UnixMillis};

/// What one indexing pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct PassReport {
    pub indexed: u64,
    pub removed: u64,
    pub backlog: Backlog,
}

impl PassReport {
    /// How long to wait before the next pass.
    pub fn pause(&self) -> Duration {
        match self.backlog {
            Backlog::Behind => Indexer::BUSY_PAUSE,
            Backlog::CaughtUp => Indexer::IDLE_PAUSE,
        }
    }
}

/// Whether finished calls wait beyond a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backlog {
    /// More wait: the next pass should come soon.
    Behind,
    /// The pass reached the newest finished call.
    CaughtUp,
}

/// Where the next pass reads finished calls from, in end-time and ID order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndexCursor {
    /// From the oldest finished call.
    Start,
    /// After this call: the last one a pass reached while behind.
    After(UnixMillis, CallId),
    /// From this end time on: caught up, so looking back over calls whose
    /// end was committed after a later one's.
    Since(UnixMillis),
}

impl IndexCursor {
    /// How far back a caught-up cursor looks again: capture commits a
    /// call's end shortly after its end time, so a later commit can sort
    /// earlier.
    const SLACK_MS: u64 = 60_000;

    /// Where the pass after one that reached `last` resumes.
    fn advance(self, last: Option<(UnixMillis, CallId)>, backlog: Backlog) -> Self {
        match (last, backlog) {
            (Some((ended, call)), Backlog::Behind) => Self::After(ended, call),
            (Some((ended, _)), Backlog::CaughtUp) => Self::Since(UnixMillis::from_millis(
                ended.as_millis().saturating_sub(Self::SLACK_MS),
            )),
            (None, Backlog::Behind | Backlog::CaughtUp) => self,
        }
    }
}

/// Fills the index from finished calls, oldest end first, and removes what
/// retention deleted. Each pass is bounded; [`PassReport::pause`] paces them.
pub struct Indexer {
    index: SearchIndex,
    reader: Reader,
    cursor: IndexCursor,
    /// Where the next retention sweep resumes.
    sweep_after: Option<CallId>,
}

impl Indexer {
    /// Calls read from the store per pass.
    const PAGE: usize = 100;
    /// Calls whose bodies are read and indexed per pass.
    const BATCH: usize = 20;
    /// Indexed IDs checked against the store per pass.
    const SWEEP: usize = 500;
    /// The pause between passes while backfilling.
    pub const BUSY_PAUSE: Duration = Duration::from_millis(200);
    /// The pause once caught up, and after a failed pass.
    pub const IDLE_PAUSE: Duration = Duration::from_secs(5);

    pub fn new(index: SearchIndex, reader: Reader) -> Self {
        Self {
            index,
            reader,
            cursor: IndexCursor::Start,
            sweep_after: None,
        }
    }

    pub async fn pass(&mut self) -> Result<PassReport, StoreError> {
        let page = self
            .reader
            .finished_calls(self.cursor, Self::limit(Self::PAGE))
            .await?;
        let ids: Vec<CallId> = page.iter().map(|(_, call)| *call).collect();
        let known = self.index.indexed(&ids).await?;
        let mut batch = Vec::new();
        let mut last = None;
        let mut backlog = if page.len() >= Self::PAGE {
            Backlog::Behind
        } else {
            Backlog::CaughtUp
        };
        for (ended, call) in &page {
            if batch.len() >= Self::BATCH {
                backlog = Backlog::Behind;
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
        self.cursor = self.cursor.advance(last, backlog);
        Ok(PassReport {
            indexed: Self::count(batch.len()),
            removed: self.sweep().await?,
            backlog,
        })
    }

    /// Remove indexed calls the store no longer has.
    async fn sweep(&mut self) -> Result<u64, StoreError> {
        let page = self
            .index
            .indexed_page(self.sweep_after, Self::limit(Self::SWEEP))
            .await?;
        self.sweep_after = if page.len() < Self::SWEEP {
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
        Ok(Self::count(gone.len()))
    }

    fn limit(rows: usize) -> i64 {
        i64::try_from(rows).unwrap_or(i64::MAX)
    }

    fn count(rows: usize) -> u64 {
        u64::try_from(rows).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn a_cursor_resumes_after_a_backlog_and_looks_back_once_caught_up() {
        let call = CallId::restore(Uuid::from_u128(7));
        let at = UnixMillis::from_millis(100_000);
        let last = Some((at, call));
        assert_eq!(
            IndexCursor::Start.advance(last, Backlog::Behind),
            IndexCursor::After(at, call)
        );
        assert_eq!(
            IndexCursor::Start.advance(last, Backlog::CaughtUp),
            IndexCursor::Since(UnixMillis::from_millis(40_000))
        );
        // Early history: the look-back stops at the epoch.
        assert_eq!(
            IndexCursor::Start.advance(Some((UnixMillis::from_millis(5), call)), Backlog::CaughtUp),
            IndexCursor::Since(UnixMillis::from_millis(0))
        );
        // A pass that read nothing stays where it was.
        let since = IndexCursor::Since(at);
        assert_eq!(since.advance(None, Backlog::CaughtUp), since);
        assert_eq!(
            IndexCursor::Start.advance(None, Backlog::Behind),
            IndexCursor::Start
        );
    }

    #[test]
    fn passes_are_paced_by_their_backlog() {
        let report = |backlog| PassReport {
            indexed: 0,
            removed: 0,
            backlog,
        };
        assert_eq!(report(Backlog::Behind).pause(), Indexer::BUSY_PAUSE);
        assert_eq!(report(Backlog::CaughtUp).pause(), Indexer::IDLE_PAUSE);
    }
}
