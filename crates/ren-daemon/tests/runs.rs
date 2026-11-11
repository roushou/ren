//! Runs through the control API end to end: start a scripted Codex agent in
//! a root, prompt it, answer its approval, follow its updates, and stop it;
//! what the store kept afterwards. Assembled with `TestGateway`; the agent is
//! in memory, so no client, provider, or credentials are involved.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly, and scripted agents are spawned tasks"
)]

use axum::Router;
use http::StatusCode;
use ren_protocol::label::Stored;
use ren_recorder::capture::{ProcessExit, RunEnd};
use ren_store::{Reader, query::PageLimit};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

mod support;

use support::{TestGateway, agent::ScriptedHost, client, config, mock};

struct Control {
    base: String,
    http: reqwest::Client,
}

impl Control {
    fn new(gateway: &TestGateway) -> Self {
        Self {
            base: format!("{}/control/v1", gateway.base()),
            http: client(),
        }
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(TestGateway::CONTROL_TOKEN)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.bytes().await.unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, body)
    }

    async fn runs(&self) -> Value {
        self.http
            .get(format!("{}/runs", self.base))
            .bearer_auth(TestGateway::CONTROL_TOKEN)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .map(|bytes| serde_json::from_slice(&bytes).unwrap())
            .unwrap()
    }

    /// The run's state once `wanted` holds for it.
    async fn until(&self, wanted: impl Fn(&Value) -> bool) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let runs = self.runs().await;
                if let Some(state) = runs["runs"]
                    .as_array()
                    .and_then(|runs| runs.first())
                    .map(|run| run["state"].clone())
                    && wanted(&state)
                {
                    return state;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the run reached the wanted state")
    }
}

