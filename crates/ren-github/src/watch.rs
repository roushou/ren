//! A watch on one repository: the GraphQL query Ren sends, and the
//! [`Snapshot`] an answer shows. One query reads the default branch's head
//! and every open pull request with its checks, so a poll is one request.

use crate::{
    check::{Check, CheckState, ChecksState},
    pull::{GithubLogin, MergeState, PullNumber, PullRequest, Readiness, ReviewDecision},
    repository::RepositorySlug,
};
use ren_release::Commit;
use serde_json::json;

/// A watch on one GitHub repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watch {
    repository: RepositorySlug,
}

/// What one answer showed of a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The default branch, and its head; none for an empty repository.
    pub default_branch: Option<(String, Commit)>,
    /// Its open pull requests, most recently updated first.
    pub pull_requests: Vec<PullRequest>,
    /// Whether every open pull request is listed, or only the first
    /// [`Watch::PULL_REQUESTS`].
    pub listing: Listing,
}

/// Whether a list is whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listing {
    Complete,
    /// More exist than one answer holds; checks are cut the same way.
    Truncated,
}

/// Why an answer shows no snapshot.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("GitHub's answer is not the shape asked for: {0}")]
    Shape(String),
    /// GitHub refused the query, or part of it: its messages, which never
    /// hold the token.
    #[error("GitHub refused the query: {}", .0.join("; "))]
    Refused(Vec<String>),
    #[error("GitHub has no repository {0} this token can read")]
    NoRepository(RepositorySlug),
    #[error("GitHub reported a head that is not a commit: {0:?}")]
    Head(String),
    #[error("GitHub reported a pull request number that is not one: {0}")]
    Number(u64),
}

impl Watch {
    /// Open pull requests one answer lists.
    pub const PULL_REQUESTS: u32 = 50;
    /// Checks one answer lists for each pull request's head.
    pub const CHECKS: u32 = 50;

    const QUERY: &str = "query($owner: String!, $name: String!, $pulls: Int!, $checks: Int!) {
  repository(owner: $owner, name: $name) {
    defaultBranchRef { name target { oid } }
    pullRequests(states: OPEN, first: $pulls, orderBy: {field: UPDATED_AT, direction: DESC}) {
      pageInfo { hasNextPage }
      nodes {
        number title body url isDraft
        author { login }
        headRefName headRefOid baseRefName
        mergeStateStatus reviewDecision
        commits(last: 1) { nodes { commit { statusCheckRollup {
          state
          contexts(first: $checks) {
            pageInfo { hasNextPage }
            nodes {
              __typename
              ... on CheckRun { name status conclusion }
              ... on StatusContext { context state }
            }
          }
        } } } }
      }
    }
  }
}";

    pub fn new(repository: RepositorySlug) -> Self {
        Self { repository }
    }

    pub fn repository(&self) -> &RepositorySlug {
        &self.repository
    }

    /// The request body to POST to GitHub's GraphQL endpoint.
    pub fn query(&self) -> Vec<u8> {
        json!({
            "query": Self::QUERY,
            "variables": {
                "owner": self.repository.owner(),
                "name": self.repository.name(),
                "pulls": Self::PULL_REQUESTS,
                "checks": Self::CHECKS,
            },
        })
        .to_string()
        .into_bytes()
    }

    /// What GitHub's answer to [`Watch::query`] shows.
    pub fn read(&self, answer: &[u8]) -> Result<Snapshot, ReadError> {
        let answer: wire::Answer =
            serde_json::from_slice(answer).map_err(|error| ReadError::Shape(error.to_string()))?;
        if let Some(errors) = answer.errors.filter(|errors| !errors.is_empty()) {
            return Err(ReadError::Refused(
                errors.into_iter().map(|error| error.message).collect(),
            ));
        }
        let repository = answer
            .data
            .and_then(|data| data.repository)
            .ok_or_else(|| ReadError::NoRepository(self.repository.clone()))?;
        let default_branch = repository
            .default_branch_ref
            .and_then(|branch| Some((branch.name, branch.target?.oid)))
            .map(|(name, oid)| Ok::<_, ReadError>((name, Self::commit(oid)?)))
            .transpose()?;
        let mut listing = Self::listing(repository.pull_requests.page_info.has_next_page);
        let mut pull_requests = Vec::new();
        for node in repository.pull_requests.nodes {
            let (pull, checks) = Self::pull_request(node)?;
            if checks == Listing::Truncated {
                listing = Listing::Truncated;
            }
            pull_requests.push(pull);
        }
        Ok(Snapshot {
            default_branch,
            pull_requests,
            listing,
        })
    }

    fn pull_request(node: wire::PullRequest) -> Result<(PullRequest, Listing), ReadError> {
        let rollup = node
            .commits
            .nodes
            .into_iter()
            .next_back()
            .and_then(|node| node.commit.status_check_rollup);
        let (checks, check_list, listing) = match rollup {
            None => (ChecksState::of_rollup(None), Vec::new(), Listing::Complete),
            Some(rollup) => (
                ChecksState::of_rollup(Some(&rollup.state)),
                rollup.contexts.nodes.into_iter().map(Self::check).collect(),
                Self::listing(rollup.contexts.page_info.has_next_page),
            ),
        };
        let pull = PullRequest {
            number: PullNumber::new(node.number).ok_or(ReadError::Number(node.number))?,
            title: node.title,
            body: node.body,
            url: node.url,
            author: node.author.map(|author| GithubLogin::new(author.login)),
            readiness: if node.is_draft {
                Readiness::Draft
            } else {
                Readiness::Ready
            },
            head_branch: node.head_ref_name,
            head: Self::commit(node.head_ref_oid)?,
            base_branch: node.base_ref_name,
            merge: MergeState::of_status(&node.merge_state_status),
            review: ReviewDecision::of_decision(node.review_decision.as_deref()),
            checks,
            check_list,
        };
        Ok((pull, listing))
    }

    fn check(context: wire::Context) -> Check {
        match context {
            wire::Context::CheckRun {
                name,
                status,
                conclusion,
            } => Check {
                name,
                state: CheckState::of_run(&status, conclusion.as_deref()),
            },
            wire::Context::StatusContext { context, state } => Check {
                name: context,
                state: CheckState::of_status(&state),
            },
        }
    }

    fn commit(oid: String) -> Result<Commit, ReadError> {
        oid.parse().map_err(|_| ReadError::Head(oid))
    }

    fn listing(has_next_page: bool) -> Listing {
        if has_next_page {
            Listing::Truncated
        } else {
            Listing::Complete
        }
    }
}

