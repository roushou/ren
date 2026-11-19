//! Runs through the control API end to end: create a session with a scripted
//! Codex agent in a root, prompt it, answer its approval, follow its
//! process's updates, and stop it; what the store kept afterwards. Assembled with `TestGateway`; the agent is
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

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(TestGateway::CONTROL_TOKEN)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.bytes().await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
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

    /// `run`'s state once `wanted` holds for it.
    async fn until_run(&self, run: &str, wanted: impl Fn(&Value) -> bool) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let runs = self.runs().await;
                if let Some(state) = runs["runs"]
                    .as_array()
                    .and_then(|runs| runs.iter().find(|entry| entry["run"] == run))
                    .map(|entry| entry["state"].clone())
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

    /// `session` as the control API answers it, once `wanted` holds.
    async fn until_session(&self, session: &str, wanted: impl Fn(&Value) -> bool) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let (_, body) = self.get(&format!("/sessions/{session}")).await;
                if wanted(&body["session"]) {
                    return body["session"].clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the session reached the wanted state")
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
async fn a_session_is_started_prompted_approved_and_stopped_through_the_control_api() {
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

    // The projects a run can start in: a repository, and a root's folder
    // with none below it.
    let projects: serde_json::Value = serde_json::from_slice(
        &control
            .http
            .get(format!("{}/runs/places", control.base))
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
        projects["places"],
        json!([
            { "path": "/work/acme", "name": "acme", "shown": "~/acme", "kind": "project",
              "git": { "kind": "branch", "name": "main" } },
            { "path": "/work/web", "name": "web", "shown": "~/web", "kind": "project",
              "git": { "kind": "none" } },
        ])
    );

    // The clients a run can start with here: the scripted host has every
    // program.
    let clients: serde_json::Value = serde_json::from_slice(
        &control
            .http
            .get(format!("{}/runs/clients", control.base))
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
        clients["clients"],
        json!([
            { "harness": "claude_code", "status": "ready", "approvals": ["ask", "edits", "auto"],
              "midway": ["model", "approvals"], "sign_in": { "kind": "signed_out" },
              "sign_in_methods": ["claude_subscription", "anthropic_console"] },
            { "harness": "codex", "status": "ready", "approvals": ["ask", "edits", "auto"],
              "midway": ["steer", "model", "effort", "approvals"],
              "sign_in": { "kind": "signed_out" }, "sign_in_methods": ["chatgpt"] },
            { "harness": "pi", "status": "ready", "approvals": [], "midway": ["steer", "model", "effort"],
              "sign_in": { "kind": "signed_out" },
              "sign_in_methods": ["chatgpt", "anthropic_api_key", "openai_api_key"] },
            { "harness": "opencode", "status": "ready", "approvals": ["ask", "edits"],
              "midway": ["steer", "model", "effort"], "sign_in": { "kind": "signed_out" },
              "sign_in_methods": ["chatgpt", "anthropic_api_key", "openai_api_key"] },
        ])
    );

    // The models a client offers, as it lists them itself.
    let models: serde_json::Value = serde_json::from_slice(
        &control
            .http
            .get(format!("{}/runs/clients/codex/models", control.base))
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
        models,
        json!({ "api_version": 1, "harness": "codex", "models": [
            { "id": "gpt-fast", "name": "GPT Fast", "description": "Quick.",
              "provider": null, "choice": "default", "efforts": [], "effort": null },
        ] })
    );

    let (status, body) = control
        .post(
            "/sessions",
            json!({ "harness": "codex", "dir": "/work/acme", "model": "gpt-fast",
                    "effort": "high", "approvals": "edits" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session = body["session"].as_str().unwrap().to_owned();
    // A draft runs no client: its first prompt starts one.
    let (_, made) = control.get(&format!("/sessions/{session}")).await;
    assert!(made["session"]["served"]["process"].is_null(), "{made}");
    let (status, _) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "edit src" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let made = control
        .until_session(&session, |s| !s["served"]["process"].is_null())
        .await;
    let run = made["served"]["process"].as_str().unwrap().to_owned();
    // A running run is not deleted (once its start is recorded, it is found).
    let refused = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let answer = control
                .post(&format!("/history/runs/{run}/delete"), json!({}))
                .await;
            if answer.0 != StatusCode::NOT_FOUND {
                return answer;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the run is recorded");
    assert_eq!(
        (refused.0, &refused.1["error"]),
        (StatusCode::CONFLICT, &json!("run_running"))
    );

    let (status, body) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "again" }),
        )
        .await;
    assert_eq!(
        (status, &body["error"]),
        (StatusCode::CONFLICT, &json!("busy"))
    );
    let waiting = control
        .until_run(&run, |state| {
            state["approvals"].as_array().is_some_and(|a| !a.is_empty())
        })
        .await;
    assert_eq!(waiting["phase"], "working");
    assert_eq!(
        waiting["approvals"],
        json!([{ "approval": "ap-1", "ask": { "kind": "file_change", "reason": "edit src" } }])
    );

    let (status, _) = control
        .post(
            &format!("/sessions/{session}/approvals/ap-1"),
            json!({ "decision": "accept" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    control
        .until_run(&run, |state| state["phase"] == "idle")
        .await;

    let (status, _) = control
        .post(&format!("/sessions/{session}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let ended = control
        .until_run(&run, |state| state["phase"] == "ended")
        .await;
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
    // Its conversation is stored whole: the prompt, the file change, and
    // the reply, in turn order.
    let conversation: Value = serde_json::from_slice(
        &control
            .http
            .get(format!("{}/api/runs/{run}/conversation", gateway.base()))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
    )
    .unwrap();
    let kinds: Vec<&str> = conversation["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["event"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds.first(), Some(&"turn_started"), "{conversation}");
    assert_eq!(kinds.last(), Some(&"turn_ended"), "{conversation}");
    assert!(
        conversation.to_string().contains("Edited."),
        "{conversation}"
    );
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
    // Its settings, as it was started.
    let recorded = &listed["runs"][0];
    assert_eq!(
        (
            &recorded["model"],
            &recorded["effort"],
            &recorded["approvals"]
        ),
        (&json!("gpt-fast"), &json!("high"), &json!("edits"))
    );
    // Ended, it is archived: listed only on the archived shelf.
    let (status, _) = control
        .post(&format!("/history/runs/{run}/archive"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let body = control
                .http
                .get(format!("{}/api/runs?shelf=archived", gateway.base()))
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            let body: Value = serde_json::from_slice(&body).unwrap();
            if body["runs"][0]["id"] == run.as_str() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the archived run");

    gateway.stop().await;
    let reader = Reader::open(dir.path(), 1).await.unwrap();
    let stored = reader
        .runs(PageLimit::DEFAULT, ren_store::query::Shelf::Archived, None)
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id.to_string(), run);
    assert_eq!(stored[0].thread.as_deref(), Some(support::agent::THREAD));
    assert_eq!(stored[0].end, Some(Stored::Known(RunEnd::Stopped)));
    assert_eq!(stored[0].exit, Some(ProcessExit::Signal(9)));
    reader.close().await;
}

#[tokio::test]
async fn sessions_refuse_bad_requests_browsers_and_missing_tokens() {
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
            json!({ "dir": "/work/acme" }),
            StatusCode::BAD_REQUEST,
            "invalid_request",
        ),
    ];
    for (body, status, error) in cases {
        let (answered, answer) = control.post("/sessions", body.clone()).await;
        assert_eq!(
            (answered, answer["error"].as_str()),
            (status, Some(error)),
            "{body}"
        );
    }
    let unknown = "00000000-0000-0000-0000-00000000ffff";
    for session in [unknown, "not-a-session"] {
        let (status, body) = control
            .post(&format!("/sessions/{session}/stop"), json!({}))
            .await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("not_found")),
            "{session}"
        );
    }

    let anonymous = control
        .http
        .post(format!("{}/sessions", control.base))
        .header("content-type", "application/json")
        .body(json!({ "harness": "codex", "dir": "/work/a" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let browser = control
        .http
        .post(format!("{}/sessions", control.base))
        .bearer_auth(TestGateway::CONTROL_TOKEN)
        .header("origin", "https://evil.example")
        .header("content-type", "application/json")
        .body(json!({ "harness": "codex", "dir": "/work/a" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(browser.status(), StatusCode::FORBIDDEN);
    assert!(control.runs().await["runs"].as_array().unwrap().is_empty());
    let (status, sessions) = control.get("/sessions").await;
    assert_eq!(status, StatusCode::OK, "{sessions}");
    assert!(
        sessions["sessions"].as_array().unwrap().is_empty(),
        "{sessions}"
    );
    gateway.stop().await;
}

#[tokio::test]
async fn without_roots_runs_are_disabled() {
    let upstream = mock(Router::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let control = Control::new(&gateway);
    let (status, body) = control
        .post("/sessions", json!({ "harness": "codex", "dir": "/work/a" }))
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::NOT_FOUND, Some("runs_disabled"))
    );
    gateway.stop().await;
}

#[tokio::test]
async fn a_session_without_a_folder_starts_in_a_scratch_folder_of_its_own() {
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
    let (status, body) = control
        .post("/sessions", json!({ "harness": "codex" }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session = body["session"].as_str().unwrap().to_owned();
    let (status, _) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "go" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let made = control
        .until_session(&session, |s| !s["served"]["process"].is_null())
        .await;
    let run = made["served"]["process"].as_str().unwrap();
    let runs = control.runs().await;
    let workdir = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["run"] == run)
        .map(|entry| entry["workdir"].as_str().unwrap().to_owned())
        .unwrap();
    assert!(workdir.starts_with("/scratch/scratch-"), "{workdir}");
    gateway.stop().await;
}

#[tokio::test]
async fn a_prompt_queued_during_a_turn_goes_out_as_it_ends() {
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
    let (status, body) = control
        .post(
            "/sessions",
            json!({ "harness": "codex", "dir": "/work/acme" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session = body["session"].as_str().unwrap().to_owned();
    let (status, body) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "first" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    control
        .until(|state| {
            state["phase"] == "working"
                && state["approvals"].as_array().is_some_and(|a| !a.is_empty())
        })
        .await;

    // What a prompt is offered: the client's commands (this scripted
    // Codex lists none) and the folder's files.
    let (status, body) = control.get(&format!("/sessions/{session}/commands")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["commands"], json!([]));
    let (status, body) = control
        .get(&format!("/sessions/{session}/files?q=upl&limit=5"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["files"],
        json!([{ "path": "src/upload.rs", "kind": "file", "positions": [4, 5, 6] }])
    );
    let (status, _) = control
        .get("/sessions/00000000-0000-0000-0000-000000000063/files?q=x")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = control
        .get(&format!("/sessions/{session}/files?nope=1"))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Mid-turn: queued for after it, a steer into it, and new settings.
    let (status, body) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "second", "delivery": "queue" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let state = control.until(|state| state["queued"] == "second").await;
    assert_eq!(state["phase"], "working");
    let (status, body) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "and tests", "delivery": "steer" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let (status, body) = control
        .post(
            &format!("/sessions/{session}/settings"),
            json!({ "effort": "high" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let runs = control.runs().await;
    assert_eq!(runs["runs"][0]["settings"]["effort"], "high");
    // Nothing queued twice is taken back once.
    let (status, _) = control
        .post(&format!("/sessions/{session}/unqueue"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, body) = control
        .post(&format!("/sessions/{session}/unqueue"), json!({}))
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::CONFLICT, Some("nothing_queued"))
    );
    // Queued again, it goes out as the turn ends: the next turn asks again.
    let _ = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "second", "delivery": "queue" }),
        )
        .await;
    control.until(|state| state["queued"] == "second").await;
    let (status, _) = control
        .post(
            &format!("/sessions/{session}/approvals/ap-1"),
            json!({ "decision": "accept" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    control
        .until(|state| {
            state["phase"] == "working"
                && state.get("queued").is_none()
                && state["approvals"].as_array().is_some_and(|a| !a.is_empty())
        })
        .await;
    gateway.stop().await;
}

/// A session is deleted whole once its process has stopped: refused while
/// it runs, then gone from both the store and the supervisor, so no list
/// shows it again.
#[tokio::test]
async fn a_session_is_deleted_once_its_process_has_stopped() {
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
    let (status, body) = control
        .post(
            "/sessions",
            json!({ "harness": "codex", "dir": "/work/acme" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session = body["session"].as_str().unwrap().to_owned();
    let (status, _) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "go" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let made = control
        .until_session(&session, |s| {
            !s["stored"].is_null() && !s["served"]["process"].is_null()
        })
        .await;
    let process = made["served"]["process"].as_str().unwrap().to_owned();
    control
        .until_run(&process, |state| state["phase"] == "working")
        .await;

    let (status, body) = control
        .post(&format!("/sessions/{session}/delete"), json!({}))
        .await;
    assert_eq!(
        (status, body["error"].clone()),
        (StatusCode::CONFLICT, json!("run_running"))
    );
    let (status, _) = control
        .post(&format!("/sessions/{session}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    control
        .until_run(&process, |state| state["phase"] == "ended")
        .await;
    control
        .until_session(&session, |s| s["stored"]["runs"][0]["ended_at"].is_number())
        .await;
    let (status, body) = control
        .post(&format!("/sessions/{session}/delete"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    // Gone from the store once the recorder reaches the edit, and from the
    // supervisor at once.
    let mut listed = true;
    for _ in 0..200 {
        let (_, body) = control.get("/sessions").await;
        listed = body["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["session"] == session.as_str());
        if !listed {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!listed, "the deleted session is still listed");
    let (status, _) = control.get(&format!("/sessions/{session}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = control
        .post(&format!("/sessions/{session}/delete"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_session_is_created_driven_continued_archived_and_served_again_after_a_restart() {
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
    let (status, body) = control
        .post(
            "/sessions",
            json!({ "harness": "codex", "dir": "/work/acme" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session = body["session"].as_str().unwrap().to_owned();

    // A draft runs no client; the store records it a moment later.
    let made = control
        .until_session(&session, |s| !s["stored"].is_null())
        .await;
    assert_eq!(
        made["served"]["state"],
        json!({ "shelf": "open", "phase": "draft" })
    );
    assert!(made["served"]["process"].is_null(), "{made}");
    assert_eq!(made["stored"]["workdir"], "/work/acme");
    let (status, body) = control
        .get(&format!("/sessions/{session}/files?q=upl"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["files"][0]["path"], "src/upload.rs");
    let (status, body) = control.get(&format!("/sessions/{session}/commands")).await;
    assert_eq!(
        (status, body["commands"].clone()),
        (StatusCode::OK, json!([]))
    );

    // Its first prompt starts its client and fixes it.
    let (status, body) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "first" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let started = control
        .until_session(&session, |s| !s["served"]["process"].is_null())
        .await;
    let first = started["served"]["process"].as_str().unwrap().to_owned();
    control
        .until_run(&first, |state| {
            state["phase"] == "working"
                && state["approvals"].as_array().is_some_and(|a| !a.is_empty())
        })
        .await;
    let (status, body) = control
        .post(
            &format!("/sessions/{session}/client"),
            json!({ "harness": "pi" }),
        )
        .await;
    assert_eq!(
        (status, body["error"].clone()),
        (StatusCode::CONFLICT, json!("client_fixed"))
    );

    // Its process stops; the next prompt continues the same session.
    let (status, _) = control
        .post(&format!("/sessions/{session}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    control
        .until_run(&first, |state| state["phase"] == "ended")
        .await;
    let (status, body) = control
        .post(&format!("/sessions/{session}/interrupt"), json!({}))
        .await;
    assert_eq!(
        (status, body["error"].clone()),
        (StatusCode::CONFLICT, json!("no_process"))
    );
    let (status, _) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "second" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let continued = control
        .until_session(&session, |s| {
            s["stored"]["runs"]
                .as_array()
                .is_some_and(|runs| runs.len() == 2)
        })
        .await;
    let second = continued["served"]["process"].as_str().unwrap().to_owned();
    assert_ne!(second, first);
    assert_eq!(continued["served"]["state"]["phase"], "active");
    let state = control
        .until_run(&second, |state| state["phase"] == "working")
        .await;
    assert_eq!(
        state["thread"],
        support::agent::THREAD,
        "it resumed the conversation"
    );

    // Archived, its process stops and it leaves the current list.
    let (status, _) = control
        .post(&format!("/sessions/{session}/archive"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    control
        .until_run(&second, |state| state["phase"] == "ended")
        .await;
    control
        .until_session(&session, |s| !s["stored"]["archived_at"].is_null())
        .await;
    let (_, current) = control.get("/sessions").await;
    assert!(
        current["sessions"].as_array().unwrap().is_empty(),
        "{current}"
    );
    let (_, archived) = control.get("/sessions?shelf=archived").await;
    assert_eq!(archived["sessions"][0]["session"], session);
    let (status, _) = control
        .get("/sessions/00000000-0000-0000-0000-000000000063")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    gateway.stop().await;

    // After a restart, the stored session is served again on its first
    // request: written to, it resumes its conversation, current again.
    let gateway = TestGateway::with_runs(
        &config(&upstream),
        dir.path(),
        vec![PathBuf::from("/work")],
        ScriptedHost,
    )
    .await;
    let control = Control::new(&gateway);
    let (_, before) = control.get(&format!("/sessions/{session}")).await;
    assert!(before["session"]["served"].is_null(), "{before}");
    let (status, body) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "third" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let back = control
        .until_session(&session, |s| {
            s["stored"]["runs"]
                .as_array()
                .is_some_and(|runs| runs.len() == 3)
        })
        .await;
    assert_eq!(
        back["served"]["state"],
        json!({ "shelf": "open", "phase": "active" })
    );
    let third = back["served"]["process"].as_str().unwrap().to_owned();
    let state = control
        .until_run(&third, |state| state["phase"] == "working")
        .await;
    assert_eq!(state["thread"], support::agent::THREAD);
    control
        .until_session(&session, |s| s["stored"]["archived_at"].is_null())
        .await;
    gateway.stop().await;
}

#[tokio::test]
async fn a_client_is_signed_in_through_its_own_login_relayed_by_the_control_api() {
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
    let sign_in = "/runs/clients/claude_code/sign-in";

    // Nothing to cancel or hand a code to before a login runs.
    let (status, body) = control.post(&format!("{sign_in}/cancel"), json!({})).await;
    assert_eq!(
        (status, body["error"].clone()),
        (StatusCode::CONFLICT, json!("no_sign_in"))
    );
    let (status, body) = control
        .post(&format!("{sign_in}/answer"), json!({ "answer": "good" }))
        .await;
    assert_eq!(
        (status, body["error"].clone()),
        (StatusCode::CONFLICT, json!("no_sign_in"))
    );

    // Its status command, run now.
    let (status, body) = control.post(&format!("{sign_in}/check"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sign_in"], json!({ "kind": "signed_out" }));

    // The login answers with the link Claude Code printed.
    let (status, body) = control.post(sign_in, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["method"], "claude_subscription");
    assert_eq!(
        body["login"],
        json!({ "kind": "awaiting_code", "link": "https://login.example.test/authorize?state=acme",
                "said": "Opening browser to sign in" })
    );

    // A wrong code: the login fails and says why, and the client is still
    // signed out as its status says. The code is never answered with.
    let (status, body) = control
        .post(&format!("{sign_in}/answer"), json!({ "answer": "bad" }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["outcome"],
        json!({
            "end": { "end": "exited", "exit": { "kind": "code", "value": 1 },
                     "said": "Login failed: That code wasn't accepted." },
            "state": { "kind": "signed_out" },
        })
    );
    assert!(!body.to_string().contains("\"bad\""));

    // A login replaced by a newer one is cancelled; a code is refused once
    // the login has ended; a method the client lacks is invalid.
    let (status, _) = control
        .post(sign_in, json!({ "method": "anthropic_console" }))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = control.post(&format!("{sign_in}/cancel"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = control
        .post(&format!("{sign_in}/answer"), json!({ "answer": "good" }))
        .await;
    assert_eq!(
        (status, body["error"].clone()),
        (StatusCode::CONFLICT, json!("no_sign_in"))
    );
    let (status, _) = control
        .post(
            "/runs/clients/pi/sign-in",
            json!({ "method": "claude_subscription" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = control
        .post(&format!("{sign_in}/answer"), json!({ "answer": "" }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_device_login_shows_its_code_and_is_waited_on_until_it_ends() {
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
    let sign_in = "/runs/clients/codex/sign-in";

    let (status, _) = control.post(&format!("{sign_in}/wait"), json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Started, it answers once both the link and the code are printed.
    let (status, body) = control.post(sign_in, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["method"], "chatgpt");
    assert_eq!(
        body["login"],
        json!({ "kind": "awaiting_approval", "link": "https://login.example.test/codex/device",
                "code": "ACME-12345",
                "said": "2. Enter this one-time code (expires in 15 minutes)" })
    );
    // Nothing is pasted into a device login.
    let (status, body) = control
        .post(&format!("{sign_in}/answer"), json!({ "answer": "x" }))
        .await;
    assert_eq!(
        (status, body["error"].clone()),
        (StatusCode::CONFLICT, json!("not_awaiting_answer"))
    );

    // Approved, it ends by itself; the status check after it decides.
    let (status, body) = control.post(&format!("{sign_in}/wait"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["waited"],
        json!({ "kind": "done", "outcome": {
            "end": { "end": "exited", "exit": { "kind": "code", "value": 0 },
                     "said": "Successfully logged in" },
            "state": { "kind": "signed_out" } } })
    );
    // Asked again after it ended, the same outcome.
    let (_, again) = control.post(&format!("{sign_in}/wait"), json!({})).await;
    assert_eq!(again["waited"], body["waited"]);
}

#[tokio::test]
async fn a_key_is_typed_into_the_login_and_a_redirect_is_opened_only_when_it_is_the_one_waited_on()
{
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

    // Pi's API key: asked for at once, typed into its login, never answered with.
    let pi = "/runs/clients/pi/sign-in";
    let (status, body) = control
        .post(pi, json!({ "method": "anthropic_api_key" }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["login"],
        json!({ "kind": "awaiting_key", "said": null })
    );
    let (status, body) = control
        .post(
            &format!("{pi}/answer"),
            json!({ "answer": "sk-ant-acme-not-a-key" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["outcome"]["end"],
        json!({ "end": "exited", "exit": { "kind": "code", "value": 0 },
                "said": "Saved API key for Anthropic." })
    );
    assert!(!body.to_string().contains("sk-ant-acme"));

    // OpenCode's token sharing: the address pasted back must be the one its
    // link sends to.
    let opencode = "/runs/clients/opencode/sign-in";
    let (status, body) = control.post(opencode, json!({ "method": "chatgpt" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["login"]["kind"], "awaiting_redirect");
    for (pasted, refused) in [
        (
            "http://127.0.0.1:1/auth/callback?code=c&state=s",
            "redirect_elsewhere",
        ),
        (
            "http://127.0.0.1:39547/auth/callback?error=denied",
            "redirect_incomplete",
        ),
        ("approved", "redirect_not_an_address"),
    ] {
        let (status, body) = control
            .post(&format!("{opencode}/answer"), json!({ "answer": pasted }))
            .await;
        assert_eq!(
            (status, body["error"].clone()),
            (StatusCode::CONFLICT, json!(refused))
        );
    }
    let (status, body) = control
        .post(
            &format!("{opencode}/answer"),
            json!({ "answer": "http://127.0.0.1:39547/auth/callback?code=acme&state=acme" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["outcome"]["end"]["said"], "Connected to OpenAI");
    assert_eq!(body["outcome"]["state"], json!({ "kind": "signed_out" }));
}

/// A PNG header of `width` by `height` pixels.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

impl Control {
    /// Upload `bytes` to `session` as `name`.
    async fn upload(&self, session: &str, name: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
        let response = self
            .http
            // The names used here need no escaping in a query.
            .post(format!(
                "{}/sessions/{session}/attachments?name={name}",
                self.base
            ))
            .bearer_auth(TestGateway::CONTROL_TOKEN)
            .body(bytes)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.bytes().await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// An attachment's bytes and the headers they came with.
    async fn download(&self, session: &str, attachment: &str) -> reqwest::Response {
        self.http
            .get(format!(
                "{}/sessions/{session}/attachments/{attachment}",
                self.base
            ))
            .bearer_auth(TestGateway::CONTROL_TOKEN)
            .send()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn an_image_is_uploaded_read_removed_kept_across_a_restart_and_deleted_with_its_session() {
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
    let (status, body) = control
        .post(
            "/sessions",
            json!({ "harness": "codex", "dir": "/work/acme" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session = body["session"].as_str().unwrap().to_owned();
    // A draft takes images before its client starts.
    control
        .until_session(&session, |s| !s["stored"].is_null())
        .await;

    // Uploaded, read from its bytes, and kept in the session's folder.
    let (status, body) = control
        .upload(&session, "../Desktop/shot.png", png(64, 48))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let attachment = body["attachment"]["id"].as_str().unwrap().to_owned();
    let read = &body["attachment"]["read"];
    assert_eq!(read["name"], "shot.png");
    assert_eq!(read["kind"]["format"], "png");
    assert_eq!(
        read["kind"]["dimensions"],
        json!({ "width": 64, "height": 48 })
    );
    let file = dir
        .path()
        .join("attachments")
        .join(&session)
        .join(&attachment)
        .join("shot.png");
    assert_eq!(std::fs::read(&file).unwrap(), png(64, 48));
    let response = control.download(&session, &attachment).await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(headers["cache-control"], "no-store");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(response.bytes().await.unwrap().as_ref(), png(64, 48));

    // What is not an image Ren takes, or is too large, is refused.
    let (status, body) = control
        .upload(&session, "page.svg", b"<svg/>".to_vec())
        .await;
    assert_eq!(
        (status, body["error"].clone()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            json!("attachment_unrecognized")
        )
    );
    let mut large = png(10, 10);
    large.resize(5 * 1024 * 1024 + 1, 0);
    let (status, body) = control.upload(&session, "large.png", large).await;
    assert_eq!(
        (status, body["error"].clone()),
        (StatusCode::PAYLOAD_TOO_LARGE, json!("attachment_too_large"))
    );

    // Removed, it is gone.
    let (status, body) = control.upload(&session, "other.png", png(1, 1)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let other = body["attachment"]["id"].as_str().unwrap().to_owned();
    let removed = control
        .http
        .delete(format!(
            "{}/sessions/{session}/attachments/{other}",
            control.base
        ))
        .bearer_auth(TestGateway::CONTROL_TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::ACCEPTED);
    assert_eq!(
        control.download(&session, &other).await.status(),
        StatusCode::NOT_FOUND
    );
    assert!(
        !dir.path()
            .join("attachments")
            .join(&session)
            .join(&other)
            .exists()
    );

    // Sent with a prompt, it is listed as sent and stays.
    let (status, body) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "what is this", "attachments": [attachment] }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let started = control
        .until_session(&session, |s| !s["served"]["process"].is_null())
        .await;
    let process = started["served"]["process"].as_str().unwrap().to_owned();
    let (status, listed) = control
        .get(&format!("/sessions/{session}/attachments"))
        .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["attachments"][0]["id"], attachment.as_str());
    assert_eq!(listed["attachments"][0]["sent"], true);
    let kept = control
        .http
        .delete(format!(
            "{}/sessions/{session}/attachments/{attachment}",
            control.base
        ))
        .bearer_auth(TestGateway::CONTROL_TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(kept.status(), StatusCode::CONFLICT);
    let (status, body) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "", "attachments": ["00000000-0000-0000-0000-0000000000ff"] }),
        )
        .await;
    assert_eq!(
        (status, body["error"].clone()),
        (StatusCode::NOT_FOUND, json!("not_found"))
    );

    // Kept across a restart: the stored session is served again with it.
    let (status, _) = control
        .post(&format!("/sessions/{session}/stop"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    control
        .until_run(&process, |state| state["phase"] == "ended")
        .await;
    control
        .until_session(&session, |s| s["stored"]["runs"][0]["ended_at"].is_number())
        .await;
    gateway.stop().await;
    let gateway = TestGateway::with_runs(
        &config(&upstream),
        dir.path(),
        vec![PathBuf::from("/work")],
        ScriptedHost,
    )
    .await;
    let control = Control::new(&gateway);
    let response = control.download(&session, &attachment).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await.unwrap().as_ref(), png(64, 48));
    assert_eq!(
        control.download(&session, &other).await.status(),
        StatusCode::NOT_FOUND
    );
    let (_, listed) = control
        .get(&format!("/sessions/{session}/attachments"))
        .await;
    assert_eq!(listed["attachments"][0]["sent"], true, "{listed}");

    // Deleted with its session, folder and all.
    let (status, body) = control
        .post(&format!("/sessions/{session}/delete"), json!({}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert!(!dir.path().join("attachments").join(&session).exists());
}

/// Every request the CLI builds reaches a control route: the answer is the
/// control API's JSON (a result or one of its refusals), never the router's
/// own 404 or 405 for a path or method it does not serve. The bodies are
/// the routes' own types, so a body a route refuses is `invalid_request`.
#[tokio::test]
async fn every_control_request_reaches_its_route() {
    use ren_gateway::ControlRefusal;
    use ren_gateway::api::control::request::{
        AnswerRequest, BlockRequest, CancelRequest, ControlFeed, ControlMethod, ControlRequest,
        CreateRequest, LandWorkRequest, PauseRequest, PreviewWorkRequest, PromoteRequest,
        PromptRequest, SendWorkRequest, ShipWorkRequest, WorkChange,
    };
    use ren_harness::{Harness, drive::Decision};
    use ren_protocol::native::NativeSessionId;
    use ren_recorder::batch::{HistoryChange, HistorySubject};
    use ren_recorder::id::{CallId, RecordingId, RunId, SessionId};

    let upstream = mock(Router::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::with_runs(
        &config(&upstream),
        dir.path(),
        vec![PathBuf::from("/work")],
        ScriptedHost,
    )
    .await;
    let id = |n: u128| uuid::Uuid::from_u128(0x0190_f3a0_0000_7000_8000_0000_0000_0000 | n);
    let session = SessionId::restore(id(1));
    let native = NativeSessionId::new(id(2)).unwrap();
    let block = BlockRequest {
        duration_seconds: Some(60),
    };
    let requests = [
        ControlRequest::State,
        ControlRequest::Pause(PauseRequest {
            retry_after_seconds: 5,
        }),
        ControlRequest::Resume,
        ControlRequest::BlockSession {
            session: native,
            block: block.clone(),
        },
        ControlRequest::UnblockSession(native),
        ControlRequest::BlockUser {
            user: "nobody".parse().unwrap(),
            block,
        },
        ControlRequest::UnblockUser("nobody".parse().unwrap()),
        ControlRequest::Cancel {
            call: CallId::restore(id(3)),
            cancel: CancelRequest::default(),
        },
        ControlRequest::Backup,
        ControlRequest::Edit {
            subject: HistorySubject::Recording(RecordingId::restore(id(4))),
            change: HistoryChange::Archive,
        },
        ControlRequest::Edit {
            subject: HistorySubject::Run(RunId::restore(id(5))),
            change: HistoryChange::Unarchive,
        },
        ControlRequest::Edit {
            subject: HistorySubject::OperatorSession(session),
            change: HistoryChange::Delete,
        },
        ControlRequest::Runs,
        ControlRequest::Sessions,
        ControlRequest::Places,
        ControlRequest::Create(CreateRequest::new(
            Harness::Codex,
            Some("/elsewhere".into()),
        )),
        ControlRequest::Prompt {
            session,
            prompt: PromptRequest::text("hello".into()),
        },
        ControlRequest::Answer {
            session,
            approval: "approval-1".into(),
            answer: AnswerRequest {
                decision: Decision::Accept,
            },
        },
        ControlRequest::Interrupt(session),
        ControlRequest::Stop(session),
        ControlRequest::QueueWork {
            order: Box::new(
                serde_json::from_value(json!({
                    "page": { "path": "/", "route": "/", "title": "Overview", "width": 1440,
                              "height": 900, "theme": "light", "revision": null },
                    "marks": [{ "selector": "main", "text": "", "source": null, "width": 10,
                                "height": 10, "note": "Make it clearer" }],
                }))
                .unwrap(),
            ),
            project: None,
        },
        ControlRequest::Work {
            item: ren_recorder::id::WorkItemId::restore(id(6)),
            change: WorkChange::Send(SendWorkRequest { session }),
        },
        ControlRequest::Work {
            item: ren_recorder::id::WorkItemId::restore(id(6)),
            change: WorkChange::Land(LandWorkRequest {
                commit: "62e9e9e0000000000000000000000000000000ff".parse().unwrap(),
            }),
        },
        ControlRequest::Work {
            item: ren_recorder::id::WorkItemId::restore(id(6)),
            change: WorkChange::Ship(ShipWorkRequest {
                revision: "62e9e9e0000000000000000000000000000000ff".parse().unwrap(),
            }),
        },
        ControlRequest::Work {
            item: ren_recorder::id::WorkItemId::restore(id(6)),
            change: WorkChange::Dismiss,
        },
        ControlRequest::Work {
            item: ren_recorder::id::WorkItemId::restore(id(6)),
            change: WorkChange::Preview(PreviewWorkRequest { preview: None }),
        },
        ControlRequest::Promote(PromoteRequest {
            commit: "62e9e9e0000000000000000000000000000000ff".parse().unwrap(),
        }),
    ];
    let http = client();
    for request in &requests {
        let url = format!("{}/control/v1/{}", gateway.base(), request.path());
        let mut sending = match request.method() {
            ControlMethod::Get => http.get(url),
            ControlMethod::Post => http.post(url),
        }
        .bearer_auth(TestGateway::CONTROL_TOKEN);
        if let Some(body) = request.body().unwrap() {
            sending = sending
                .header("content-type", "application/json")
                .body(body);
        }
        let response = sending.send().await.unwrap();
        let status = response.status();
        let body: Value = serde_json::from_slice(&response.bytes().await.unwrap())
            .unwrap_or_else(|_| panic!("{request:?}: {status} without the API's JSON"));
        assert_eq!(body["api_version"], 1, "{request:?}: {body}");
        let refused = body["error"]
            .as_str()
            .map(|code| code.parse::<ControlRefusal>().unwrap());
        assert_ne!(
            refused,
            Some(ControlRefusal::InvalidRequest),
            "{request:?}: its body is not the route's"
        );
        if let Some(refusal) = refused {
            assert_eq!(status, refusal.status(), "{request:?}");
        }
    }
    for feed in [ControlFeed::Runs, ControlFeed::Sessions] {
        let response = http
            .get(format!("{}/control/v1/{}", gateway.base(), feed.path()))
            .bearer_auth(TestGateway::CONTROL_TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{feed:?}");
    }
    gateway.stop().await;
}

#[tokio::test]
async fn a_draft_moves_folder_and_a_clients_commands_are_read_without_a_session() {
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

    // What a client offers a prompt in a folder, before any session.
    let (status, body) = control
        .get("/runs/clients/codex/commands?dir=/work/acme")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["commands"], json!([]));
    let (status, body) = control.get("/runs/clients/codex/commands?dir=/etc").await;
    assert_eq!(
        (status, body["error"].clone()),
        (StatusCode::BAD_REQUEST, json!("invalid_directory"))
    );
    let (status, _) = control.get("/runs/clients/gemini/commands").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A draft moves to another folder; its first prompt fixes it there.
    let (status, body) = control
        .post(
            "/sessions",
            json!({ "harness": "codex", "dir": "/work/acme" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session = body["session"].as_str().unwrap().to_owned();
    let (status, body) = control
        .post(
            &format!("/sessions/{session}/folder"),
            json!({ "dir": "/work/other" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let moved = control
        .until_session(&session, |s| s["stored"]["workdir"] == "/work/other")
        .await;
    assert_eq!(moved["served"]["workdir"], "/work/other");
    assert!(moved["served"]["process"].is_null());
    let (status, _) = control
        .post(
            &format!("/sessions/{session}/prompt"),
            json!({ "text": "go" }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, body) = control
        .post(
            &format!("/sessions/{session}/folder"),
            json!({ "dir": "/work/acme" }),
        )
        .await;
    assert_eq!(
        (status, body["error"].clone()),
        (StatusCode::CONFLICT, json!("folder_fixed"))
    );
    gateway.stop().await;
}
