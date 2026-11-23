//! A GitHub client for a repository's pull requests: one GraphQL query
//! reads its default branch and every open [`PullRequest`], each with its
//! head, merge state, review, and [`Check`]s, into a [`Snapshot`].
//!
//! - [`Watch`] is the query and the reading of its answer.
//! - [`Client`] sends it to GitHub's GraphQL [`Endpoint`] with a
//!   [`Token`].
//! - [`Poller`] reads repositories once a [`PollPeriod`], keeping each
//!   one's [`Observation`]: waiting, seen, or failed with the last answer
//!   kept apart.
//!
//! - [`Installations`] mints, for a GitHub [`App`], a [`RepositoryToken`]
//!   that can push to one repository and open its pull requests.
//!
//! Values GitHub sends that this crate does not know are kept as
//! [`Reported::Unknown`], never dropped or rounded to a known one.

mod app;
mod check;
mod client;
mod observation;
mod oid;
mod poller;
mod pull;
mod reported;
mod repository;
mod watch;

pub use app::{
    App, AppId, AppKey, AppKeyError, AppToken, InstallationId, Installations, MintError,
    RepositoryToken, SignError,
};
pub use check::{Check, CheckState, ChecksState};
pub use client::{Client, Endpoint, EndpointError, FetchError, Token};
pub use observation::{Observation, Sighting};
pub use oid::{NotAnOid, Oid};
pub use poller::{PollPeriod, PollPeriodOutOfRange, Poller};
pub use pull::{
    Login, MergeState, NotAPullNumber, PullNumber, PullRequest, Readiness, ReviewDecision,
};
pub use reported::Reported;
pub use repository::{RepositorySlug, SlugError};
pub use watch::{Listing, ReadError, Snapshot, Watch};
