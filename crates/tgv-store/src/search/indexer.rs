//! The index's only writer: bounded passes over finished calls.

use super::SearchIndex;
use crate::secrets::{ScannerError, SecretScanner};
use crate::{Reader, StoreError};
use tgv_recorder::{id::CallId, time::UnixMillis};

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
    scanner: SecretScanner,
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

    pub fn new(index: SearchIndex, reader: Reader) -> Result<Self, ScannerError> {
        Ok(Self {
            index,
            reader,
            scanner: SecretScanner::new()?,
            cursor: None,
            sweep_after: None,
        })
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
                .read_documents(*call, SearchIndex::FIELD_CHARS, &self.scanner)
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
