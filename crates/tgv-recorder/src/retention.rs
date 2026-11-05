//! Retention policy: what the store keeps.
//!
//! Limits are measured on retained data. The byte limit is real disk usage of
//! both files (pages in use, excluding free pages), not raw body bytes, because
//! deduplication means deleting a call frees only the chunks nothing else uses.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetentionPolicy {
    pub enabled: bool,
    pub max_age_seconds: u64,
    pub max_calls: u64,
    pub max_storage_bytes: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_age_seconds: 7 * 24 * 60 * 60,
            max_calls: 10_000,
            max_storage_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RetentionError {
    #[error("retention.max_age_seconds must be between 1 second and 10 years")]
    MaxAge,
    #[error("retention.max_calls must be between 1 and 10000000")]
    MaxCalls,
    #[error("retention.max_storage_bytes must be between 1 MiB and 16 TiB")]
    MaxStorage,
}

impl RetentionPolicy {
    pub fn validate(&self) -> Result<(), RetentionError> {
        if !(1..=3650 * 24 * 60 * 60).contains(&self.max_age_seconds) {
            return Err(RetentionError::MaxAge);
        }
        if !(1..=10_000_000).contains(&self.max_calls) {
            return Err(RetentionError::MaxCalls);
        }
        if !(1024 * 1024..=16 * 1024 * 1024 * 1024 * 1024).contains(&self.max_storage_bytes) {
            return Err(RetentionError::MaxStorage);
        }
        Ok(())
    }

    pub fn max_age_millis(&self) -> u64 {
        self.max_age_seconds.saturating_mul(1000)
    }
}
