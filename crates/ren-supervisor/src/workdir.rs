//! Where a run may work: inside a configured root (where projects are found),
//! inside another configured place (`~/.config`, a dotfiles folder), or in
//! the scratch folder Ren owns. A requested path is read lexically first,
//! then resolved by the host (symlinks included) and checked again, so a link
//! cannot lead a run outside them. No I/O here: resolving is the host's.

use std::path::{Component, Path, PathBuf};

/// The folders runs may work in, each already resolved by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots(Vec<PathBuf>);

/// Everywhere a run may work, each folder already resolved by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bounds {
    roots: Roots,
    /// Other places, found apart from projects and shown only when asked.
    others: Vec<PathBuf>,
    /// The folder scratch runs get their own folder in; none when it could
    /// not be set up.
    scratch: Option<PathBuf>,
    /// The operator's home, for showing paths as `~/…`.
    home: Option<PathBuf>,
}

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
    #[error("the directory is outside every configured root and place")]
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

impl Bounds {
    /// Each folder must be absolute, as the host resolved it.
    pub fn new(
        roots: Roots,
        others: Vec<PathBuf>,
        scratch: Option<PathBuf>,
        home: Option<PathBuf>,
    ) -> Result<Self, WorkdirError> {
        if others
            .iter()
            .chain(&scratch)
            .any(|path| !path.is_absolute())
        {
            return Err(WorkdirError::Relative);
        }
        Ok(Self {
            roots,
            others,
            scratch,
            home,
        })
    }

    pub fn roots(&self) -> &Roots {
        &self.roots
    }

    pub fn others(&self) -> &[PathBuf] {
        &self.others
    }

    pub fn scratch(&self) -> Option<&Path> {
        self.scratch.as_deref()
    }

    /// `path` as the operator reads it: under their home as `~/…`.
    pub fn shown(&self, path: &Path) -> String {
        match self
            .home
            .as_deref()
            .and_then(|home| path.strip_prefix(home).ok())
        {
            Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
            Some(rest) => format!("~/{}", rest.display()),
            None => path.display().to_string(),
        }
    }

    /// Whether `path` lies inside a root, another place, or the scratch
    /// folder, compared component by component.
    fn holds(&self, path: &Path) -> bool {
        self.roots
            .iter()
            .chain(self.others.iter().map(PathBuf::as_path))
            .chain(self.scratch.as_deref())
            .any(|bound| path.starts_with(bound))
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
    /// inside a root, another place, or the scratch folder.
    pub fn confirm(bounds: &Bounds, resolved: PathBuf) -> Result<Self, WorkdirError> {
        if !resolved.is_absolute() {
            return Err(WorkdirError::Relative);
        }
        if resolved.to_str().is_none() {
            return Err(WorkdirError::NotUtf8);
        }
        if bounds.holds(&resolved) {
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
    use super::{Bounds, RequestedDir, Roots, Workdir, WorkdirError};
    use std::path::{Path, PathBuf};

    fn roots() -> Bounds {
        Bounds::new(
            Roots::new(vec![PathBuf::from("/home/op/dev")]).unwrap(),
            vec![PathBuf::from("/home/op/.config")],
            Some(PathBuf::from("/home/op/.local/share/ren/scratch")),
            Some(PathBuf::from("/home/op")),
        )
        .unwrap()
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
        // Another place, and a scratch run's own folder.
        assert!(Workdir::confirm(&roots, "/home/op/.config/nvim".into()).is_ok());
        assert!(
            Workdir::confirm(&roots, "/home/op/.local/share/ren/scratch/scratch-1".into()).is_ok()
        );
        for outside in ["/home/op/devious", "/home/op", "/home/op/.ssh", "/etc"] {
            assert_eq!(
                Workdir::confirm(&roots, outside.into()),
                Err(WorkdirError::OutsideRoots),
                "{outside}"
            );
        }
    }

    #[test]
    fn shows_paths_under_home_with_a_tilde() {
        let bounds = roots();
        assert_eq!(bounds.shown(Path::new("/home/op/dev/ren")), "~/dev/ren");
        assert_eq!(bounds.shown(Path::new("/home/op")), "~");
        assert_eq!(bounds.shown(Path::new("/srv/ren")), "/srv/ren");
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
