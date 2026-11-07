//! `tgv mcp` as an MCP client runs it: the built binary over stdio, reading
//! traffic a real daemon recorded, through the daemon's read API.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly, and mock servers are spawned tasks"
)]

use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};
use tgv_daemon::{Daemon, config::Config};

const META: &str = r#"{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}"#;

/// Runs `tgv mcp --gateway <gateway>` with `messages` on stdin, one per
/// line, and returns every line it wrote to stdout, parsed.
fn session(home: &std::path::Path, gateway: &str, messages: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tgv"))
        .args(["mcp", "--gateway", gateway])
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("TGV_GATEWAY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for message in messages {
        writeln!(stdin, "{message}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn modern(id: u64, method: &str, params: Value) -> Value {
    let mut params = params;
    params["_meta"] = serde_json::from_str(META).unwrap();
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// A tool result's text, parsed as the JSON it carries.
fn data(reply: &Value) -> Value {
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_read_recorded_traffic_over_stdio_in_both_protocol_eras() {
    use axum::{Router, response::IntoResponse};
    let payload = r#"{"type":"message","model":"fixture-model","content":[{"type":"text","text":"hello from the fixture"}],"stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":3}}"#;
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
    let home = temp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let mut daemon = Daemon::new(
        Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            anthropic_base_url: format!("http://{address}"),
            ..Config::default()
        }
        .resolve()
        .unwrap(),
        temp.path().join("state"),
        tgv_cli::Cli::build(),
    );
    daemon.start().await.unwrap();
    let gateway = format!("http://{}", daemon.address().unwrap());
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .post(format!("{gateway}/anthropic/v1/messages"))
        .header(
            "x-claude-code-session-id",
            "00000000-0000-4000-8000-000000000004",
        )
        .body(r#"{"model":"fixture-model","messages":[]}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), payload);

    // Capture commits shortly after the response.
    let search = modern(
        3,
        "tools/call",
        json!({ "name": "search_calls", "arguments": { "client": "claude_code", "limit": 5 } }),
    );
    let mut calls = Value::Null;
    for _ in 0..100 {
        calls = data(&session(&home, &gateway, std::slice::from_ref(&search))[0]);
        if calls["calls"].as_array().unwrap().len() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(calls["calls"].as_array().unwrap().len(), 1, "{calls}");
    let call = calls["calls"][0]["id"].as_str().unwrap().to_owned();

    // A modern client: no handshake, every request carries its version.
    let replies = session(
        &home,
        &gateway,
        &[
            modern(1, "server/discover", json!({})),
            json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 0 } }),
            modern(2, "tools/list", json!({})),
            modern(
                4,
                "tools/call",
                json!({ "name": "get_body", "arguments": { "call_id": call, "direction": "response" } }),
            ),
            modern(
                5,
                "tools/call",
                json!({ "name": "get_call", "arguments": { "call_id": call } }),
            ),
            modern(6, "tools/call", json!({ "name": "usage", "arguments": {} })),
            modern(
                7,
                "tools/call",
                json!({ "name": "get_call", "arguments": { "call_id": "00000000-0000-4000-8000-00000000ffff" } }),
            ),
            modern(
                8,
                "tools/call",
                json!({ "name": "list_sessions", "arguments": { "limit": 0 } }),
            ),
        ],
    );
    // One reply per request, in order; the notification gets none.
    let ids: Vec<_> = replies.iter().map(|reply| reply["id"].clone()).collect();
    assert_eq!(ids, [1, 2, 4, 5, 6, 7, 8]);
    assert_eq!(replies[0]["result"]["supportedVersions"][0], "2026-07-28");
    assert_eq!(
        replies[0]["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
        tgv_cli::Cli::build().version()
    );
    assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 9);
    let body = data(&replies[2]);
    assert_eq!(body["extent"], "complete");
    assert_eq!(body["text"], payload);
    assert_eq!(data(&replies[3])["call"]["id"], call.as_str());
    assert_eq!(data(&replies[4])["totals"]["calls"], 1);
    assert_eq!(replies[5]["result"]["isError"], true);
    assert!(
        replies[5]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no such record")
    );
    assert_eq!(replies[6]["result"]["isError"], true);

    // A legacy client: initialize once, then plain requests.
    let replies = session(
        &home,
        &gateway,
        &[
            json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "fixture", "version": "1" } } }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "list_sessions", "arguments": {} } }),
        ],
    );
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    let sessions = data(&replies[1]);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert!(replies[1]["result"].get("resultType").is_none());

    // An unreachable gateway is a tool error the model can read, not a
    // failed server.
    let replies = session(
        &home,
        "http://127.0.0.1:1",
        &[modern(
            1,
            "tools/call",
            json!({ "name": "usage", "arguments": {} }),
        )],
    );
    assert_eq!(replies[0]["result"]["isError"], true);
    assert!(
        replies[0]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("cannot reach the gateway")
    );

    daemon.shutdown();
    daemon.wait().await.unwrap();
    upstream.abort();
}

