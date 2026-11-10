//! The read API's JSON, pinned for the UI. Each read's answer to a fixed,
//! synthetic history is compared with `ui/tests/fixtures/api/<read>.json`,
//! and the UI's decoder tests parse those same files, so a change on either
//! side that breaks the contract fails a test. A test binary of its own, so
//! its gateway's sequential IDs are the same on every run.
//!
//! After an intended change, regenerate with `REN_UPDATE_FIXTURES=1`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly; mock servers spawn tasks"
)]

mod support;

use axum::{Router, body::Body, http::Response, routing::post};
use std::path::PathBuf;
use support::{TestGateway, client, config, mock};

const SESSION: &str = "00000000-0000-4000-8000-0000000000aa";
const PROMPT: &str = "00000000-0000-4000-8000-0000000000bb";

/// A streamed Messages response: a reply and a tool call.
fn stream() -> String {
    let event = |name: &str, data: serde_json::Value| format!("event: {name}\ndata: {data}\n\n");
    [
        event("message_start", serde_json::json!({ "type": "message_start", "message": { "model": "claude-test", "usage": { "input_tokens": 12, "cache_read_input_tokens": 3000, "cache_creation_input_tokens": 40 } } })),
        event("content_block_start", serde_json::json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } })),
        event("content_block_delta", serde_json::json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "Running the parser tests." } })),
        event("content_block_stop", serde_json::json!({ "type": "content_block_stop", "index": 0 })),
        event("content_block_start", serde_json::json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {} } })),
        event("content_block_delta", serde_json::json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{\"command\":\"cargo test parser\"}" } })),
        event("content_block_stop", serde_json::json!({ "type": "content_block_stop", "index": 1 })),
        event("message_delta", serde_json::json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": { "output_tokens": 30 } })),
        event("message_stop", serde_json::json!({ "type": "message_stop" })),
    ]
    .concat()
}

/// The configuration revision hashes the mock upstream's port, which
/// differs per run: it is pinned as a placeholder.
fn normalized(mut value: serde_json::Value) -> serde_json::Value {
    match &mut value {
        serde_json::Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                *field = if key == "config_revision" && field.is_string() {
                    serde_json::Value::String("<config revision>".to_owned())
                } else {
                    normalized(field.take())
                };
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                *item = normalized(item.take());
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
    value
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/tests/fixtures/api")
}

#[tokio::test]
async fn the_read_api_answers_as_the_ui_fixtures_pin() {
    let upstream = mock(Router::new().route(
        "/v1/messages",
        post(|| async {
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from(stream()))
                .unwrap()
        }),
    ))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let key = format!("sk-ant-{}a1b2", "api03xY9z".repeat(5));
    let requests = [
        serde_json::json!({ "model": "claude-test", "stream": true, "messages": [
            { "role": "user", "content": "Fix the parser" },
        ] }),
        serde_json::json!({ "model": "claude-test", "stream": true, "messages": [
            { "role": "user", "content": "Fix the parser" },
            { "role": "assistant", "content": [{ "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {} }] },
            { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "is_error": true, "content": format!("test parser ... FAILED\nK={key}") }] },
        ] }),
    ];
    let base = gateway.base();
    let read = |path: String| {
        let url = format!("{base}{path}");
        async move {
            let response = client().get(&url).send().await.unwrap();
            assert!(response.status().is_success(), "{url}");
            serde_json::from_slice::<serde_json::Value>(&response.bytes().await.unwrap()).unwrap()
        }
    };
    // One call at a time, each recorded before the next starts: the store
    // takes session and turn IDs as it commits, from the same sequence.
    for (sent, request) in requests.iter().enumerate() {
        let response = client()
            .post(format!("{}/anthropic/v1/messages", gateway.base()))
            .header("x-claude-code-session-id", SESSION)
            .header("x-claude-code-prompt-id", PROMPT)
            .body(request.to_string())
            .send()
            .await
            .unwrap();
        response.text().await.unwrap();
        for _ in 0..200 {
            let recorded = read("/api/calls".to_owned()).await;
            if recorded["calls"].as_array().is_some_and(|calls| {
                calls.len() == sent + 1 && calls.iter().all(|call| call["state"] == "finished")
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    // Digests are written as each call finishes; wait for both.
    let mut calls = serde_json::Value::Null;
    for _ in 0..200 {
        calls = read("/api/calls".to_owned()).await;
        let finished = calls["calls"].as_array().is_some_and(|calls| {
            calls.len() == 2 && calls.iter().all(|call| call["state"] == "finished")
        });
        let searchable = read("/api/search?q=parser".to_owned()).await["indexed_calls"] == 2;
        if finished && searchable {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let call = calls["calls"][0]["id"].as_str().unwrap().to_owned();
    let session = calls["calls"][0]["session"].as_str().unwrap().to_owned();
    let turn = calls["calls"][0]["turn"].as_str().unwrap().to_owned();
    let answers = [
        ("calls", calls.clone()),
        ("call", read(format!("/api/calls/{call}")).await),
        (
            "call_activity",
            read(format!("/api/calls/{call}/activity")).await,
        ),
        ("sessions", read("/api/sessions".to_owned()).await),
        (
            "session_calls",
            read(format!("/api/sessions/{session}/calls")).await,
        ),
        (
            "turns",
            read(format!("/api/sessions/{session}/turns")).await,
        ),
        ("turn", read(format!("/api/turns/{turn}")).await),
        (
            "report",
            read(format!("/api/sessions/{session}/report")).await,
        ),
        ("usage", read("/api/usage".to_owned()).await),
        ("search", read("/api/search?q=parser".to_owned()).await),
        ("exposures", read("/api/exposures".to_owned()).await),
    ];
    gateway.stop().await;

    let update = std::env::var_os("REN_UPDATE_FIXTURES").is_some();
    let mut changed = Vec::new();
    for (name, answer) in answers {
        let path = fixtures().join(format!("{name}.json"));
        let answer = normalized(answer);
        if update {
            std::fs::create_dir_all(fixtures()).unwrap();
            let text = format!("{}\n", serde_json::to_string_pretty(&answer).unwrap());
            std::fs::write(&path, text).unwrap();
        } else {
            // Compared as JSON values, so the UI's formatter may lay the
            // files out as it likes.
            let pinned = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            if pinned.as_ref() != Some(&answer) {
                changed.push(name);
            }
        }
    }
    assert!(
        changed.is_empty(),
        "the read API's JSON changed for {changed:?}; if intended, run with REN_UPDATE_FIXTURES=1 and update the UI's decoders"
    );
}
