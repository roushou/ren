//! GitHub as Ren observes it (docs/plans/self-build.md, phase 8), pure: a
//! [`Watch`] on one repository is the query Ren sends GitHub's GraphQL API
//! and the [`Snapshot`] an answer shows: the default branch's head and each
//! open [`PullRequest`], with its head, its merge state, its review, and
//! its [`Check`]s. Values GitHub sends that Ren does not know are kept as
//! [`Stored::Unknown`](ren_protocol::label::Stored), never dropped or
//! rounded to a known one. The daemon sends the query and polls.

mod check;
mod pull;
mod repository;
mod watch;

#[cfg(test)]
mod purity;

pub use check::{Check, CheckState, ChecksState};
pub use pull::{
    GithubLogin, MergeState, NotAPullNumber, PullNumber, PullRequest, Readiness, ReviewDecision,
};
pub use repository::{RepositorySlug, SlugError};
pub use watch::{Listing, ReadError, Snapshot, Watch};
