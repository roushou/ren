//! The calls the dev instance's traffic and fake agents send, as pure data
//! ([`scenario`]), and the count that keeps each run's sessions apart. An
//! instance's history is seeded by `ren seed`, not by these.

mod scenario;

pub use scenario::{Call, Run};
pub(in crate::dev) use scenario::{Caller, Client, PROMPTS, Side};

use std::{fs, path::Path};

/// Why the run count could not be kept.
#[derive(Debug, thiserror::Error)]
#[error("cannot keep the run count in {path}: {source}")]
pub struct RunCountError {
    path: String,
    source: std::io::Error,
}

impl Run {
    /// The file counting runs, in the instance's data directory.
    const COUNT: &str = "seed-runs";

    /// The next run, counted in `data_dir`, which a reset removes.
    pub fn next(data_dir: &Path) -> Result<Self, RunCountError> {
        let path = data_dir.join(Self::COUNT);
        let previous: u32 = fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0);
        let next = previous.saturating_add(1);
        fs::write(&path, next.to_string()).map_err(|source| RunCountError {
            path: path.display().to_string(),
            source,
        })?;
        Ok(Self(next))
    }
}
