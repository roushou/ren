//! The fake provider's stand-in for GitHub's GraphQL endpoint: it answers
//! the daemon's pull-request query (`ren_github::Watch`) for the dev slot's
//! fake repositories, `acme/web`, `acme/api`, and `acme/ren`, with invented pull
//! requests in the states the project page draws: passing and approved,
//! failing, behind with checks running, conflicting, blocked, and a draft.
//! Any other repository is not found, as GitHub says for one a token
//! cannot read.

use axum::{Json, http::StatusCode};
use serde_json::{Value, json};

/// The fake GitHub.
pub(super) struct FakeGithub;

/// One invented pull request, in GitHub's terms.
struct FakePull {
    number: u64,
    title: &'static str,
    branch: &'static str,
    /// `mergeStateStatus`.
    merge: &'static str,
    /// `reviewDecision`.
    review: Option<&'static str>,
    /// Its head's checks summed up, and each one; none for a head with no
    /// checks.
    checks: Option<(&'static str, Vec<FakeCheck>)>,
    draft: bool,
}

/// One check run on a head.
struct FakeCheck {
    name: &'static str,
    status: &'static str,
    conclusion: Option<&'static str>,
}

impl FakeGithub {
    /// Where the fake provider serves it.
    pub(super) const PATH: &str = "/github/graphql";
    /// Where it serves an App's installation of a repository.
    pub(super) const INSTALLATION: &str = "/github/api/repos/{owner}/{name}/installation";
    /// Where it mints an installation's token.
    pub(super) const ACCESS_TOKENS: &str = "/github/api/app/installations/{id}/access_tokens";

    /// The fake App's one installation, on the fake repositories alone.
    pub(super) async fn installation(
        axum::extract::Path((owner, name)): axum::extract::Path<(String, String)>,
    ) -> (StatusCode, Json<Value>) {
        match (owner.as_str(), name.as_str()) {
            ("acme", "web" | "api" | "ren") => (StatusCode::OK, Json(json!({ "id": 1 }))),
            _ => (
                StatusCode::NOT_FOUND,
                Json(json!({ "message": "Not Found" })),
            ),
        }
    }

    /// A token for the repositories asked for. Invented: it opens nothing.
    pub(super) async fn access_tokens() -> (StatusCode, Json<Value>) {
        (
            StatusCode::CREATED,
            Json(json!({ "token": "ghs_dev-fake-grant", "expires_at": "2099-01-01T00:00:00Z" })),
        )
    }

    /// The answer to one query.
    pub(super) async fn answer(Json(query): Json<Value>) -> (StatusCode, Json<Value>) {
        let owner = query["variables"]["owner"].as_str().unwrap_or_default();
        let name = query["variables"]["name"].as_str().unwrap_or_default();
        let pulls = match (owner, name) {
            ("acme", "web") => Self::web(),
            ("acme", "api") => Self::api(),
            ("acme", "ren") => Self::ren(),
            _ => {
                return (
                    StatusCode::OK,
                    Json(json!({ "data": { "repository": null }, "errors": [{
                        "type": "NOT_FOUND",
                        "message": format!("Could not resolve to a Repository with the name '{owner}/{name}'."),
                    }] })),
                );
            }
        };
        let nodes: Vec<Value> = pulls.iter().map(|pull| pull.node(name)).collect();
        // acme/api's default branch lists a registry of its own for its
        // runs; the others have no manifest.
        let manifest = match name {
            "api" => {
                json!({ "text": "reach = [\"static.crates.io:443\"]\n", "isTruncated": false })
            }
            _ => Value::Null,
        };
        (
            StatusCode::OK,
            Json(json!({ "data": { "repository": {
                "defaultBranchRef": { "name": "main", "target": { "oid": FakePull::oid(0xa0) } },
                "manifest": manifest,
                "pullRequests": { "pageInfo": { "hasNextPage": false }, "nodes": nodes },
            } } })),
        )
    }

