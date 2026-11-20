//! The GitHub poller against a loopback stand-in for GitHub's GraphQL
//! endpoint: it sends the watch's query with the operator's token, records
//! what the answer shows, and records a refusal as a failure that keeps
//! the last answer apart. Synthetic answers and a fixture token only.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly; mock servers spawn tasks"
)]

mod support;

use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};
use http::StatusCode;
use ren_daemon::{
    config::{Config, ConfigError, GithubOptions, WatchedRepository},
    github::GithubPoller,
};
use ren_gateway::{
    Shutdown,
    github::{Observation, PullBoard},
};
use ren_harness::host::HostEnv;
use ren_recorder::time::{ManualClock, UnixMillis};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use support::mock;

const TOKEN: &str = "github_pat_FIXTURE";

/// What the stand-in saw, and whether it refuses.
#[derive(Default)]
struct GithubStandIn {
    requests: Mutex<Vec<(Option<String>, Value)>>,
    refuse: AtomicBool,
}

async fn graphql(
    State(stand_in): State<Arc<GithubStandIn>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    stand_in.requests.lock().unwrap().push((auth, body));
    if stand_in.refuse.load(Ordering::SeqCst) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "Bad credentials" })),
        );
    }
    (
        StatusCode::OK,
        Json(json!({ "data": { "repository": {
            "defaultBranchRef": { "name": "main", "target": { "oid": "1111111111111111111111111111111111111111" } },
            "pullRequests": { "pageInfo": { "hasNextPage": false }, "nodes": [{
                "number": 7, "title": "fix: the parser", "body": "", "url": "https://github.com/acme/web/pull/7",
                "isDraft": false, "author": { "login": "acme-bot" },
                "headRefName": "parser", "headRefOid": "2222222222222222222222222222222222222222",
                "baseRefName": "main", "mergeStateStatus": "CLEAN", "reviewDecision": "APPROVED",
                "commits": { "nodes": [{ "commit": { "statusCheckRollup": null } }] }
            }] }
        } } })),
    )
}

fn options(endpoint: String) -> GithubOptions {
    GithubOptions {
        endpoint,
        repository: vec![WatchedRepository {
            project: "acme-web".parse().unwrap(),
            github: "acme/web".parse().unwrap(),
        }],
        ..GithubOptions::default()
    }
}

fn config(endpoint: String) -> Config {
    Config {
        github: options(endpoint),
        ..Config::default()
    }
}

/// `board`'s observation of acme-web once `done` holds of it.
async fn until(board: &PullBoard, done: impl Fn(&Observation) -> bool) -> Observation {
    let project = "acme-web".parse().unwrap();
    for _ in 0..500 {
        let observation = board.watched(&project).unwrap().observation;
        if done(&observation) {
            return observation;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the board never got there");
}

#[tokio::test]
async fn the_poller_records_what_github_shows_and_keeps_it_apart_on_failure() {
    let stand_in = Arc::new(GithubStandIn::default());
    let github = mock(
        Router::new()
            .route("/graphql", post(graphql))
            .with_state(stand_in.clone()),
    )
    .await;
    let runtime = config(format!("http://{}/graphql", github.address))
        .resolve_with(&HostEnv::from_pairs([("REN_GITHUB_TOKEN", TOKEN)]))
        .unwrap();
    let mut watch = runtime.github().unwrap().clone();
    // The shortest period config allows is ten seconds; the test resolves
    // a real one, then polls again by hand rather than wait for it.
    assert_eq!(watch.period.duration(), Duration::from_secs(60));
    watch.period = 10.try_into().unwrap();
    let board = watch.board();
    let clock = Arc::new(ManualClock::at(UnixMillis::from_millis(1_800_000_000_000)));
    let shutdown = Shutdown::default();
    let poller = GithubPoller::new(watch.clone(), board.clone(), clock.clone()).unwrap();
    let task = tokio::spawn(poller.run(shutdown.clone()));

    let seen = until(&board, |observation| {
        matches!(observation, Observation::Seen(_))
    })
    .await;
    let Observation::Seen(sighting) = seen else {
        unreachable!()
    };
    assert_eq!(sighting.at, UnixMillis::from_millis(1_800_000_000_000));
    assert_eq!(sighting.snapshot.pull_requests[0].number.get(), 7);
    {
        let requests = stand_in.requests.lock().unwrap();
        let (auth, body) = &requests[0];
        assert_eq!(auth.as_deref(), Some(format!("Bearer {TOKEN}").as_str()));
        assert_eq!(body["variables"]["owner"], "acme");
        assert_eq!(body["variables"]["name"], "web");
    }
    shutdown.drain();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the poller stops as the gateway drains")
        .unwrap();

    // A refused poll is a failure that keeps the last answer apart.
    stand_in.refuse.store(true, Ordering::SeqCst);
    clock.advance(Duration::from_secs(60));
    let shutdown = Shutdown::default();
    let poller = GithubPoller::new(watch, board.clone(), clock).unwrap();
    let task = tokio::spawn(poller.run(shutdown.clone()));
    let failed = until(&board, |observation| {
        matches!(observation, Observation::Failed { .. })
    })
    .await;
    let Observation::Failed { error, at, last } = failed else {
        unreachable!()
    };
    assert_eq!(error, "GitHub answered 401 Unauthorized");
    assert!(!error.contains(TOKEN));
    assert_eq!(at, UnixMillis::from_millis(1_800_000_060_000));
    assert_eq!(last, Some(sighting));
    shutdown.drain();
    task.await.unwrap();
}

#[test]
fn a_watch_needs_its_token_and_a_safe_endpoint() {
    let endpoint = || GithubOptions::default().endpoint;
    let missing = config(endpoint()).resolve_with(&HostEnv::default());
    assert!(
        matches!(missing, Err(ConfigError::MissingGithubToken(name)) if name == "REN_GITHUB_TOKEN")
    );
    // Checking the structure alone does not need it.
    config(endpoint()).check().unwrap();

    let insecure = config("http://github.example/graphql".into()).validate();
    assert!(matches!(insecure, Err(ConfigError::GithubEndpoint(_))));

    let mut twice = config(endpoint());
    let first = twice.github.repository[0].clone();
    twice.github.repository.push(first);
    assert!(matches!(
        twice.validate(),
        Err(ConfigError::GithubProjectTwice(_))
    ));

    // Nothing named, nothing watched, and no token needed.
    let off = Config::default().resolve_with(&HostEnv::default()).unwrap();
    assert!(off.github().is_none());
}

#[test]
fn the_github_section_reads_from_toml() {
    let config: Config = toml::from_str(
        "[github]\ntoken_env = \"ACME_TOKEN\"\npoll_seconds = 120\n\n[[github.repository]]\nproject = \"acme-web\"\ngithub = \"acme/web\"\n",
    )
    .unwrap();
    config.validate().unwrap();
    let watch = config
        .resolve_with(&HostEnv::from_pairs([("ACME_TOKEN", TOKEN)]))
        .unwrap();
    let watch = watch.github().unwrap();
    assert_eq!(watch.period.duration(), Duration::from_secs(120));
    assert!(!format!("{:?}", watch.token).contains(TOKEN));
    assert!(toml::from_str::<Config>("[github]\npoll_seconds = 5\n").is_err());
}
