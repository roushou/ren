//! Versioned, credential-free Pi extension bundled with the Rust executable.
//!
//! The extension is published once per content hash into a private cache
//! directory and verified before every launch. The cache root comes from the
//! host environment (`XDG_CACHE_HOME`, then `HOME`), handed in as a value.

use crate::host::HostEnv;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// Why the extension could not be published or verified.
#[derive(Debug, thiserror::Error)]
pub enum PiCacheError {
    #[error("set an absolute XDG_CACHE_HOME or HOME for the Pi extension cache")]
    NoRoot,
    #[error("cannot create Pi extension cache")]
    Create(#[source] std::io::Error),
    #[error("cannot use the Pi extension cache")]
    Io(#[from] std::io::Error),
    #[error("cannot publish Pi extension cache file")]
    Publish,
    #[error("Pi extension cache must use regular files and directories, not symlinks")]
    NotRegular,
    #[error("Pi extension cache must not be writable by group or others")]
    Shared,
    #[error("Pi extension cache differs from the bundled source; remove the cache file and retry")]
    Modified,
}

/// How strictly a cache path is checked.
#[derive(Clone, Copy)]
enum CacheEntry {
    /// The user's cache root: symlinks are followed.
    Root,
    /// A TGV-owned directory: must not be a symlink.
    Directory,
    /// The published extension file: must not be a symlink.
    File,
}

/// The user's cache, where the bundled extension is published.
#[derive(Debug, Clone)]
pub struct PiCache {
    root: PathBuf,
}

impl PiCache {
    pub const SOURCE: &'static str = include_str!("../../../integrations/pi/tgv.mjs");
    /// Shown in dry runs instead of the cache path.
    pub const PLACEHOLDER: &'static str = "<TGV-managed Pi extension>";

    /// The cache root: `$XDG_CACHE_HOME`, else `~/.cache`.
    pub fn locate(env: &HostEnv) -> Result<Self, PiCacheError> {
        env.absolute_path("XDG_CACHE_HOME")
            .or_else(|| env.absolute_path("HOME").map(|home| home.join(".cache")))
            .map(|root| Self { root })
            .ok_or(PiCacheError::NoRoot)
    }

    /// Publish (or verify) the extension; its path.
    pub fn materialize(&self) -> Result<PathBuf, PiCacheError> {
        Self::materialize_under(&self.root)
    }

    /// The user's cache root may be a symlink (common with dotfile managers);
    /// only the directories TGV owns beneath it must be real, private
    /// directories.
    fn materialize_under(root: &Path) -> Result<PathBuf, PiCacheError> {
        Self::ensure_directory(root, CacheEntry::Root)?;
        let parent = root.join("tgv");
        Self::ensure_directory(&parent, CacheEntry::Directory)?;
        Self::materialize_in(&parent.join("pi"))
    }

    fn ensure_directory(directory: &Path, entry: CacheEntry) -> Result<(), PiCacheError> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory).map_err(PiCacheError::Create)?;
        Self::private_path(directory, entry)
    }

    fn materialize_in(directory: &Path) -> Result<PathBuf, PiCacheError> {
        Self::ensure_directory(directory, CacheEntry::Directory)?;
        let digest: String = Sha256::digest(Self::SOURCE.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        // .ts opts into Pi's loader, which resolves its bundled pi-ai module. A
        // native .mjs import would search this cache directory's node_modules.
        let path = directory.join(format!("tgv-{digest}.ts"));
        if !path.try_exists()? {
            let mut file = tempfile::NamedTempFile::new_in(directory)?;
            file.write_all(Self::SOURCE.as_bytes())?;
            file.as_file().sync_all()?;
            // Another launcher can publish the identical artifact concurrently.
            if let Err(error) = file.persist_noclobber(&path)
                && error.error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(PiCacheError::Publish);
            }
        }
        Self::private_path(&path, CacheEntry::File)?;
        let expected = u64::try_from(Self::SOURCE.len()).unwrap_or(u64::MAX);
        if std::fs::metadata(&path)?.len() != expected
            || std::fs::read(&path)? != Self::SOURCE.as_bytes()
        {
            return Err(PiCacheError::Modified);
        }
        Ok(path)
    }

    fn private_path(path: &Path, entry: CacheEntry) -> Result<(), PiCacheError> {
        let metadata = match entry {
            CacheEntry::Root => std::fs::metadata(path)?,
            CacheEntry::Directory | CacheEntry::File => std::fs::symlink_metadata(path)?,
        };
        let regular = match entry {
            CacheEntry::Root | CacheEntry::Directory => metadata.is_dir(),
            CacheEntry::File => metadata.is_file(),
        };
        if !regular {
            return Err(PiCacheError::NotRegular);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o022 != 0 {
                return Err(PiCacheError::Shared);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{HostEnv, PiCache, PiCacheError};

    #[test]
    fn caches_verified_code_without_overwriting_modified_files() {
        let root = tempfile::tempdir().unwrap();
        let path = PiCache::materialize_in(&root.path().join("pi")).unwrap();
        assert_eq!(
            PiCache::materialize_in(path.parent().unwrap()).unwrap(),
            path
        );
        std::fs::write(&path, "modified").unwrap();
        assert!(PiCache::materialize_in(path.parent().unwrap()).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "modified");
    }

    #[test]
    fn the_cache_root_comes_from_the_given_environment() {
        let temp = tempfile::tempdir().unwrap();
        // A temporary directory's mode follows the umask, and a group-writable
        // root is refused (`Shared`); make it private as a user's cache is.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let cache = temp.path().to_str().unwrap();
        let env = HostEnv::from_pairs([("XDG_CACHE_HOME", cache), ("HOME", "/nonexistent")]);
        let path = PiCache::locate(&env).unwrap().materialize().unwrap();
        assert!(path.starts_with(temp.path().join("tgv/pi")));
        let relative = HostEnv::from_pairs([("XDG_CACHE_HOME", "relative")]);
        assert!(matches!(
            PiCache::locate(&relative),
            Err(PiCacheError::NoRoot)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_or_shared_writable_code() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("pi");
        let path = PiCache::materialize_in(&dir).unwrap();
        let copy = root.path().join("copy.ts");
        std::fs::rename(&path, &copy).unwrap();
        symlink(&copy, &path).unwrap();
        assert!(PiCache::materialize_in(&dir).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::rename(&copy, &path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(PiCache::materialize_in(&dir).is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(PiCache::materialize_in(&dir).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn follows_a_symlinked_cache_root_but_not_tgv_directories() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        // Directories here are made private explicitly: their default mode
        // follows the umask, and a group-writable one is refused as `Shared`,
        // which would hide the refusal under test.
        let private = std::fs::Permissions::from_mode(0o700);
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real-cache");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, private.clone()).unwrap();
        let root = temp.path().join("cache");
        symlink(&real, &root).unwrap();
        let path = PiCache::materialize_under(&root).unwrap();
        assert!(
            real.join("tgv/pi")
                .join(path.file_name().unwrap())
                .is_file()
        );

        let other = tempfile::tempdir().unwrap();
        let linked_root = temp.path().join("linked-tgv");
        std::fs::create_dir(&linked_root).unwrap();
        std::fs::set_permissions(&linked_root, private).unwrap();
        symlink(other.path(), linked_root.join("tgv")).unwrap();
        assert!(matches!(
            PiCache::materialize_under(&linked_root),
            Err(PiCacheError::NotRegular)
        ));
    }
}
