//! A project's pull requests on GitHub (docs/plans/self-build.md, phase 8):
//! `GET /api/projects/{name}/pulls`, what the daemon's poller last saw of
//! the repository the project is watched on, `404` for a name no project
//! has. Each pull request names the work items its description's
//! `Work-Item:` trailers address.

use super::{ApiError, ApiState};
use crate::github::PullBoard;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use ren_github::{
    Check, CheckState, ChecksState, Listing, MergeState, Observation, Oid, PullRequest, Readiness,
    Reported, ReviewDecision, Sighting, Snapshot,
};
use ren_recorder::{id::WorkItemId, work::Landing};
use serde::Serialize;
use std::sync::Arc;
use std::time::SystemTime;
use ts_rs::TS;

/// `/api/projects/{name}/pulls`' answer.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
#[serde(tag = "watch", rename_all = "snake_case")]
pub enum PullsBody {
    /// No GitHub repository is configured for the project.
    Off,
    /// Its repository is watched and has not answered yet.
    Waiting { repository: String },
    /// Its newest poll was answered.
    Seen {
        repository: String,
        seen: SightingBody,
    },
    /// Its newest poll failed; `last` is the last answer, if any, not current.
    Failed {
        repository: String,
        error: String,
        /// Unix milliseconds.
        at: u64,
        last: Option<SightingBody>,
    },
}

/// One answer from GitHub.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SightingBody {
    /// Unix milliseconds.
    pub at: u64,
    pub default_branch: Option<BranchBody>,
    pub pull_requests: Vec<PullBody>,
    /// Whether more open pull requests (or checks) exist than were listed.
    pub truncated: bool,
}

/// A branch and its head.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct BranchBody {
    pub name: String,
    #[ts(as = "String")]
    pub head: Oid,
}

/// One open pull request.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct PullBody {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// Its author's GitHub login; none for a deleted account.
    pub author: Option<String>,
    /// `draft` or `ready`.
    #[ts(as = "String")]
    pub readiness: Readiness,
    pub head_branch: String,
    #[ts(as = "String")]
    pub head: Oid,
    pub base_branch: String,
    /// Whether it can merge now: `clean`, `behind`, `blocked`,
    /// `conflicting`, `draft`, `unstable`, `has_hooks`, `computing`, or
    /// GitHub's own text for a state Ren does not know.
    #[ts(as = "String")]
    pub merge: Reported<MergeState>,
    /// `approved`, `changes_requested`, `review_required`, or
    /// `not_required`.
    #[ts(as = "String")]
    pub review: Reported<ReviewDecision>,
    /// Its head's checks summed up: `passing`, `failing`, `errored`,
    /// `pending`, `expected`, or `none`.
    #[ts(as = "String")]
    pub checks: Reported<ChecksState>,
    pub check_list: Vec<CheckBody>,
    /// The work items its description names.
    #[ts(as = "Vec<String>")]
    pub work_items: Vec<WorkItemId>,
}

/// One check on a pull request's head.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct CheckBody {
    pub name: String,
    #[ts(as = "String")]
    pub state: Reported<CheckState>,
}

impl ApiState {
    /// A project's pull requests, as last seen.
    pub(super) async fn project_pulls(
        State(state): State<Arc<Self>>,
        Path(name): Path<String>,
    ) -> Result<Response, ApiError> {
        let project = state.named(&name).await?;
        Ok(Self::versioned(PullsBody::of(&state.pulls, &project.name)).into_response())
    }
}

impl PullsBody {
    fn of(board: &PullBoard, project: &ren_recorder::project::ProjectName) -> Self {
        let Some(watched) = board.watched(project) else {
            return Self::Off;
        };
        let repository = watched.repository.to_string();
        match watched.observation {
            Observation::Waiting => Self::Waiting { repository },
            Observation::Seen(sighting) => Self::Seen {
                repository,
                seen: SightingBody::from(sighting),
            },
            Observation::Failed { error, at, last } => Self::Failed {
                repository,
                error,
                at: SightingBody::millis(at),
                last: last.map(SightingBody::from),
            },
        }
    }
}

impl SightingBody {
    /// A time as the API sends it: Unix milliseconds, zero before 1970.
    fn millis(at: SystemTime) -> u64 {
        at.duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            })
    }
}

impl From<Sighting> for SightingBody {
    fn from(sighting: Sighting) -> Self {
        let Snapshot {
            default_branch,
            pull_requests,
            listing,
        } = sighting.snapshot;
        Self {
            at: Self::millis(sighting.at),
            default_branch: default_branch.map(|(name, head)| BranchBody { name, head }),
            pull_requests: pull_requests.into_iter().map(PullBody::from).collect(),
            truncated: match listing {
                Listing::Complete => false,
                Listing::Truncated => true,
            },
        }
    }
}

impl From<PullRequest> for PullBody {
    fn from(pull: PullRequest) -> Self {
        Self {
            work_items: Landing::named([pull.body.as_str()], &[]),
            number: pull.number.get(),
            title: pull.title,
            url: pull.url,
            author: pull.author.map(|login| login.as_str().to_owned()),
            readiness: pull.readiness,
            head_branch: pull.head_branch,
            head: pull.head,
            base_branch: pull.base_branch,
            merge: pull.merge,
            review: pull.review,
            checks: pull.checks,
            check_list: pull.check_list.into_iter().map(CheckBody::from).collect(),
        }
    }
}

impl From<Check> for CheckBody {
    fn from(check: Check) -> Self {
        Self {
            name: check.name,
            state: check.state,
        }
    }
}
