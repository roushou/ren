use super::*;
use serde_json::json;
use std::io::Write;

fn reader(protocol: Protocol) -> ActivityReader {
    ActivityReader::new(protocol, ActivityLimits::default())
}

fn body(bytes: &[u8]) -> CapturedBody<'_> {
    CapturedBody {
        bytes,
        extent: BodyExtent::Complete,
        encoding: None,
        messages: None,
    }
}

fn sse(events: &[Value]) -> Vec<u8> {
    let mut stream = Vec::new();
    for event in events {
        let name = event["type"].as_str().unwrap();
        stream.extend_from_slice(format!("event: {name}\ndata: {event}\n\n").as_bytes());
    }
    stream
}

#[test]
fn messages_requests_yield_the_newest_user_input() {
    let first = json!({
        "model": "m",
        "messages": [{ "role": "user", "content": "Fix the failing test" }],
    })
    .to_string();
    let activity = reader(Protocol::Messages).request(body(first.as_bytes()));
    assert_eq!(activity.user_text, ["Fix the failing test"]);
    assert!(activity.tool_results.is_empty());
    assert!(activity.limitations.is_empty());

    let later = json!({
        "messages": [
            { "role": "user", "content": [{ "type": "text", "text": "Fix the failing test" }] },
            { "role": "assistant", "content": [{ "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {} }] },
            { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_1", "is_error": true, "content": [{ "type": "text", "text": "exit 1" }] },
                { "type": "image", "source": {} },
                { "type": "text", "text": "<system-reminder>x</system-reminder>" },
            ] },
            { "role": "system", "content": [{ "type": "text", "text": "context, not a prompt" }] },
        ],
    })
    .to_string();
    let activity = reader(Protocol::Messages).request(body(later.as_bytes()));
    assert_eq!(activity.user_text, ["<system-reminder>x</system-reminder>"]);
    assert!(activity.limitations.is_empty());
    assert_eq!(
        activity.tool_results,
        [ToolResult {
            id: "toolu_1".to_owned(),
            status: ResultStatus::Failed,
            output: "exit 1".to_owned(),
        }]
    );
}

#[test]
fn messages_streams_assemble_tool_input_from_deltas() {
    let stream = sse(&[
        json!({ "type": "message_start", "message": {} }),
        json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
        json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "Reading " } }),
        json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "it." } }),
        json!({ "type": "content_block_stop", "index": 0 }),
        json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "tool_use", "id": "toolu_2", "name": "Read", "input": {} } }),
        json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{\"file_pa" } }),
        json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": "th\":\"src/lib.rs\"}" } }),
        json!({ "type": "content_block_stop", "index": 1 }),
        json!({ "type": "content_block_start", "index": 2, "content_block": { "type": "tool_use", "id": "toolu_3", "name": "Bash", "input": {} } }),
        json!({ "type": "content_block_delta", "index": 2, "delta": { "type": "input_json_delta", "partial_json": "{\"comm" } }),
    ]);
    // Compressed as the upstream sent it.
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gzip.write_all(&stream).unwrap();
    let compressed = gzip.finish().unwrap();
    let activity = reader(Protocol::Messages).response(CapturedBody {
        encoding: Some(ResponseEncoding::Gzip),
        ..body(&compressed)
    });
    assert_eq!(activity.reply, "Reading it.");
    assert_eq!(activity.tool_uses.len(), 2);
    assert_eq!(activity.tool_uses[0].name, "Read");
    assert_eq!(activity.tool_uses[0].id.as_deref(), Some("toolu_2"));
    assert_eq!(
        activity.tool_uses[0].input,
        ToolInput::Json(json!({ "file_path": "src/lib.rs" }))
    );
    // The cut-off call keeps what was streamed, and says so.
    assert_eq!(
        activity.tool_uses[1].input,
        ToolInput::Text("{\"comm".to_owned())
    );
    assert_eq!(
        activity.limitations,
        [ActivityLimitation::UnfinishedToolCall]
    );
}

#[test]
fn messages_documents_read_content_blocks() {
    let document = json!({
        "type": "message",
        "content": [
            { "type": "thinking", "thinking": "..." },
            { "type": "text", "text": "Done." },
            { "type": "tool_use", "id": "toolu_4", "name": "Edit", "input": { "file_path": "a.rs" } },
        ],
    })
    .to_string();
    let activity = reader(Protocol::Messages).response(body(document.as_bytes()));
    assert_eq!(activity.reply, "Done.");
    assert_eq!(activity.tool_uses[0].kind, "tool_use");
    assert!(activity.limitations.is_empty());
}

#[test]
fn responses_requests_read_items_after_the_models_last_output() {
    let request = json!({
        "model": "gpt",
        "input": [
            { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "Add a test" }] },
            { "type": "reasoning", "summary": [] },
            { "type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{}" },
            { "type": "function_call_output", "call_id": "call_1", "output": "Exit code: 0" },
            { "type": "custom_tool_call_output", "call_id": "call_2", "output": "Done!" },
        ],
    })
    .to_string();
    // Codex may compress its request with zstd; no encoding is recorded.
    let compressed = zstd::encode_all(request.as_bytes(), 1).unwrap();
    let activity = reader(Protocol::Responses).request(body(&compressed));
    assert!(activity.user_text.is_empty());
    let ids: Vec<_> = activity
        .tool_results
        .iter()
        .map(|result| result.id.as_str())
        .collect();
    assert_eq!(ids, ["call_1", "call_2"]);
    assert_eq!(activity.tool_results[0].status, ResultStatus::Unreported);
    assert_eq!(activity.tool_results[0].output, "Exit code: 0");

    // A WebSocket message continuing a response holds only the new input.
    let frame = json!({
        "type": "response.create",
        "input": [{ "role": "user", "content": "Now run it" }],
    })
    .to_string();
    let range = 0..frame.len();
    let activity = reader(Protocol::Responses).request(CapturedBody {
        messages: Some(std::slice::from_ref(&range)),
        ..body(frame.as_bytes())
    });
    assert_eq!(activity.user_text, ["Now run it"]);
}

