//! Where a run may work: inside one of the configured roots. A requested path
//! is read lexically first, then resolved by the host (symlinks included) and
//! checked again, so a link cannot lead a run outside the roots. No I/O here:
//! resolving is the host's.

use std::path::{Component, Path, PathBuf};

/// The folders runs may work in, each already resolved by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots(Vec<PathBuf>);

/// A directory a run was asked to work in, before the host resolved it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedDir(PathBuf);

/// A resolved directory inside a root: where a run works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workdir(PathBuf);

/// Why a directory was refused. Paths are not echoed back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WorkdirError {
    #[error("the directory must be an absolute path")]
    Relative,
    #[error("the directory must not contain `..`")]
    Unnormalized,
    #[error("the directory is outside every configured root")]
    OutsideRoots,
    #[error("no roots are configured, so no run can start")]
    NoRoots,
    #[error("the directory's path is not valid UTF-8")]
    NotUtf8,
}

impl Roots {
    /// Roots as the host resolved them. Each must be absolute.
    pub fn new(resolved: Vec<PathBuf>) -> Result<Self, WorkdirError> {
        if resolved.is_empty() {
            return Err(WorkdirError::NoRoots);
        }
        if resolved.iter().any(|root| !root.is_absolute()) {
            return Err(WorkdirError::Relative);
        }
        Ok(Self(resolved))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Path> {
        self.0.iter().map(PathBuf::as_path)
    }
}

impl RequestedDir {
    pub fn parse(raw: &str) -> Result<Self, WorkdirError> {
        let path = PathBuf::from(raw);
        if !path.is_absolute() {
            return Err(WorkdirError::Relative);
        }
        let normal = path.components().all(|component| match component {
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => true,
            Component::CurDir | Component::ParentDir => false,
        });
        if !normal {
            return Err(WorkdirError::Unnormalized);
        }
        Ok(Self(path.components().collect()))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl Workdir {
    /// `resolved` is what the host made of a [`RequestedDir`]; it must lie
    /// inside a root, compared component by component.
    pub fn confirm(roots: &Roots, resolved: PathBuf) -> Result<Self, WorkdirError> {
        if !resolved.is_absolute() {
            return Err(WorkdirError::Relative);
        }
        if resolved.to_str().is_none() {
            return Err(WorkdirError::NotUtf8);
        }
        if roots.iter().any(|root| resolved.starts_with(root)) {
            Ok(Self(resolved))
        } else {
            Err(WorkdirError::OutsideRoots)
        }
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// The directory as text; [`Workdir::confirm`] made sure it is UTF-8.
    pub fn as_str(&self) -> &str {
        self.0.to_str().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{RequestedDir, Roots, Workdir, WorkdirError};
    use std::path::PathBuf;

    fn roots() -> Roots {
        Roots::new(vec![PathBuf::from("/home/op/dev")]).unwrap()
    }

    #[test]
    fn reads_requests_lexically() {
        assert!(RequestedDir::parse("/home/op/dev/acme").is_ok());
        assert_eq!(RequestedDir::parse("dev/acme"), Err(WorkdirError::Relative));
        assert_eq!(
            RequestedDir::parse("/home/op/dev/../.ssh"),
            Err(WorkdirError::Unnormalized)
        );
        // `.` changes nothing, and is dropped.
        assert_eq!(
            RequestedDir::parse("/home/op/./dev").unwrap().as_path(),
            std::path::Path::new("/home/op/dev")
        );
    }

    #[test]
    fn confirms_only_resolved_paths_inside_a_root() {
        let roots = roots();
        assert!(Workdir::confirm(&roots, "/home/op/dev".into()).is_ok());
        assert!(Workdir::confirm(&roots, "/home/op/dev/acme/api".into()).is_ok());
        for outside in ["/home/op/devious", "/home/op", "/etc"] {
            assert_eq!(
                Workdir::confirm(&roots, outside.into()),
                Err(WorkdirError::OutsideRoots),
                "{outside}"
            );
        }
    }

    #[test]
    fn needs_absolute_roots() {
        assert_eq!(Roots::new(vec![]), Err(WorkdirError::NoRoots));
        assert_eq!(
            Roots::new(vec![PathBuf::from("dev")]),
            Err(WorkdirError::Relative)
        );
    }
}