    fn web() -> Vec<FakePull> {
        vec![
            FakePull {
                number: 41,
                title: "feat: the pricing page shows yearly plans",
                branch: "pricing-yearly",
                merge: "CLEAN",
                review: Some("APPROVED"),
                checks: Some((
                    "SUCCESS",
                    vec![
                        FakeCheck::done("CI", "SUCCESS"),
                        FakeCheck::done("Compatibility", "SUCCESS"),
                    ],
                )),
                draft: false,
            },
            FakePull {
                number: 40,
                title: "fix: the signup form keeps its fields after an error",
                branch: "signup-fields",
                merge: "BEHIND",
                review: Some("REVIEW_REQUIRED"),
                checks: Some((
                    "PENDING",
                    vec![
                        FakeCheck::running("CI"),
                        FakeCheck::done("Compatibility", "SUCCESS"),
                    ],
                )),
                draft: false,
            },
            FakePull {
                number: 38,
                title: "refactor: one layout for every marketing page",
                branch: "marketing-layout",
                merge: "DIRTY",
                review: Some("CHANGES_REQUESTED"),
                checks: Some((
                    "FAILURE",
                    vec![
                        FakeCheck::done("CI", "FAILURE"),
                        FakeCheck::done("Compatibility", "SUCCESS"),
                    ],
                )),
                draft: false,
            },
        ]
    }

    /// The checkout's own project, `ren`: its pull requests' invented heads
    /// are on no remote, so the factory's previews of them fail to fetch,
    /// as the Overview shows.
    fn ren() -> Vec<FakePull> {
        vec![
            FakePull {
                number: 7,
                title: "feat: the Overview lists each pull request's preview",
                branch: "pull-previews",
                merge: "CLEAN",
                review: Some("APPROVED"),
                checks: Some((
                    "SUCCESS",
                    vec![
                        FakeCheck::done("CI", "SUCCESS"),
                        FakeCheck::done("Compatibility", "SUCCESS"),
                    ],
                )),
                draft: false,
            },
            FakePull {
                number: 6,
                title: "fix: a canary pass keeps its previews' builds",
                branch: "keep-builds",
                merge: "BEHIND",
                review: Some("REVIEW_REQUIRED"),
                checks: Some(("PENDING", vec![FakeCheck::running("CI")])),
                draft: false,
            },
        ]
    }

    fn api() -> Vec<FakePull> {
        vec![
            FakePull {
                number: 17,
                title: "feat: rate limits per API key",
                branch: "key-limits",
                merge: "BLOCKED",
                review: Some("REVIEW_REQUIRED"),
                checks: Some((
                    "FAILURE",
                    vec![
                        FakeCheck::done("CI", "SUCCESS"),
                        FakeCheck::done("Compatibility", "FAILURE"),
                    ],
                )),
                draft: false,
            },
            FakePull {
                number: 16,
                title: "wip: move sessions to the new store",
                branch: "sessions-store",
                merge: "DRAFT",
                review: None,
                checks: None,
                draft: true,
            },
        ]
    }
}

impl FakePull {
    /// The pull request as GitHub's answer lists it, in `repository`.
    fn node(&self, repository: &str) -> Value {
        let rollup = self.checks.as_ref().map(|(state, checks)| {
            let nodes: Vec<Value> = checks.iter().map(FakeCheck::node).collect();
            json!({ "state": state, "contexts": { "pageInfo": { "hasNextPage": false }, "nodes": nodes } })
        });
        json!({
            "number": self.number,
            "title": self.title,
            "body": "",
            "url": format!("https://github.com/acme/{repository}/pull/{}", self.number),
            "isDraft": self.draft,
            "author": { "login": "acme-dev" },
            "headRefName": self.branch,
            "headRefOid": Self::oid(self.number),
            "baseRefName": "main",
            "mergeStateStatus": self.merge,
            "reviewDecision": self.review,
            "commits": { "nodes": [{ "commit": { "statusCheckRollup": rollup } }] },
        })
    }

    /// An invented commit id: 40 hex digits from `seed`.
    fn oid(seed: u64) -> String {
        format!("{seed:040x}")
    }
}

impl FakeCheck {
    fn done(name: &'static str, conclusion: &'static str) -> Self {
        Self {
            name,
            status: "COMPLETED",
            conclusion: Some(conclusion),
        }
    }

    fn running(name: &'static str) -> Self {
        Self {
            name,
            status: "IN_PROGRESS",
            conclusion: None,
        }
    }

    fn node(&self) -> Value {
        json!({
            "__typename": "CheckRun",
            "name": self.name,
            "status": self.status,
            "conclusion": self.conclusion,
        })
    }
}
