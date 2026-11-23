//! What of the host a container sees.

use std::path::PathBuf;

/// What a container may do with a path it sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
}

/// A host path a container sees, at the same path, so every path reads the
/// same inside and out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub path: PathBuf,
    pub access: Access,
}

impl Mount {
    pub fn read_only(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            access: Access::ReadOnly,
        }
    }

    pub fn read_write(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            access: Access::ReadWrite,
        }
    }
}