#[tokio::test]
async fn a_run_is_started_prompted_approved_and_stopped_through_the_control_api() {
    let upstream = mock(Router::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::with_runs(
        &config(&upstream),
        dir.path(),
        vec![PathBuf::from("/work")],
        ScriptedHost,
    )
    .await;
    let control = Control::new(&gateway);

    // Updates stream as server-sent events from the moment of subscribing.
    let events = control
        .http
        .get(format!("{}/runs/events", control.base))
        .bearer_auth(TestGateway::CONTROL_TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);

    // The folders a run can start in: the root and the folders inside it.
    let folders: serde_json::Value = serde_json::from_slice(
        &control
            .http
            .get(format!("{}/runs/folders", control.base))
            .bearer_auth(TestGateway::CONTROL_TOKEN)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        folders["folders"],
        json!([
            { "path": "/work", "name": "work" },
            { "path": "/work/acme", "name": "work/acme" },
            { "path": "/work/web", "name": "work/web" },
        ])
    );

    let (status, body) = control
        .post("/runs", json!({ "harness": "codex", "dir": "/work/acme" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let run = body["run"].as_str().unwrap().to_owned();
    control.until(|state| state["phase"] == "idle").await;

    let (status, _) = control
        .post(
            &format!("/runs/{run}/prompt"),
            json!({ "text": "edit src" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, body) = control
        .post(&format!("/runs/{run}/prompt"), json!({ "text": "again" }))
        .await;
    assert_eq!(
        (status, &body["error"]),
        (StatusCode::CONFLICT, &json!("busy"))
    );
    let waiting = control
        .until(|state| state["approvals"].as_array().is_some_and(|a| !a.is_empty()))
        .await;
    assert_eq!(waiting["phase"], "working");
    assert_eq!(
        waiting["approvals"],
        json!([{ "approval": "ap-1", "ask": { "kind": "file_change", "reason": "edit src" } }])
    );

    let (status, _) = control
        .post(
            &format!("/runs/{run}/approvals/ap-1"),
            json!({ "decision": "accept" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    control.until(|state| state["phase"] == "idle").await;

    let (status, _) = control.post(&format!("/runs/{run}/stop"), json!({})).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let ended = control.until(|state| state["phase"] == "ended").await;
    assert_eq!(ended["end"], "stopped");
    assert_eq!(ended["exit"], json!({ "kind": "signal", "value": 9 }));

    // The feed carried the run's states and what the agent reported.
    let mut feed = String::new();
    let mut events = events;
    while !feed.contains("\"phase\":\"ended\"") {
        let chunk = tokio::time::timeout(Duration::from_secs(5), events.chunk())
            .await
            .expect("an update")
            .unwrap()
            .expect("the feed is open");
        feed.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(feed.contains("event: run"));
    assert!(feed.contains("\"kind\":\"approval\""), "{feed}");
    assert!(feed.contains("\"text\":\"Edited.\""), "{feed}");
    drop(events);

    // The read API serves the stored run once its end is committed.
    let stored = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let body = control
                .http
                .get(format!("{}/api/runs/{run}", gateway.base()))
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            if body["end"] == "stopped" {
                return body;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the stored run");
    assert_eq!(stored["harness"], "codex");
    assert_eq!(stored["workdir"], "/work/acme");
    assert_eq!(stored["thread"], support::agent::THREAD);
    let listed = control
        .http
        .get(format!("{}/api/runs?limit=5", gateway.base()))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let listed: Value = serde_json::from_slice(&listed).unwrap();
    assert_eq!(listed["runs"][0]["id"], run.as_str());

    gateway.stop().await;
    let reader = Reader::open(dir.path(), 1).await.unwrap();
    let stored = reader.runs(PageLimit::DEFAULT).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id.to_string(), run);
    assert_eq!(stored[0].thread.as_deref(), Some(support::agent::THREAD));
    assert_eq!(stored[0].end, Some(Stored::Known(RunEnd::Stopped)));
    assert_eq!(stored[0].exit, Some(ProcessExit::Signal(9)));
    reader.close().await;
}

#[tokio::test]
async fn runs_refuse_bad_requests_browsers_and_missing_tokens() {
    let upstream = mock(Router::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::with_runs(
        &config(&upstream),
        dir.path(),
        vec![PathBuf::from("/work")],
        ScriptedHost,
    )
    .await;
    let control = Control::new(&gateway);
    let cases = [
        (
            json!({ "harness": "codex", "dir": "/elsewhere" }),
            StatusCode::BAD_REQUEST,
            "invalid_directory",
        ),
        (
            json!({ "harness": "codex", "dir": "/work/../etc" }),
            StatusCode::BAD_REQUEST,
            "invalid_directory",
        ),
        (
            json!({ "harness": "pi", "dir": "/work/a" }),
            StatusCode::BAD_REQUEST,
            "not_drivable",
        ),
        (
            json!({ "harness": "codex" }),
            StatusCode::BAD_REQUEST,
            "invalid_request",
        ),
    ];
    for (body, status, error) in cases {
        let (answered, answer) = control.post("/runs", body.clone()).await;
        assert_eq!(
            (answered, answer["error"].as_str()),
            (status, Some(error)),
            "{body}"
        );
    }
    let unknown = "00000000-0000-0000-0000-00000000ffff";
    let (status, body) = control
        .post(&format!("/runs/{unknown}/stop"), json!({}))
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::NOT_FOUND, Some("not_found"))
    );

    let anonymous = control
        .http
        .post(format!("{}/runs", control.base))
        .header("content-type", "application/json")
        .body(json!({ "harness": "codex", "dir": "/work/a" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let browser = control
        .http
        .post(format!("{}/runs", control.base))
        .bearer_auth(TestGateway::CONTROL_TOKEN)
        .header("origin", "https://evil.example")
        .header("content-type", "application/json")
        .body(json!({ "harness": "codex", "dir": "/work/a" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(browser.status(), StatusCode::FORBIDDEN);
    assert!(control.runs().await["runs"].as_array().unwrap().is_empty());
    gateway.stop().await;
}

#[tokio::test]
async fn without_roots_runs_are_disabled() {
    let upstream = mock(Router::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let control = Control::new(&gateway);
    let (status, body) = control
        .post("/runs", json!({ "harness": "codex", "dir": "/work/a" }))
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::NOT_FOUND, Some("runs_disabled"))
    );
    gateway.stop().await;
}
