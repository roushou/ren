//! Files xtask writes on the host, and the errors that name them.
use std::{
    fs,
    io::Write as _,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

/// An I/O failure, with the path it concerns.
#[derive(Debug, thiserror::Error)]
#[error("{}: {source}", path.display())]
pub struct FileError {
    path: PathBuf,
    #[source]
    source: std::io::Error,
}

impl FileError {
    /// Wraps an I/O error with the path it concerns.
    pub fn at(path: &Path) -> impl FnOnce(std::io::Error) -> Self + '_ {
        move |source| Self {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// Who may read a written file, and whether it runs.
#[derive(Debug, Clone, Copy)]
pub enum FileMode {
    /// Only its owner reads it (`0600`): configuration, saved flags.
    Private,
    /// Anyone reads it (`0644`): a systemd unit.
    Readable,
    /// Anyone runs it (`0755`): a binary or script.
    Executable,
}

impl FileMode {
    fn bits(self) -> u32 {
        match self {
            Self::Private => 0o600,
            Self::Readable => 0o644,
            Self::Executable => 0o755,
        }
    }

    /// Publishes `contents` at `path` atomically, with this mode.
    pub fn write(self, path: &Path, contents: &[u8]) -> Result<(), FileError> {
        let dir = path.parent().unwrap_or(Path::new("/"));
        let mut file = tempfile::Builder::new()
            .prefix(".ren.")
            .tempfile_in(dir)
            .map_err(FileError::at(dir))?;
        file.write_all(contents).map_err(FileError::at(path))?;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(self.bits()))
            .map_err(FileError::at(path))?;
        file.as_file().sync_all().map_err(FileError::at(path))?;
        file.persist(path)
            .map_err(|error| FileError::at(path)(error.error))?;
        Ok(())
    }
}
