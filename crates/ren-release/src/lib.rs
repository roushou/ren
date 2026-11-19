//! How a build of Ren moves from `main` to production
//! (docs/plans/self-build.md, phase 4), pure: which build an instance runs
//! ([`Build`], its [`Revision`]).

mod build;

pub use build::{Build, Commit, Comparison, Revision, RevisionError, Tree};
