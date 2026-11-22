//! DRAFT

use crate::packages::ChangeKind;
use crate::packages::ChangeOutcome;
use crate::packages::GenerationState;
use ren_depot::job::Stage;
use ren_depot::package::Kind;
use ren_depot::package::Origin;
use ren_depot::package::PackageId;
use ren_depot::package::ProgramName;
use ren_depot::package_set::Generation;
use ren_depot::source::Platform;
use ren_harness::Harness;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Every package Ren knows, what this host has of each, and the package set
/// generations kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PackageListing {
    /// The platform Ren installs for here; `None` where it installs nothing.
    #[ts(as = "Option<String>")]
    pub platform: Option<Platform>,
    /// The current generation; 0 before any.
    #[ts(as = "u64")]
    pub generation: Generation,
    /// The current generation's folder of programs, which runs put first on
    /// their search path; `None` before any.
    pub bin: Option<String>,
    pub packages: Vec<ListedPackage>,
    /// Every generation kept, newest first.
    pub generations: Vec<GenerationView>,
}

impl From<crate::packages::PackageListing> for PackageListing {
    fn from(value: crate::packages::PackageListing) -> Self {
        let crate::packages::PackageListing {
            platform,
            generation,
            bin,
            packages,
            generations,
        } = value;
        Self {
            platform,
            generation,
            bin,
            packages: packages.into_iter().map(Into::into).collect(),
            generations: generations.into_iter().map(Into::into).collect(),
        }
    }
}

/// A change's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PackageSetChange {
    #[ts(as = "String")]
    pub outcome: ChangeOutcome,
    /// The current generation after the change.
    #[ts(as = "u64")]
    pub generation: Generation,
    pub changes: Vec<PackageChange>,
    pub failures: Vec<InstallFailure>,
}

impl From<crate::packages::PackageSetChange> for PackageSetChange {
    fn from(value: crate::packages::PackageSetChange) -> Self {
        let crate::packages::PackageSetChange {
            outcome,
            generation,
            changes,
            failures,
        } = value;
        Self {
            outcome,
            generation,
            changes: changes.into_iter().map(Into::into).collect(),
            failures: failures.into_iter().map(Into::into).collect(),
        }
    }
}

/// What a collection removed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PackageCollection {
    #[ts(as = "Vec<u64>")]
    pub generations: Vec<Generation>,
    pub installs: Vec<String>,
}

impl From<crate::packages::PackageCollection> for PackageCollection {
    fn from(value: crate::packages::PackageCollection) -> Self {
        let crate::packages::PackageCollection {
            generations,
            installs,
        } = value;
        Self {
            generations,
            installs,
        }
    }
}

/// One install a generation holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GenerationInstall {
    #[ts(as = "String")]
    pub package: PackageId,
    #[ts(as = "String")]
    pub version: semver::Version,
}

impl From<crate::packages::GenerationInstall> for GenerationInstall {
    fn from(value: crate::packages::GenerationInstall) -> Self {
        let crate::packages::GenerationInstall { package, version } = value;
        Self { package, version }
    }
}

/// A generation kept in the store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GenerationView {
    #[ts(as = "u64")]
    pub generation: Generation,
    #[ts(as = "String")]
    pub state: GenerationState,
    pub installs: Vec<GenerationInstall>,
}

impl From<crate::packages::GenerationView> for GenerationView {
    fn from(value: crate::packages::GenerationView) -> Self {
        let crate::packages::GenerationView {
            generation,
            state,
            installs,
        } = value;
        Self {
            generation,
            state,
            installs: installs.into_iter().map(Into::into).collect(),
        }
    }
}

/// An install that did not succeed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InstallFailure {
    #[ts(as = "String")]
    pub package: PackageId,
    #[ts(as = "String")]
    pub version: semver::Version,
    #[ts(as = "String")]
    pub stage: Stage,
    pub reason: String,
}

impl From<crate::packages::InstallFailure> for InstallFailure {
    fn from(value: crate::packages::InstallFailure) -> Self {
        let crate::packages::InstallFailure {
            package,
            version,
            stage,
            reason,
        } = value;
        Self {
            package,
            version,
            stage,
            reason,
        }
    }
}

/// A package's install in the current generation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InstalledPackage {
    #[ts(as = "String")]
    pub version: semver::Version,
    pub sha256: String,
    /// Its folder in the store.
    pub dir: String,
}

impl From<crate::packages::InstalledPackage> for InstalledPackage {
    fn from(value: crate::packages::InstalledPackage) -> Self {
        let crate::packages::InstalledPackage {
            version,
            sha256,
            dir,
        } = value;
        Self {
            version,
            sha256,
            dir,
        }
    }
}

/// One package's change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PackageChange {
    #[ts(as = "String")]
    pub change: ChangeKind,
    #[ts(as = "String")]
    pub package: PackageId,
    #[ts(as = "Option<String>")]
    pub from: Option<semver::Version>,
    #[ts(as = "Option<String>")]
    pub to: Option<semver::Version>,
}

impl From<crate::packages::PackageChange> for PackageChange {
    fn from(value: crate::packages::PackageChange) -> Self {
        let crate::packages::PackageChange {
            change,
            package,
            from,
            to,
        } = value;
        Self {
            change,
            package,
            from,
            to,
        }
    }
}

/// A package's program on the host's search path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SystemProgram {
    pub path: String,
    /// The version it reported, when Ren could read one.
    #[ts(as = "Option<String>")]
    pub version: Option<semver::Version>,
    /// What it printed instead, or why it did not answer.
    pub unread: Option<String>,
}

impl From<crate::packages::SystemProgram> for SystemProgram {
    fn from(value: crate::packages::SystemProgram) -> Self {
        let crate::packages::SystemProgram {
            path,
            version,
            unread,
        } = value;
        Self {
            path,
            version,
            unread,
        }
    }
}

/// One package: one the catalog names, or one the current generation
/// holds that the catalog no longer names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ListedPackage {
    #[ts(as = "String")]
    pub id: PackageId,
    /// Which catalog names it; `None` once it was taken out of the
    /// operator's configuration and only the install is left.
    #[ts(as = "Option<String>")]
    pub origin: Option<Origin>,
    /// What it is for; `None` when the catalog no longer names it.
    #[ts(as = "Option<String>")]
    pub kind: Option<Kind>,
    /// The program it provides (`rg` for `ripgrep`).
    #[ts(as = "String")]
    pub program: ProgramName,
    /// The release Ren installs; `None` for a package it only finds.
    #[ts(as = "Option<String>")]
    pub release: Option<semver::Version>,
    /// What the current generation holds of it.
    pub installed: Option<InstalledPackage>,
    /// What this host's search path has.
    pub system: Option<SystemProgram>,
    /// The clients that need it.
    #[ts(as = "Vec<String>")]
    pub needed_by: Vec<Harness>,
}

impl From<crate::packages::ListedPackage> for ListedPackage {
    fn from(value: crate::packages::ListedPackage) -> Self {
        let crate::packages::ListedPackage {
            id,
            origin,
            kind,
            program,
            release,
            installed,
            system,
            needed_by,
        } = value;
        Self {
            id,
            origin,
            kind,
            program,
            release,
            installed: installed.map(Into::into),
            system: system.map(Into::into),
            needed_by,
        }
    }
}