/// GitHub's answer, as far as the query asks.
mod wire {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub(super) struct Answer {
        pub data: Option<Data>,
        pub errors: Option<Vec<Error>>,
    }

    #[derive(Deserialize)]
    pub(super) struct Error {
        pub message: String,
    }

    #[derive(Deserialize)]
    pub(super) struct Data {
        pub repository: Option<Repository>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct Repository {
        pub default_branch_ref: Option<BranchRef>,
        pub pull_requests: Page<PullRequest>,
    }

    #[derive(Deserialize)]
    pub(super) struct BranchRef {
        pub name: String,
        pub target: Option<Target>,
    }

    #[derive(Deserialize)]
    pub(super) struct Target {
        pub oid: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct Page<T> {
        pub page_info: PageInfo,
        pub nodes: Vec<T>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct PageInfo {
        pub has_next_page: bool,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct PullRequest {
        pub number: u64,
        pub title: String,
        pub body: String,
        pub url: String,
        pub is_draft: bool,
        pub author: Option<Author>,
        pub head_ref_name: String,
        pub head_ref_oid: String,
        pub base_ref_name: String,
        pub merge_state_status: String,
        pub review_decision: Option<String>,
        pub commits: Nodes<CommitNode>,
    }

    #[derive(Deserialize)]
    pub(super) struct Author {
        pub login: String,
    }

    #[derive(Deserialize)]
    pub(super) struct Nodes<T> {
        pub nodes: Vec<T>,
    }

    #[derive(Deserialize)]
    pub(super) struct CommitNode {
        pub commit: Commit,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct Commit {
        pub status_check_rollup: Option<Rollup>,
    }

    #[derive(Deserialize)]
    pub(super) struct Rollup {
        pub state: String,
        pub contexts: Page<Context>,
    }

    /// A check on a commit: a check run (GitHub Actions and apps) or a
    /// commit status. GitHub types every context as one of the two.
    #[derive(Deserialize)]
    #[serde(tag = "__typename")]
    pub(super) enum Context {
        CheckRun {
            name: String,
            status: String,
            conclusion: Option<String>,
        },
        StatusContext {
            context: String,
            state: String,
        },
    }
}
