//! Where each watched project's runs may reach, as its manifest on its
//! default branch on GitHub lists: read from the watch's snapshots, never
//! from a run's checkout, which the run could change.

use ren_github::{RepositorySlug, Snapshot};
use ren_manifest::{Manifest, ManifestError};
use ren_relay::Destination;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

/// The reach each watched repository's default branch declares.
#[derive(Debug, Clone, Default)]
pub struct ProjectReaches(Arc<Mutex<HashMap<RepositorySlug, Vec<Destination>>>>);

impl ProjectReaches {
    /// Learns what `snapshot` of `repository` declares: its manifest's
    /// reach, or none when it has no manifest. A manifest that does not
    /// read declares none, and its error is returned.
    pub fn learn(
        &self,
        repository: &RepositorySlug,
        snapshot: &Snapshot,
    ) -> Result<(), ManifestError> {
        let read = snapshot.manifest.as_deref().map(Manifest::read).transpose();
        let reach = match &read {
            Ok(manifest) => manifest
                .as_ref()
                .map(|manifest| manifest.reach().to_vec())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(repository.clone(), reach);
        read.map(drop)
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
