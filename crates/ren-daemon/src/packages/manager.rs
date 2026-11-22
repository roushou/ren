//! The daemon's [`Packages`]: the control API's packages, over this host's
//! package store, the configured catalog, and its search path. The CLI uses it too,
//! so `ren packages` and `/control/v1/packages` answer alike.

use super::HostPrograms;
use super::{Changed, PackageStore, PackageStoreError, fetch::Fetch};
use futures_util::{FutureExt as _, future::BoxFuture};
use ren_depot::{
    Depot, Surveyed,
    job::{Failure, JobState, Stage},
    package_set::{Change, Edit, Generation, PackageSet},
    provision::Provision,
    source::{Platform, Source},
    version::ObservedVersion,
};
use ren_gateway::packages::{
    ChangeKind, ChangeOutcome, GenerationInstall, GenerationState, GenerationView, InstallFailure,
    InstalledPackage, ListedPackage, PackageChange, PackageCollection, PackageListing,
    PackageSetChange, Packages, PackagesError, SystemProgram,
};
use ren_harness::Harness;
use ren_protocol::label::Labeled as _;
use ren_store::Reader;
use std::sync::Arc;

/// The packages of this host.
pub struct HostPackages<F> {
    inner: Arc<Inner<F>>,
}

struct Inner<F> {
    store: PackageStore<F>,
    depot: Depot,
    host: HostPrograms,
    platform: Option<Platform>,
    /// Which generations runs use; without one, none are known to.
    reader: Option<Reader>,
}

impl<F> Clone for HostPackages<F> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<F: Fetch + 'static> HostPackages<F> {
    pub fn new(
        store: PackageStore<F>,
        depot: Depot,
        host: HostPrograms,
        platform: Option<Platform>,
        reader: Option<Reader>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                depot,
                host,
                platform,
                reader,
            }),
        }
    }

    /// The listing; reading versions runs each program found, so it runs
    /// on a blocking thread.
    pub async fn read_listing(&self) -> Result<PackageListing, PackagesError> {
        let inner = self.inner.clone();
        let live = self.live().await?;
        tokio::task::spawn_blocking(move || inner.listing(&live))
            .await
            .map_err(|error| PackagesError::Failed(error.to_string()))?
    }

    pub async fn apply(&self, edit: Edit) -> Result<PackageSetChange, PackagesError> {
        let platform = self.inner.platform.ok_or(PackagesError::Unsupported)?;
        let changed = self
            .inner
            .store
            .change(&self.inner.depot, platform, &edit)
            .await
            .map_err(Inner::<F>::failed)?;
        Ok(Inner::<F>::change_view(changed))
    }

    pub async fn go_back(&self, generation: Generation) -> Result<PackageSetChange, PackagesError> {
        let inner = self.inner.clone();
        let changed = tokio::task::spawn_blocking(move || inner.store.roll_back(generation))
            .await
            .map_err(|error| PackagesError::Failed(error.to_string()))?
            .map_err(|error| match error {
                PackageStoreError::NotInstalled(_) => PackagesError::Collected(generation),
                other @ (PackageStoreError::Io { .. }
                | PackageStoreError::Corrupt(_)
                | PackageStoreError::Plan(_)
                | PackageStoreError::Unsupported
                | PackageStoreError::Busy
                | PackageStoreError::NoGeneration(_)) => Inner::<F>::failed(other),
            })?;
        Ok(Inner::<F>::change_view(changed))
    }

    pub async fn sweep(&self) -> Result<PackageCollection, PackagesError> {
        let live = self.live().await?;
        let inner = self.inner.clone();
        // Collecting waits on the store's lock and removes folders.
        let collected = tokio::task::spawn_blocking(move || inner.store.collect(&live))
            .await
            .map_err(|error| PackagesError::Failed(error.to_string()))?
            .map_err(Inner::<F>::failed)?;
        Ok(PackageCollection {
            generations: collected.generations,
            installs: collected.installs.iter().map(ToString::to_string).collect(),
        })
    }

    async fn live(&self) -> Result<Vec<Generation>, PackagesError> {
        match &self.inner.reader {
            None => Ok(Vec::new()),
            Some(reader) => reader
                .live_generations()
                .await
                .map_err(|error| PackagesError::Failed(error.to_string())),
        }
    }
}

impl<F> Inner<F> {
    fn failed(error: PackageStoreError) -> PackagesError {
        match error {
            PackageStoreError::Busy => PackagesError::Busy,
            PackageStoreError::Plan(error) => PackagesError::Refused(error),
            PackageStoreError::Unsupported => PackagesError::Unsupported,
            PackageStoreError::NoGeneration(generation) => PackagesError::NoGeneration(generation),
            error @ (PackageStoreError::Io { .. }
            | PackageStoreError::Corrupt(_)
            | PackageStoreError::NotInstalled(_)) => PackagesError::Failed(error.to_string()),
        }
    }