#[test]
fn responses_streams_read_done_items_or_the_terminal_response() {
    let function = json!({ "type": "function_call", "call_id": "call_9", "name": "shell", "arguments": "{\"command\":[\"cargo\",\"test\"]}" });
    let patch = json!({ "type": "custom_tool_call", "call_id": "call_10", "name": "apply_patch", "input": "*** Begin Patch" });
    let message = json!({ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "Running tests." }] });
    let stream = sse(&[
        json!({ "type": "response.created", "response": {} }),
        json!({ "type": "response.output_item.done", "item": message }),
        json!({ "type": "response.output_item.done", "item": function }),
        json!({ "type": "response.output_item.done", "item": patch }),
        json!({ "type": "response.completed", "response": { "output": [] } }),
    ]);
    let activity = reader(Protocol::Responses).response(body(&stream));
    assert_eq!(activity.reply, "Running tests.");
    assert_eq!(activity.tool_uses.len(), 2);
    assert_eq!(
        activity.tool_uses[0].input,
        ToolInput::Json(json!({ "command": ["cargo", "test"] }))
    );
    assert_eq!(
        activity.tool_uses[1].input,
        ToolInput::Text("*** Begin Patch".to_owned())
    );

    // Over WebSocket each event is a message; here only the terminal event
    // was captured with output.
    let terminal =
        json!({ "type": "response.completed", "response": { "output": [function] } }).to_string();
    let range = 0..terminal.len();
    let activity = reader(Protocol::Responses).response(CapturedBody {
        messages: Some(std::slice::from_ref(&range)),
        ..body(terminal.as_bytes())
    });
    assert_eq!(activity.tool_uses[0].id.as_deref(), Some("call_9"));
}

#[test]
fn unreadable_bodies_are_limitations_never_facts() {
    let reader = reader(Protocol::Messages);
    let not_json = reader.request(body(b"{not json"));
    assert_eq!(not_json.limitations, [ActivityLimitation::NotJson]);
    let shape = reader.request(body(b"{\"messages\": 3}"));
    assert_eq!(shape.limitations, [ActivityLimitation::UnexpectedShape]);
    let cut = reader.request(CapturedBody {
        extent: BodyExtent::Truncated,
        ..body(b"{\"messages\": [")
    });
    assert_eq!(
        cut.limitations,
        [ActivityLimitation::Truncated, ActivityLimitation::NotJson]
    );
    let gap = reader.response(CapturedBody {
        extent: BodyExtent::Gap,
        encoding: Some(ResponseEncoding::Unsupported),
        ..body(b"x")
    });
    assert_eq!(
        gap.limitations,
        [
            ActivityLimitation::CaptureGap,
            ActivityLimitation::UnsupportedEncoding
        ]
    );
    let corrupt = reader.response(CapturedBody {
        encoding: Some(ResponseEncoding::Gzip),
        ..body(b"\x1f\x8b\x08garbage")
    });
    assert!(
        corrupt
            .limitations
            .contains(&ActivityLimitation::DecodingError)
    );
    assert!(corrupt.tool_uses.is_empty());
}

#[test]
fn both_readers_cap_tool_calls_alike_and_keep_later_text() {
    let limited = |protocol| {
        ActivityReader::new(
            protocol,
            ActivityLimits {
                max_tool_uses: 1,
                ..ActivityLimits::default()
            },
        )
    };
    let messages = json!({
        "type": "message",
        "content": [
            { "type": "text", "text": "" },
            { "type": "tool_use", "id": "a", "name": "Read", "input": {} },
            { "type": "tool_use", "id": "b", "name": "Read", "input": {} },
            { "type": "tool_use", "id": "c", "name": "Read", "input": {} },
            { "type": "text", "text": "After." },
        ],
    })
    .to_string();
    let responses = json!({
        "object": "response",
        "output": [
            { "type": "message", "content": [{ "type": "output_text", "text": "" }] },
            { "type": "function_call", "call_id": "a", "name": "shell", "arguments": "{}" },
            { "type": "function_call", "call_id": "b", "name": "shell", "arguments": "{}" },
            { "type": "function_call", "call_id": "c", "name": "shell", "arguments": "{}" },
            { "type": "message", "content": [{ "type": "output_text", "text": "After." }] },
        ],
    })
    .to_string();
    for activity in [
        limited(Protocol::Messages).response(body(messages.as_bytes())),
        limited(Protocol::Responses).response(body(responses.as_bytes())),
    ] {
        assert_eq!(activity.tool_uses.len(), 1);
        assert_eq!(activity.limitations, [ActivityLimitation::TooManyToolCalls]);
        // Empty text parts leave no blank lines; text after the cap is kept.
        assert_eq!(activity.reply, "After.");
    }
}
