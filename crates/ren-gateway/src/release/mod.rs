//! How a build of Ren moves from `main` to production
//! (docs/plans/self-build.md, phase 4): which build an instance runs
//! ([`Build`], its `ren_git::SourceRevision`); what the canary's checks found of a build
//! ([`Verdict`]); when production takes a build that passed ([`Policy`]);
//! and where a release stands, with what promotion decided ([`Release`],
//! its [`Decision`]). The gateway reports
//! them (`/health`, `/api/release`); the factory (`cargo xtask canary`) runs
//! the canary and writes them.

mod build;
mod folder;
mod policy;
mod standing;
mod verdict;

pub use build::{Build, Comparison};
pub use folder::{NotAShotName, PromotionRequest, ShotName};
pub use policy::{
    MinuteOfDay, Policy, PolicyError, PreviewCapacity, PreviewCapacityOutOfRange, ProjectRelease,
    ReleaseOptions, Window,
};
pub use standing::{Assessment, Attempt, Canary, Decision, Lineage, Release};
pub use verdict::{Check, CheckKind, Outcome, Suite, Verdict};