/// Runs the built `tgv` with an isolated HOME and returns its stdout as JSON.
fn cli_json(home: &std::path::Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_tgv"))
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("TGV_GATEWAY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turns_activity_reads_the_same_locally_through_a_gateway_and_over_mcp() {
    use axum::{Router, response::IntoResponse};
    use std::sync::atomic::{AtomicUsize, Ordering};
    // The first response streams a Bash tool call; the second answers in text.
    let first = [
        r#"{"type":"message_start","message":{"model":"fixture"}}"#,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"Bash","input":{}}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"cargo test\"}"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}"#,
        r#"{"type":"message_stop"}"#,
    ]
    .iter()
    .map(|event| {
        let name = serde_json::from_str::<Value>(event).unwrap()["type"]
            .as_str()
            .unwrap()
            .to_owned();
        format!("event: {name}\ndata: {event}\n\n")
    })
    .collect::<String>();
    let second = r#"{"type":"message","model":"fixture","content":[{"type":"text","text":"Tests fail in parser.rs."}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":4}}"#;
    let count = std::sync::Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().fallback(move || {
                let count = count.clone();
                let first = first.clone();
                async move {
                    if count.fetch_add(1, Ordering::SeqCst) == 0 {
                        ([("content-type", "text/event-stream")], first).into_response()
                    } else {
                        ([("content-type", "application/json")], second).into_response()
                    }
                }
            }),
        )
        .await
        .unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let state = temp.path().join("state");
    let mut daemon = Daemon::new(
        Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            anthropic_base_url: format!("http://{address}"),
            ..Config::default()
        }
        .resolve()
        .unwrap(),
        &state,
        tgv_cli::Cli::build(),
    );
    daemon.start().await.unwrap();
    let gateway = format!("http://{}", daemon.address().unwrap());
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let requests = [
        json!({ "model": "fixture", "messages": [
            { "role": "user", "content": [
                { "type": "text", "text": "<system-reminder>context</system-reminder>" },
                { "type": "text", "text": "Why do the tests fail?" },
            ] },
        ] }),
        json!({ "model": "fixture", "messages": [
            { "role": "user", "content": "Why do the tests fail?" },
            { "role": "assistant", "content": [{ "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "command": "cargo test" } }] },
            { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "is_error": true, "content": "test parser ... FAILED" }] },
        ] }),
    ];
    for request in requests {
        let response = client
            .post(format!("{gateway}/anthropic/v1/messages"))
            .header(
                "x-claude-code-session-id",
                "00000000-0000-4000-8000-000000000004",
            )
            .header(
                "x-claude-code-prompt-id",
                "00000000-0000-4000-8000-000000000005",
            )
            .body(request.to_string())
            .send()
            .await
            .unwrap();
        response.bytes().await.unwrap();
    }
    let data_dir = state.to_str().unwrap();
    let mut sessions = Value::Null;
    for _ in 0..100 {
        sessions = cli_json(&home, &["--data-dir", data_dir, "sessions"]);
        if sessions[0]["call_count"] == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let session_id = sessions[0]["id"].as_str().unwrap().to_owned();

    // Locally and through the gateway, the same JSON.
    let turns = cli_json(&home, &["--data-dir", data_dir, "turns", &session_id]);
    assert_eq!(
        cli_json(&home, &["turns", &session_id, "--gateway", &gateway]),
        turns
    );
    assert_eq!(turns.as_array().unwrap().len(), 1);
    assert_eq!(turns[0]["call_count"], 2);
    assert_eq!(turns[0]["prompt"]["text"], "Why do the tests fail?");
    let turn_id = turns[0]["turn"].as_str().unwrap().to_owned();
    let turn = cli_json(&home, &["--data-dir", data_dir, "turn", &turn_id]);
    assert_eq!(
        cli_json(&home, &["turn", &turn_id, "--gateway", &gateway]),
        turn
    );
    assert_eq!(turn["client"], "claude_code");
    assert_eq!(turn["commands"], json!(["cargo test"]));
    assert_eq!(turn["failed_tool_calls"], 1);
    let bash = &turn["calls"][0]["tool_calls"][0];
    assert_eq!(bash["name"], "Bash");
    assert_eq!(
        bash["effects"],
        json!([{ "kind": "run_command", "target": "cargo test" }])
    );
    assert_eq!(bash["result"]["status"], "failed");
    assert_eq!(bash["result"]["output"]["text"], "test parser ... FAILED");
    assert_eq!(
        turn["calls"][1]["reply"]["text"],
        "Tests fail in parser.rs."
    );
    let call = turn["calls"][1]["call"].as_str().unwrap().to_owned();
    let activity = cli_json(&home, &["--data-dir", data_dir, "activity", &call]);
    assert_eq!(activity["reply"]["text"], "Tests fail in parser.rs.");

    // An agent asks the same over MCP.
    let replies = session(
        &home,
        &gateway,
        &[
            modern(
                1,
                "tools/call",
                json!({ "name": "list_turns", "arguments": { "session_id": session_id } }),
            ),
            modern(
                2,
                "tools/call",
                json!({ "name": "get_turn", "arguments": { "turn_id": turn_id } }),
            ),
            modern(
                3,
                "tools/call",
                json!({ "name": "get_call_activity", "arguments": { "call_id": call } }),
            ),
        ],
    );
    assert_eq!(data(&replies[0]), turns);
    let mut over_mcp = data(&replies[1]);
    over_mcp.as_object_mut().unwrap().remove("api_version");
    assert_eq!(over_mcp, turn);
    assert_eq!(data(&replies[2])["reply"], activity["reply"]);

    daemon.shutdown();
    daemon.wait().await.unwrap();
    upstream.abort();
}
