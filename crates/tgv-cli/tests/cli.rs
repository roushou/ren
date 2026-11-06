//! Public command/output contracts, using isolated state and synthetic traffic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly, and mock servers are spawned tasks"
)]

use serde_json::Value;
use std::process::{Command, Output};
use std::sync::Arc;
use tgv_daemon::{Daemon, config::Config, host::RandomIds};
use tgv_recorder::time::{ManualClock, UnixMillis};
use tgv_store::{Reader, Store, query::CallQuery};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tgv"))
        .args(args)
        .env("NO_COLOR", "1")
        .env("TERM", "xterm")
        .output()
        .unwrap()
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn version_names_the_tgv_program_and_its_revision() {
    let output = cli(&["--version"]);
    success(&output);
    let build = tgv_cli::Cli::build();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("tgv {} ({})\n", build.version(), build.revision())
    );
}

#[test]
fn command_structs_preserve_help_flags_and_dry_run_output() {
    let output = cli(&["--help"]);
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    assert_eq!(cli(&[]).status.code(), Some(2));
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("state");
    let output = cli(&[
        "run",
        "pi",
        "--dry-run",
        "--data-dir",
        data.to_str().unwrap(),
        "--gateway",
        "http://127.0.0.1:1",
        "--",
        "PRIVATE_PROMPT",
    ]);
    success(&output);
    let plan = json(&output);
    assert_eq!(plan["executable"], "pi");
    assert!(plan.get("result").is_none());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE_PROMPT"));
    assert!(output.stderr.is_empty());
    assert!(!data.exists());
    assert_eq!(cli(&["history", "--limit", "0"]).status.code(), Some(2));
}

#[tokio::test]
async fn history_keeps_direct_json_and_missing_ids_fail_on_stderr() {
    let temp = tempfile::tempdir().unwrap();
    let clock = ManualClock::at(UnixMillis::EPOCH);
    let (store, _) = Store::open(temp.path(), &clock, Arc::new(RandomIds))
        .await
        .unwrap();
    store.close().await.unwrap();
    for name in ["history", "sessions"] {
        let output = cli(&["--data-dir", temp.path().to_str().unwrap(), name]);
        success(&output);
        assert_eq!(json(&output), serde_json::json!([]));
        assert!(output.stderr.is_empty());
    }
    let output = cli(&[
        "--data-dir",
        temp.path().to_str().unwrap(),
        "inspect",
        "00000000-0000-4000-8000-000000000001",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("call, session, or body not found"));
}

#[tokio::test(flavor = "multi_thread")]
async fn recorded_requests_flow_through_lists_inspection_pagination_and_private_export() {
    use axum::{Router, response::IntoResponse};
    let payload = r#"{"type":"message","model":"fixture","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":3}}"#;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(move || async move {
                ([("content-type", "application/json")], payload).into_response()
            }),
        )
        .await
        .unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::new(
        Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            anthropic_base_url: format!("http://{address}"),
            ..Config::default()
        }
        .resolve()
        .unwrap(),
        temp.path(),
        tgv_cli::Cli::build(),
    );
    daemon.start().await.unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for _ in 0..2 {
        let response = client
            .post(format!(
                "http://{}/anthropic/v1/messages",
                daemon.address().unwrap()
            ))
            .header(
                "x-claude-code-session-id",
                "00000000-0000-4000-8000-000000000003",
            )
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), payload);
    }
    daemon.shutdown();
    daemon.wait().await.unwrap();
    upstream.abort();
    let data = temp.path().to_str().unwrap();
    let output = cli(&["--data-dir", data, "history"]);
    success(&output);
    let records = json(&output);
    let stored = Reader::open(temp.path(), 1)
        .await
        .unwrap()
        .calls(&CallQuery::default())
        .await
        .unwrap();
    assert_eq!(records, serde_json::to_value(stored.calls).unwrap());
    assert_eq!(records.as_array().unwrap().len(), 2);
    let id = records[0]["id"].as_str().unwrap();
    let output = cli(&["--data-dir", data, "inspect", id]);
    success(&output);
    let detail = json(&output);
    assert_eq!(detail["call"], records[0]);
    assert_eq!(detail["attempts"][0]["inspection"]["outcome"], "completed");
    let output = cli(&["--data-dir", data, "sessions"]);
    let sessions = json(&output);
    let session = sessions[0]["id"].as_str().unwrap();
    let output = cli(&["--data-dir", data, "timeline", session, "--limit", "1"]);
    success(&output);
    let page = json(&output);
    let after = page["next"].as_str().unwrap();
    let output = cli(&[
        "--data-dir",
        data,
        "timeline",
        session,
        "--limit",
        "1",
        "--after",
        after,
    ]);
    success(&output);
    let last = json(&output);
    assert_ne!(last["calls"][0]["id"], page["calls"][0]["id"]);
    assert_eq!(last["next"], Value::Null);
    let export = temp.path().join("response.bin");
    let output = cli(&[
        "--data-dir",
        data,
        "inspect",
        id,
        "--body",
        "response",
        "--output",
        export.to_str().unwrap(),
    ]);
    success(&output);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(std::fs::read(&export).unwrap(), payload.as_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&export).unwrap().permissions().mode() & 0o077,
            0
        );
    }
    let output = cli(&[
        "--data-dir",
        data,
        "inspect",
        id,
        "--body",
        "response",
        "--output",
        export.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(std::fs::read(export).unwrap(), payload.as_bytes());
}
