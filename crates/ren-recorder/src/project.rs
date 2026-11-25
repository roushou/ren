//! Projects, pure: the repositories Ren builds, each registered by the
//! operator under a name (docs/plans/hierarchy.md). A project is a stored
//! record, never inferred from a folder a session happens to open: its
//! [`ProjectName`] names it in the CLI and in URLs, its [`RepositoryPath`]
//! is the repository it is, and its [`Factory`] says how it is built and
//! released.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt, path::Path, str::FromStr};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// A project's name: 1 to 64 lowercase ASCII letters, digits, and inner
/// hyphens (`acme-web`), so it reads the same in a URL, a folder, and a
/// command line.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr,
)]
pub struct ProjectName(String);

/// A repository's folder: an absolute path, as text, with no `.` or `..`
/// part and no trailing separator. The shell canonicalizes it before it is
/// registered, so one repository is one path.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr,
)]
pub struct RepositoryPath(String);

/// How a project is built and released: a closed set.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Factory {
    /// Ren itself: its canary checks a build against a replica of
    /// production's data (`ren deployer canary`), and `install-vps`
    /// promotes it. Its release is kept in `<data>/release.json`.
    Ren,
    /// A repository that declares its contract in `.ren/project.toml`:
    /// `ren project canary` checks it and its deploy step promotes it. Its
    /// release is kept in `<data>/projects/<name>`.
    Contract,
}

/// What registering a project names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProjectRegistration {
    pub name: ProjectName,
    pub repository: RepositoryPath,
    pub factory: Factory,
}

/// Why text is not a [`ProjectName`] or a [`RepositoryPath`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectTextError {
    #[error("a project name is 1 to 64 lowercase letters, digits, and inner hyphens: {0:?}")]
    Name(String),
    #[error("a repository is an absolute path with no `.` or `..` part, as text: {0:?}")]
    Repository(String),
}

impl ProjectName {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name a folder suggests, if its name is one.
    pub fn of_folder(folder: &Path) -> Option<Self> {
        folder.file_name()?.to_str()?.parse().ok()
    }
}

impl FromStr for ProjectName {
    type Err = ProjectTextError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        let valid = (1..=64).contains(&name.len())
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && !name.starts_with('-')
            && !name.ends_with('-');
        if valid {
            Ok(Self(name.to_owned()))
        } else {
            Err(ProjectTextError::Name(name.to_owned()))
        }
    }
}

impl fmt::Display for ProjectName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl RepositoryPath {
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    /// Whether `folder` is this repository or in it, by its text: the
    /// shell resolves a folder before it asks, as it does a repository.
    /// `/dev/acme-web2` is not in `/dev/acme-web`.
    pub fn holds(&self, folder: &str) -> bool {
        folder == self.0
            || folder
                .strip_prefix(self.0.as_str())
                .is_some_and(|rest| rest.starts_with('/') || self.0 == "/")
    }

    /// The repository at `path`, which must already be canonical.
    pub fn of(path: &Path) -> Result<Self, ProjectTextError> {
        let text = path
            .to_str()
            .ok_or_else(|| ProjectTextError::Repository(path.display().to_string()))?;
        text.parse()
    }
}

impl FromStr for RepositoryPath {
    type Err = ProjectTextError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        // By its text: `Path::components` drops an inner `.`.
        let valid = text == "/"
            || text.strip_prefix('/').is_some_and(|rest| {
                rest.split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..")
            });
        if valid {
            Ok(Self(text.to_owned()))
        } else {
            Err(ProjectTextError::Repository(text.to_owned()))
        }
    }
}

impl fmt::Display for RepositoryPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{ProjectName, RepositoryPath};
    use std::path::Path;

    #[test]
    fn names_are_slugs() {
        for name in ["acme-web", "ren", "a1", "x"] {
            assert_eq!(name.parse::<ProjectName>().unwrap().as_str(), name);
        }
        for name in [
            "",
            "-a",
            "a-",
            "Acme",
            "acme web",
            "acme_web",
            &"a".repeat(65),
        ] {
            assert!(name.parse::<ProjectName>().is_err(), "{name:?}");
        }
        assert_eq!(
            ProjectName::of_folder(Path::new("/home/op/dev/acme-web")),
            Some("acme-web".parse().unwrap())
        );
        assert_eq!(ProjectName::of_folder(Path::new("/home/op/Acme")), None);
    }

    #[test]
    fn repositories_are_plain_absolute_paths() {
        for path in ["/home/op/dev/acme-web", "/srv/ren"] {
            assert_eq!(path.parse::<RepositoryPath>().unwrap().to_string(), path);
        }
        for path in [
            "dev/acme",
            "/home/op/../acme",
            "/home/./acme",
            "/home/acme/",
            "",
        ] {
            assert!(path.parse::<RepositoryPath>().is_err(), "{path:?}");
        }
    }

    #[test]
    fn a_repository_holds_itself_and_what_is_in_it() {
        let repository: RepositoryPath = "/home/op/dev/acme-web".parse().unwrap();
        for folder in ["/home/op/dev/acme-web", "/home/op/dev/acme-web/src/ui"] {
            assert!(repository.holds(folder), "{folder}");
        }
        for folder in [
            "/home/op/dev/acme-web2",
            "/home/op/dev",
            "/home/op/dev/acme-api",
        ] {
            assert!(!repository.holds(folder), "{folder}");
        }
    }
}
