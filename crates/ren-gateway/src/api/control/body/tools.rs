//! DRAFT

use crate::tools::ChangeKind;
use crate::tools::ChangeOutcome;
use crate::tools::GenerationState;
use ren_depot::job::Stage;
use ren_depot::package::Kind;
use ren_depot::package::Origin;
use ren_depot::package::PackageId;
use ren_depot::package::ProgramName;
use ren_depot::source::Platform;
use ren_depot::toolset::Generation;
use ren_harness::Harness;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Every package Ren knows, what this host has of each, and the toolset
/// generations kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolListing {
    /// The platform Ren installs for here; `None` where it installs nothing.
    #[ts(as = "Option<String>")]
    pub platform: Option<Platform>,
    /// The current generation; 0 before any.
    #[ts(as = "u64")]
    pub generation: Generation,
    /// The current generation's folder of programs, which runs put first on
    /// their search path; `None` before any.
    pub bin: Option<String>,
    pub packages: Vec<ToolPackage>,
    /// Every generation kept, newest first.
    pub generations: Vec<GenerationView>,
}

impl From<crate::tools::ToolListing> for ToolListing {
    fn from(value: crate::tools::ToolListing) -> Self {
        let crate::tools::ToolListing {
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
pub struct ToolChange {
    #[ts(as = "String")]
    pub outcome: ChangeOutcome,
    /// The current generation after the change.
    #[ts(as = "u64")]
    pub generation: Generation,
    pub changes: Vec<PackageChange>,
    pub failures: Vec<InstallFailure>,
}

impl From<crate::tools::ToolChange> for ToolChange {
    fn from(value: crate::tools::ToolChange) -> Self {
        let crate::tools::ToolChange {
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
pub struct ToolCollection {
    #[ts(as = "Vec<u64>")]
    pub generations: Vec<Generation>,
    pub installs: Vec<String>,
}

impl From<crate::tools::ToolCollection> for ToolCollection {
    fn from(value: crate::tools::ToolCollection) -> Self {
        let crate::tools::ToolCollection {
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

impl From<crate::tools::GenerationInstall> for GenerationInstall {
    fn from(value: crate::tools::GenerationInstall) -> Self {
        let crate::tools::GenerationInstall { package, version } = value;
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

impl From<crate::tools::GenerationView> for GenerationView {
    fn from(value: crate::tools::GenerationView) -> Self {
        let crate::tools::GenerationView {
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

impl From<crate::tools::InstallFailure> for InstallFailure {
    fn from(value: crate::tools::InstallFailure) -> Self {
        let crate::tools::InstallFailure {
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
pub struct InstalledTool {
    #[ts(as = "String")]
    pub version: semver::Version,
    pub sha256: String,
    /// Its folder in the store.
    pub dir: String,
}

impl From<crate::tools::InstalledTool> for InstalledTool {
    fn from(value: crate::tools::InstalledTool) -> Self {
        let crate::tools::InstalledTool {
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

impl From<crate::tools::PackageChange> for PackageChange {
    fn from(value: crate::tools::PackageChange) -> Self {
        let crate::tools::PackageChange {
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
pub struct SystemTool {
    pub path: String,
    /// The version it reported, when Ren could read one.
    #[ts(as = "Option<String>")]
    pub version: Option<semver::Version>,
    /// What it printed instead, or why it did not answer.
    pub unread: Option<String>,
}

impl From<crate::tools::SystemTool> for SystemTool {
    fn from(value: crate::tools::SystemTool) -> Self {
        let crate::tools::SystemTool {
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
pub struct ToolPackage {
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
    pub installed: Option<InstalledTool>,
    /// What this host's search path has.
    pub system: Option<SystemTool>,
    /// The clients that need it.
    #[ts(as = "Vec<String>")]
    pub needed_by: Vec<Harness>,
}

impl From<crate::tools::ToolPackage> for ToolPackage {
    fn from(value: crate::tools::ToolPackage) -> Self {
        let crate::tools::ToolPackage {
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
