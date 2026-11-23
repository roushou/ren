//! How a build of Ren moves from `main` to production
//! (docs/plans/self-build.md, phase 4), pure: which build an instance runs
//! ([`Build`], its [`Revision`]); what the canary's checks found of a build
//! ([`Verdict`]); when production takes a build that passed ([`Policy`]);
//! and where a release stands, with what promotion decided ([`Release`],
//! its [`Decision`]). The factory (`cargo xtask canary`) does the I/O.

mod build;
mod folder;
mod policy;
mod release;
mod verdict;

pub use build::{Build, Comparison, Revision, RevisionError};
pub use folder::{NotAShotName, PromotionRequest, ShotName};
pub use policy::{
    MinuteOfDay, Policy, PolicyError, PreviewCapacity, PreviewCapacityOutOfRange, ProjectRelease,
    ReleaseOptions, Window,
};
pub use release::{Assessment, Attempt, Canary, Decision, Lineage, Release};
pub use verdict::{Check, CheckKind, Outcome, Suite, Verdict};
