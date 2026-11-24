//! The projects watched on GitHub, as relayed runs need them: which one a
//! run works in, by its folder, and where that project's runs may reach,
//! as its manifest on its default branch on GitHub lists. Neither is read
//! from a run's checkout, which the run can change: its `origin`, its
//! manifest, and its `.git` are the run's to write.

use ren_github::{RepositorySlug, Snapshot};
use ren_manifest::{EgressEntry, Manifest, ManifestError};
use ren_relay::{Destination, NotADestination};
use ren_supervisor::Worktree;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

/// Why a project's manifest gives its runs no egress.
#[derive(Debug, thiserror::Error)]
pub enum ProjectEgressError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// An entry the manifest reads that the relay cannot admit by.
    #[error("egress in .ren/project.toml lists {0}, which the relay cannot admit")]
    Destination(EgressEntry, #[source] NotADestination),
}

/// The watched projects: each one's registered folder and GitHub
/// repository, and the reach its default branch declares.
#[derive(Debug, Clone)]
pub struct WatchedProjects {
    /// Where Ren makes worktrees, each named after the folder it is of.
    worktrees: Option<PathBuf>,
    known: Arc<Mutex<Known>>,
}

#[derive(Debug, Default)]
struct Known {
    folders: Vec<(PathBuf, RepositorySlug)>,
    egress: HashMap<RepositorySlug, Vec<Destination>>,
}

impl WatchedProjects {
    pub fn new(worktrees: Option<PathBuf>) -> Self {
        Self {
            worktrees,
            known: Arc::new(Mutex::new(Known::default())),
        }
    }

    /// Learns which folders the watched projects are registered in: each
    /// with its project's repository.
    pub fn learn_folders(&self, folders: impl IntoIterator<Item = (PathBuf, RepositorySlug)>) {
        self.known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .folders = folders.into_iter().collect();
    }

    /// The repository of the watched project a run in `workdir` works on:
    /// the one whose folder holds it, or whose folder a worktree Ren made
    /// is named after. None when no project or more than one is.
    pub fn repository_of(&self, workdir: &Path) -> Option<RepositorySlug> {
        let worktree = self
            .worktrees
            .as_deref()
            .and_then(|root| Worktree::of(root, workdir));
        let known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        let mut matching = known.folders.iter().filter(|(folder, _)| {
            workdir.starts_with(folder)
                || worktree
                    .as_ref()
                    .is_some_and(|worktree| worktree.is_of(folder))
        });
        match (matching.next(), matching.next()) {
            (Some((_, repository)), None) => Some(repository.clone()),
            (None, _) | (Some(_), Some(_)) => None,
        }
    }

    /// Learns what `snapshot` of `repository` declares: its manifest's
    /// reach, or none when it has no manifest. A manifest that does not
    /// read, or lists what the relay cannot admit by, declares none, and
    /// says why.
    pub fn learn_manifest(
        &self,
        repository: &RepositorySlug,
        snapshot: &Snapshot,
    ) -> Result<(), ProjectEgressError> {
        let read = Self::egress(snapshot.manifest.as_deref());
        self.known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .egress
            .insert(
                repository.clone(),
                read.as_ref().cloned().unwrap_or_default(),
            );
        read.map(drop)
    }

    /// The destinations `manifest` lists, as the relay admits by them: the
    /// one place a manifest's entries become the relay's.
    fn egress(manifest: Option<&str>) -> Result<Vec<Destination>, ProjectEgressError> {
        let Some(manifest) = manifest else {
            return Ok(Vec::new());
        };
        Manifest::read(manifest)?
            .egress()
            .iter()
            .map(|entry| {
                entry
                    .to_string()
                    .parse::<Destination>()
                    .map_err(|error| ProjectEgressError::Destination(entry.clone(), error))
            })
            .collect()
    }

    /// Where `repository`'s runs may reach, as last learned.
    pub fn egress_of(&self, repository: &RepositorySlug) -> Vec<Destination> {
        self.known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .egress
            .get(repository)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::WatchedProjects;
    use ren_github::{Listing, Snapshot};
    use std::path::{Path, PathBuf};

    fn snapshot(manifest: Option<&str>) -> Snapshot {
        Snapshot {
            default_branch: None,
            pull_requests: Vec::new(),
            listing: Listing::Complete,
            manifest: manifest.map(str::to_owned),
        }
    }

    #[test]
    fn a_project_reaches_what_its_default_branch_lists_and_nothing_once_it_stops() {
        let projects = WatchedProjects::new(None);
        let web = "acme/web".parse().unwrap();
        assert!(projects.egress_of(&web).is_empty());
        projects
            .learn_manifest(
                &web,
                &snapshot(Some("egress = [\"registry.npmjs.org:443\"]\n")),
            )
            .unwrap();
        assert_eq!(
            projects
                .egress_of(&web)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["registry.npmjs.org:443"]
        );
        // A manifest that does not read lists nothing, and says why.
        assert!(
            projects
                .learn_manifest(&web, &snapshot(Some("egress = [\"npmjs.org\"]\n")))
                .is_err()
        );
        assert!(projects.egress_of(&web).is_empty());
        projects
            .learn_manifest(
                &web,
                &snapshot(Some("egress = [\"registry.npmjs.org:443\"]\n")),
            )
            .unwrap();
        projects.learn_manifest(&web, &snapshot(None)).unwrap();
        assert!(projects.egress_of(&web).is_empty());
        assert!(projects.egress_of(&"acme/api".parse().unwrap()).is_empty());
    }

    #[test]
    fn a_run_works_on_the_project_its_folder_or_worktree_is_of_and_no_other() {
        let projects = WatchedProjects::new(Some(PathBuf::from("/data/ren/worktrees")));
        projects.learn_folders([
            (
                PathBuf::from("/home/op/dev/web"),
                "acme/web".parse().unwrap(),
            ),
            (
                PathBuf::from("/home/op/dev/api"),
                "acme/api".parse().unwrap(),
            ),
        ]);
        let of = |workdir: &str| {
            projects
                .repository_of(Path::new(workdir))
                .map(|repository| repository.to_string())
        };
        assert_eq!(of("/home/op/dev/web").as_deref(), Some("acme/web"));
        assert_eq!(of("/home/op/dev/web/ui").as_deref(), Some("acme/web"));
        assert_eq!(of("/data/ren/worktrees/api-3").as_deref(), Some("acme/api"));
        // Not a registered folder, nor a worktree Ren named after one.
        assert_eq!(of("/home/op/dev/webby"), None);
        assert_eq!(of("/home/op/scratch"), None);
        assert_eq!(of("/data/ren/worktrees/other-1"), None);
        // Two projects whose folders share a name: a worktree is of neither.
        projects.learn_folders([
            (
                PathBuf::from("/home/op/dev/api"),
                "acme/api".parse().unwrap(),
            ),
            (
                PathBuf::from("/home/op/old/api"),
                "acme/old-api".parse().unwrap(),
            ),
        ]);
        assert_eq!(of("/data/ren/worktrees/api-3"), None);
        assert_eq!(of("/home/op/old/api").as_deref(), Some("acme/old-api"));
    }
}
