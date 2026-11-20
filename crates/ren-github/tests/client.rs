//! The client and the poller against a loopback stand-in for GitHub's
//! GraphQL endpoint: the query goes with the token, the answer is read, and
//! a refusal is a failure that keeps the last answer apart. Synthetic
//! answers and a fixture token only.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly; the stand-in spawns a task"
)]

use axum::{Json, Router, extract::State, http::HeaderMap, http::StatusCode, routing::post};
use ren_github::{
    Client, Endpoint, FetchError, Observation, PollPeriod, Poller, RepositorySlug, Token,
};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

const TOKEN: &str = "github_pat_FIXTURE";

/// What the stand-in saw, and whether it refuses.
#[derive(Default)]
struct StandIn {
    requests: Mutex<Vec<(Option<String>, Value)>>,
    refuse: AtomicBool,
}

async fn graphql(
    State(stand_in): State<Arc<StandIn>>,
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
            "defaultBranchRef": { "name": "main", "target": { "oid": "1".repeat(40) } },
            "pullRequests": { "pageInfo": { "hasNextPage": false }, "nodes": [{
                "number": 7, "title": "fix: the parser", "body": "", "url": "https://github.com/acme/web/pull/7",
                "isDraft": false, "author": { "login": "acme-bot" },
                "headRefName": "parser", "headRefOid": "2".repeat(40),
                "baseRefName": "main", "mergeStateStatus": "CLEAN", "reviewDecision": "APPROVED",
                "commits": { "nodes": [{ "commit": { "statusCheckRollup": null } }] }
            }] }
        } } })),
    )
}

async fn stand_in() -> (Arc<StandIn>, SocketAddr) {
    let stand_in = Arc::new(StandIn::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/graphql", post(graphql))
        .with_state(stand_in.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (stand_in, address)
}

fn client(address: SocketAddr) -> Client {
    Client::new(
        Endpoint::parse(&format!("http://{address}/graphql")).unwrap(),
        Token::new(TOKEN).unwrap(),
    )
    .unwrap()
}

fn web() -> RepositorySlug {
    "acme/web".parse().unwrap()
}

#[tokio::test]
async fn the_client_sends_the_query_with_the_token() {
    let (stand_in, address) = stand_in().await;
    let snapshot = client(address).snapshot(&web()).await.unwrap();
    assert_eq!(snapshot.pull_requests[0].number.get(), 7);
    let (auth, body) = stand_in.requests.lock().unwrap()[0].clone();
    assert_eq!(auth.as_deref(), Some(format!("Bearer {TOKEN}").as_str()));
    assert_eq!(body["variables"]["owner"], "acme");
    assert_eq!(body["variables"]["name"], "web");

    stand_in.refuse.store(true, Ordering::SeqCst);
    let refused = client(address).snapshot(&web()).await.unwrap_err();
    assert!(matches!(refused, FetchError::Status(status) if status == StatusCode::UNAUTHORIZED));
    assert!(!refused.to_string().contains(TOKEN));
}

#[tokio::test]
async fn the_poller_keeps_the_last_answer_apart_after_a_failure() {
    let (stand_in, address) = stand_in().await;
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let period: PollPeriod = 10.try_into().unwrap();
    let observations = Arc::new(Mutex::new(Vec::new()));
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let seen = observations.clone();
    let stopping = Arc::new(Mutex::new(Some(stop)));
    let refuse = stand_in.clone();
    let poller = Poller::new(client(address), period, [web()]);
    poller
        .run(
            move |repository, observation| {
                assert_eq!(repository, &web());
                let mut seen = seen.lock().unwrap();
                seen.push(observation.clone());
                // Refuse from now on, and stop after the next read.
                if seen.len() == 1 {
                    refuse.refuse.store(true, Ordering::SeqCst);
                } else if let Some(stop) = stopping.lock().unwrap().take() {
                    let _ = stop.send(());
                }
            },
            move || at,
            async move {
                let _ = stopped.await;
            },
        )
        .await;
    let observations = observations.lock().unwrap();
    let [
        Observation::Seen(first),
        Observation::Failed { error, last, .. },
    ] = observations.as_slice()
    else {
        panic!("seen, then failed: {observations:?}");
    };
    assert_eq!(first.at, at);
    assert_eq!(error, "GitHub answered 401 Unauthorized");
    assert_eq!(last.as_ref(), Some(first));
}