    fn listing(&self, live: &[Generation]) -> Result<PackageListing, PackagesError> {
        let current = self.store.current().map_err(Self::failed)?;
        let survey = self.depot.survey(&self.host);
        let catalogued = survey
            .into_iter()
            .map(|Surveyed { package, provision }| ListedPackage {
                id: package.id().clone(),
                origin: Some(package.origin()),
                kind: Some(package.kind()),
                program: package.program().clone(),
                release: match package.source() {
                    Source::System => None,
                    Source::Release(release) => Some(release.version.clone()),
                },
                installed: current
                    .install(package.id())
                    .map(|install| InstalledPackage {
                        version: install.version.clone(),
                        sha256: install.sha256.to_string(),
                        dir: install.dir().to_string(),
                    }),
                system: match provision {
                    Provision::Missing => None,
                    Provision::System(found) => Some(match found.version {
                        ObservedVersion::Read(version) => SystemProgram {
                            path: found.path.display().to_string(),
                            version: Some(version),
                            unread: None,
                        },
                        ObservedVersion::Unparsed(raw) => SystemProgram {
                            path: found.path.display().to_string(),
                            version: None,
                            unread: Some(raw.to_string()),
                        },
                        ObservedVersion::Unread(failure) => SystemProgram {
                            path: found.path.display().to_string(),
                            version: None,
                            unread: Some(failure.to_string()),
                        },
                    }),
                },
                needed_by: Harness::ALL
                    .iter()
                    .copied()
                    .filter(|harness| {
                        harness
                            .requirements()
                            .iter()
                            .any(|requirement| requirement.package() == package.id())
                    })
                    .collect(),
            });
        // Installs whose package was taken out of the catalog, until they
        // are removed.
        let uncatalogued = current
            .installs()
            .filter(|install| self.depot.package(&install.package).is_none())
            .map(|install| ListedPackage {
                id: install.package.clone(),
                origin: None,
                kind: None,
                program: install.program.clone(),
                release: None,
                installed: Some(InstalledPackage {
                    version: install.version.clone(),
                    sha256: install.sha256.to_string(),
                    dir: install.dir().to_string(),
                }),
                system: None,
                needed_by: Vec::new(),
            });
        let packages = catalogued.chain(uncatalogued).collect();
        let generations = self
            .store
            .generations()
            .map_err(Self::failed)?
            .iter()
            .map(|set| Self::generation_view(set, &current, live))
            .collect();
        Ok(PackageListing {
            platform: self.platform,
            generation: current.generation(),
            bin: (current.generation() != Generation::NONE)
                .then(|| self.store.bin(current.generation()).display().to_string()),
            packages,
            generations,
        })
    }

    fn generation_view(
        set: &PackageSet,
        current: &PackageSet,
        live: &[Generation],
    ) -> GenerationView {
        let state = if set.generation() == current.generation() {
            GenerationState::Current
        } else if live.contains(&set.generation()) {
            GenerationState::InUse
        } else {
            GenerationState::Idle
        };
        GenerationView {
            generation: set.generation(),
            state,
            installs: set
                .installs()
                .map(|install| GenerationInstall {
                    package: install.package.clone(),
                    version: install.version.clone(),
                })
                .collect(),
        }
    }

    fn change_view(changed: Changed) -> PackageSetChange {
        match changed {
            Changed::Unchanged(current) => PackageSetChange {
                outcome: ChangeOutcome::Unchanged,
                generation: current.generation(),
                changes: Vec::new(),
                failures: Vec::new(),
            },
            Changed::Activated { set, changes } => PackageSetChange {
                outcome: ChangeOutcome::Activated,
                generation: set.generation(),
                changes: changes
                    .into_iter()
                    .map(|change| match change {
                        Change::Add(install) => PackageChange {
                            change: ChangeKind::Add,
                            package: install.package,
                            from: None,
                            to: Some(install.version),
                        },
                        Change::Replace { from, to } => PackageChange {
                            change: ChangeKind::Replace,
                            package: to.package,
                            from: Some(from.version),
                            to: Some(to.version),
                        },
                        Change::Remove(install) => PackageChange {
                            change: ChangeKind::Remove,
                            package: install.package,
                            from: Some(install.version),
                            to: None,
                        },
                    })
                    .collect(),
                failures: Vec::new(),
            },
            Changed::Failed { current, jobs } => PackageSetChange {
                outcome: ChangeOutcome::Failed,
                generation: current.generation(),
                changes: Vec::new(),
                failures: jobs
                    .into_iter()
                    .filter_map(|(install, state)| {
                        let (stage, reason) = match state {
                            JobState::Installed => return None,
                            JobState::Failed { stage, failure } => (stage, Self::describe(failure)),
                            JobState::Interrupted { stage } => (stage, "interrupted".to_owned()),
                            JobState::Fetching => (Stage::Fetch, "stopped unfinished".to_owned()),
                            JobState::Unpacking => (Stage::Unpack, "stopped unfinished".to_owned()),
                            JobState::Placing => (Stage::Place, "stopped unfinished".to_owned()),
                        };
                        Some(InstallFailure {
                            package: install.package,
                            version: install.version,
                            stage,
                            reason,
                        })
                    })
                    .collect(),
            },
        }
    }

    /// A failure in words, for the operator.
    fn describe(failure: Failure) -> String {
        match failure {
            Failure::Mismatch { actual } => {
                format!("the download's SHA-256 is {actual}, not the one the catalog pins")
            }
            Failure::ProgramMissing => "the archive has no program where the catalog says".into(),
            Failure::Host(failure) => failure.to_string().replace('_', " "),
        }
    }
}

impl<F: Fetch + 'static> Packages for HostPackages<F> {
    fn listing(&self) -> BoxFuture<'static, Result<PackageListing, PackagesError>> {
        let packages = self.clone();
        async move { packages.read_listing().await }.boxed()
    }

    fn change(&self, edit: Edit) -> BoxFuture<'static, Result<PackageSetChange, PackagesError>> {
        let packages = self.clone();
        async move { packages.apply(edit).await }.boxed()
    }

    fn roll_back(
        &self,
        generation: Generation,
    ) -> BoxFuture<'static, Result<PackageSetChange, PackagesError>> {
        let packages = self.clone();
        async move { packages.go_back(generation).await }.boxed()
    }

    fn collect(&self) -> BoxFuture<'static, Result<PackageCollection, PackagesError>> {
        let packages = self.clone();
        async move { packages.sweep().await }.boxed()
    }
}
