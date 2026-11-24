//! Where each watched project's runs may reach, as its manifest on its
//! default branch on GitHub lists: read from the watch's snapshots, never
//! from a run's checkout, which the run could change.

use ren_github::{RepositorySlug, Snapshot};
use ren_manifest::{Manifest, ManifestError, ReachEntry};
use ren_relay::{Destination, NotADestination};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

/// Why a project's manifest gives its runs no reach.
#[derive(Debug, thiserror::Error)]
pub enum ProjectReachError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// An entry the manifest reads that the relay cannot admit by.
    #[error("reach in .ren/project.toml lists {0}, which the relay cannot admit")]
    Destination(ReachEntry, #[source] NotADestination),
}

/// The reach each watched repository's default branch declares.
#[derive(Debug, Clone, Default)]
pub struct ProjectReaches(Arc<Mutex<HashMap<RepositorySlug, Vec<Destination>>>>);

impl ProjectReaches {
    /// Learns what `snapshot` of `repository` declares: its manifest's
    /// reach, or none when it has no manifest. A manifest that does not
    /// read, or lists what the relay cannot admit by, declares none, and
    /// says why.
    pub fn learn(
        &self,
        repository: &RepositorySlug,
        snapshot: &Snapshot,
    ) -> Result<(), ProjectReachError> {
        let read = Self::reach(snapshot.manifest.as_deref());
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                repository.clone(),
                read.as_ref().cloned().unwrap_or_default(),
            );
        read.map(drop)
    }

    /// The destinations `manifest` lists, as the relay admits by them: the
    /// one place a manifest's entries become the relay's.
    fn reach(manifest: Option<&str>) -> Result<Vec<Destination>, ProjectReachError> {
        let Some(manifest) = manifest else {
            return Ok(Vec::new());
        };
        Manifest::read(manifest)?
            .reach()
            .iter()
            .map(|entry| {
                entry
                    .to_string()
                    .parse::<Destination>()
                    .map_err(|error| ProjectReachError::Destination(entry.clone(), error))
            })
            .collect()
    }

    /// Where `repository`'s runs may reach, as last learned.
    pub fn of(&self, repository: &RepositorySlug) -> Vec<Destination> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(repository)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::ProjectReaches;
    use ren_github::{Listing, Snapshot};

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
        let reaches = ProjectReaches::default();
        let web = "acme/web".parse().unwrap();
        assert!(reaches.of(&web).is_empty());
        reaches
            .learn(
                &web,
                &snapshot(Some("reach = [\"registry.npmjs.org:443\"]\n")),
            )
            .unwrap();
        assert_eq!(
            reaches
                .of(&web)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["registry.npmjs.org:443"]
        );
        // A manifest that does not read lists nothing, and says why.
        assert!(
            reaches
                .learn(&web, &snapshot(Some("reach = [\"npmjs.org\"]\n")))
                .is_err()
        );
        assert!(reaches.of(&web).is_empty());
        reaches
            .learn(
                &web,
                &snapshot(Some("reach = [\"registry.npmjs.org:443\"]\n")),
            )
            .unwrap();
        reaches.learn(&web, &snapshot(None)).unwrap();
        assert!(reaches.of(&web).is_empty());
        assert!(reaches.of(&"acme/api".parse().unwrap()).is_empty());
    }
}
