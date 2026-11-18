//! The read API's JSON, pinned: each body reads back into the gateway's
//! types and serializes to the same JSON, and labels or findings a newer
//! daemon sends are kept rather than failing the read.

use super::activity::TurnActivity;
use super::calls::CallPage;
use super::recordings::RecordingView;
use super::report::Finding;
use serde_json::{Value, json};

const TURNS: &str = include_str!("../../../tests/fixtures/turn-activity.json");

/// What the read API sends for a call listing, every optional field
/// present, and some labels this binary does not know.
fn call_page() -> Value {
    json!({
        "calls": [{
            "id": "00000000-0000-0000-0000-000000000001",
            "channel": "00000000-0000-0000-0000-000000000002",
            "started_at": 1000,
            "ended_at": 1500,
            "provider": "anthropic",
            "wire": "a_newer_wire",
            "endpoint": "messages",
            "method": "post",
            "config_revision": "r1",
            "recording": "00000000-0000-0000-0000-000000000003",
            "turn": "00000000-0000-0000-0000-000000000004",
            "attribution": {
                "hint": {
                    "source": "claude_code_headers",
                    "session": "s",
                    "turn": "t",
                    "thread": null
                },
                "issues": []
            },
            "caller": { "kind": "user_token", "user": "acme", "login": null },
            "state": "a_newer_state",
            "terminal": "completed",
            "status": 200,
            "model": "model",
            "outcome": "completed",
            "stop_reason": "end_turn",
            "tool_names": ["Read"],
            "usage": {
                "accounting": "excludes_cache",
                "input_tokens": 10,
                "output_tokens": 20,
                "cache_read_tokens": 30,
                "cache_creation_tokens": 40,
                "reasoning_tokens": null
            },
            "capture_complete": true,
            "request": { "observed_bytes": 100, "stored_bytes": 100, "ending": "ended" },
            "response": { "observed_bytes": null, "stored_bytes": 0, "ending": "open" },
            "rejection": null,
            "operator_revision": 3,
            "plan": { "candidates": ["a"] },
            "purpose": "work"
        }],
        "next": "1000:00000000-0000-0000-0000-000000000001"
    })
}

#[test]
fn a_call_page_reads_back_as_sent() {
    let page: CallPage = serde_json::from_value(call_page()).unwrap();
    assert_eq!(page.calls[0].wire.known(), None);
    assert_eq!(serde_json::to_value(&page).unwrap(), call_page());
}

#[test]
fn a_recording_reads_back_as_sent() {
    let sent = json!({
        "id": "00000000-0000-0000-0000-000000000003",
        "provider": "openai",
        "source": "a_newer_source",
        "native_id": "native",
        "first_seen": 1000,
        "last_seen": 2000,
        "archived_at": 1500,
        "call_count": 4,
        "turn_count": 2,
        "error_count": 1,
        "tokens": { "input": 10, "output": null, "cache_read": 5 }
    });
    let recording: RecordingView = serde_json::from_value(sent.clone()).unwrap();
    assert_eq!(recording.source.known(), None);
    assert_eq!(serde_json::to_value(&recording).unwrap(), sent);
}

#[test]
fn turns_read_back_as_sent() {
    let turns: Vec<TurnActivity> = serde_json::from_str(TURNS).unwrap();
    assert_eq!(
        serde_json::to_string_pretty(&turns).unwrap().trim_end(),
        TURNS.trim_end()
    );
}

#[test]
fn turns_keep_labels_and_findings_they_do_not_know() {
    let mut sent: Vec<Value> = serde_json::from_str(TURNS).unwrap();
    let turn = &mut sent[0];
    turn["client"] = json!("a_newer_client");
    turn["findings"] = json!([{ "kind": "a_newer_finding", "call": "c", "size": 3 }]);
    let call = &mut turn["calls"][0];
    call["limitations"] = json!(["a_newer_limitation"]);
    call["tool_calls"][0]["effects"][0]["kind"] = json!("a_newer_effect");
    call["tool_calls"][0]["result"]["status"] = json!("a_newer_status");

    let turns: Vec<TurnActivity> = serde_json::from_value(json!(sent)).unwrap();
    let turn = &turns[0];
    assert_eq!(turn.client.as_ref().unwrap().label(), "a_newer_client");
    let Finding::Unknown(finding) = &turn.findings[0] else {
        panic!("{:?}", turn.findings[0]);
    };
    assert_eq!(finding["kind"], json!("a_newer_finding"));
    assert_eq!(finding["size"], json!(3));
    let tool = &turn.calls[0].tool_calls[0];
    assert_eq!(tool.effects[0].kind.known(), None);
    assert_eq!(tool.result.as_ref().unwrap().status.known(), None);
    assert_eq!(turn.calls[0].limitations[0].known(), None);
    assert_eq!(serde_json::to_value(&turns).unwrap(), json!(sent));
}
