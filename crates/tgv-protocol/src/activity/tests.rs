use super::*;
use crate::encoding::ResponseFormat;
use serde_json::json;
use std::io::Write;

use crate::CallInspector;
use crate::inspect::pipeline::{InspectionLimits, ResponseReading};

/// A request, gathered whole and read.
fn request(protocol: Protocol, bytes: &[u8]) -> RequestActivity {
    let mut reader = RequestReader::new(protocol, ActivityLimits::default());
    reader.feed(bytes);
    reader.finish(BodyEnd::Complete)
}

/// A response read by the inspection pipeline: one document, a stream, or
/// WebSocket messages (each fed as one announced message).
fn read(
    protocol: Protocol,
    format: ResponseFormat,
    encoding: ResponseEncoding,
    limits: ActivityLimits,
    messages: &[&[u8]],
) -> ResponseReading {
    let mut inspector = CallInspector::new(
        protocol,
        format,
        encoding,
        InspectionLimits {
            activity: limits,
            ..InspectionLimits::default()
        },
    );
    for message in messages {
        if format == ResponseFormat::WebSocketMessages {
            inspector.message(u64::try_from(message.len()).unwrap());
        }
        inspector.feed(message);
    }
    inspector.finish(BodyEnd::Complete)
}

fn response(protocol: Protocol, format: ResponseFormat, bytes: &[u8]) -> ResponseActivity {
    read(
        protocol,
        format,
        ResponseEncoding::Identity,
        ActivityLimits::default(),
        &[bytes],
    )
    .activity
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
    let activity = request(Protocol::Messages, first.as_bytes());
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
    let activity = request(Protocol::Messages, later.as_bytes());
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
fn requests_say_how_much_output_they_allow() {
    let messages = request(
        Protocol::Messages,
        json!({ "max_tokens": 1, "messages": [{ "role": "user", "content": "quota" }] })
            .to_string()
            .as_bytes(),
    );
    assert_eq!(messages.max_output_tokens, Some(1));
    let responses = request(
        Protocol::Responses,
        json!({ "max_output_tokens": 64, "input": "hi" })
            .to_string()
            .as_bytes(),
    );
    assert_eq!(responses.max_output_tokens, Some(64));
    // Absent, or not a count: no limit is reported.
    for body in [
        json!({ "input": "hi" }),
        json!({ "max_output_tokens": "64", "input": "hi" }),
    ] {
        assert_eq!(
            request(Protocol::Responses, body.to_string().as_bytes()).max_output_tokens,
            None
        );
    }
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
    let activity = read(
        Protocol::Messages,
        ResponseFormat::EventStream,
        ResponseEncoding::Gzip,
        ActivityLimits::default(),
        &[&compressed],
    )
    .activity;
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
    let activity = response(
        Protocol::Messages,
        ResponseFormat::Json,
        document.as_bytes(),
    );
    assert_eq!(activity.reply, "Done.");
    assert_eq!(activity.tool_uses[0].kind, "tool_use");
    assert!(activity.limitations.is_empty());
}

#[test]
fn responses_requests_read_items_after_the_models_last_output() {
    let body = json!({
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
    let compressed = zstd::encode_all(body.as_bytes(), 1).unwrap();
    let activity = request(Protocol::Responses, &compressed);
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
    let mut reader = RequestReader::new(Protocol::Responses, ActivityLimits::default());
    reader.message();
    reader.feed(&frame.as_bytes()[range]);
    let activity = reader.finish(BodyEnd::Complete);
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
    let activity = response(Protocol::Responses, ResponseFormat::EventStream, &stream);
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
    let activity = read(
        Protocol::Responses,
        ResponseFormat::WebSocketMessages,
        ResponseEncoding::Identity,
        ActivityLimits::default(),
        &[&terminal.as_bytes()[range]],
    )
    .activity;
    assert_eq!(activity.tool_uses[0].id.as_deref(), Some("call_9"));
}

#[test]
fn unreadable_bodies_are_limitations_never_facts() {
    let not_json = request(Protocol::Messages, b"{not json");
    assert_eq!(not_json.limitations, [ActivityLimitation::NotJson]);
    let shape = request(Protocol::Messages, b"{\"messages\": 3}");
    assert_eq!(shape.limitations, [ActivityLimitation::UnexpectedShape]);
    let mut cut = RequestReader::new(Protocol::Messages, ActivityLimits::default());
    cut.feed(b"{\"messages\": [");
    assert_eq!(
        cut.finish(BodyEnd::Unfinished).limitations,
        [ActivityLimitation::Truncated, ActivityLimitation::NotJson]
    );
    // A response that cannot be decoded is the inspection's limitation, and
    // shows no activity.
    let corrupt = read(
        Protocol::Messages,
        ResponseFormat::EventStream,
        ResponseEncoding::Gzip,
        ActivityLimits::default(),
        &[b"\x1f\x8b\x08garbage"],
    );
    assert!(
        corrupt
            .inspection
            .limitations()
            .contains(&crate::inspect::Limitation::DecodingError)
    );
    assert!(corrupt.activity.tool_uses.is_empty());
    assert!(corrupt.activity.reply.is_empty());
}

#[test]
fn both_readers_cap_tool_calls_alike_and_keep_later_text() {
    let limited = |protocol, document: &str| {
        read(
            protocol,
            ResponseFormat::Json,
            ResponseEncoding::Identity,
            ActivityLimits {
                max_tool_uses: 1,
                ..ActivityLimits::default()
            },
            &[document.as_bytes()],
        )
        .activity
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
        limited(Protocol::Messages, &messages),
        limited(Protocol::Responses, &responses),
    ] {
        assert_eq!(activity.tool_uses.len(), 1);
        assert_eq!(activity.limitations, [ActivityLimitation::TooManyToolCalls]);
        // Empty text parts leave no blank lines; text after the cap is kept.
        assert_eq!(activity.reply, "After.");
    }
}

/// One finished response of each protocol, as streamed and as a document.
fn responses() -> Vec<(Protocol, ResponseFormat, Vec<u8>)> {
    let function = json!({ "type": "function_call", "call_id": "call_9", "name": "shell", "arguments": "{\"command\":[\"cargo\",\"test\"]}" });
    let message = json!({ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "Running tests." }] });
    vec![
        (
            Protocol::Messages,
            ResponseFormat::EventStream,
            sse(&[
                json!({ "type": "message_start", "message": {} }),
                json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
                json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "Reading «it»." } }),
                json!({ "type": "content_block_stop", "index": 0 }),
                json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "tool_use", "id": "toolu_2", "name": "Read", "input": {} } }),
                json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{\"file_pa" } }),
                json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": "th\":\"src/lib.rs\"}" } }),
                json!({ "type": "content_block_stop", "index": 1 }),
                json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": { "output_tokens": 9 } }),
                json!({ "type": "message_stop" }),
            ]),
        ),
        (
            Protocol::Messages,
            ResponseFormat::Json,
            json!({ "type": "message", "content": [
                { "type": "text", "text": "Done." },
                { "type": "tool_use", "id": "toolu_4", "name": "Edit", "input": { "file_path": "a.rs" } },
            ], "stop_reason": "end_turn" })
            .to_string()
            .into_bytes(),
        ),
        (
            Protocol::Responses,
            ResponseFormat::EventStream,
            sse(&[
                json!({ "type": "response.created", "response": {} }),
                json!({ "type": "response.output_item.added", "output_index": 0, "item": { "type": "message", "id": "msg_1", "role": "assistant", "content": [] } }),
                json!({ "type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "Running " }),
                json!({ "type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": "tests." }),
                json!({ "type": "response.output_item.done", "output_index": 0, "item": message }),
                json!({ "type": "response.output_item.done", "item": function }),
                json!({ "type": "response.completed", "response": { "output": [] } }),
            ]),
        ),
        (
            Protocol::Responses,
            ResponseFormat::Json,
            json!({ "object": "response", "status": "completed", "output": [message, function] })
                .to_string()
                .into_bytes(),
        ),
    ]
}

proptest::proptest! {
    /// Read while streaming, in any pieces, a response shows the same
    /// activity as when it arrives in one piece.
    #[test]
    fn streamed_activity_is_independent_of_piece_boundaries(
        which in 0..4_usize,
        gzip in proptest::bool::ANY,
        cuts in proptest::collection::vec(proptest::num::usize::ANY, 0..24),
    ) {
        let (protocol, format, plain) = responses().swap_remove(which);
        let (bytes, encoding) = if gzip {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(&plain).unwrap();
            (encoder.finish().unwrap(), ResponseEncoding::Gzip)
        } else {
            (plain, ResponseEncoding::Identity)
        };
        let stored = read(protocol, format, encoding, ActivityLimits::default(), &[&bytes]).activity;
        let mut inspector = crate::CallInspector::new(
            protocol,
            format,
            encoding,
            crate::inspect::pipeline::InspectionLimits::default(),
        );
        let mut points: Vec<usize> = cuts.iter().map(|cut| cut % (bytes.len() + 1)).collect();
        points.sort_unstable();
        let mut from = 0;
        for point in points.into_iter().chain([bytes.len()]) {
            inspector.feed(&bytes[from..point.max(from)]);
            from = point.max(from);
        }
        let streamed = inspector.finish(crate::inspect::pipeline::BodyEnd::Complete).activity;
        proptest::prop_assert!(!stored.reply.is_empty());
        proptest::prop_assert_eq!(streamed, stored);
    }
}

proptest::proptest! {
    /// While a response streams, in any pieces, the reply so far is always a
    /// prefix of the final reply, and reading it mid-stream leaves the final
    /// reading unchanged.
    #[test]
    fn the_reply_so_far_grows_into_the_final_reply(
        which in 0..4_usize,
        cuts in proptest::collection::vec(proptest::num::usize::ANY, 0..24),
    ) {
        let (protocol, format, bytes) = responses().swap_remove(which);
        let stored = read(protocol, format, ResponseEncoding::Identity, ActivityLimits::default(), &[&bytes]).activity;
        let mut inspector = crate::CallInspector::new(
            protocol,
            format,
            ResponseEncoding::Identity,
            crate::inspect::pipeline::InspectionLimits::default(),
        );
        let mut points: Vec<usize> = cuts.iter().map(|cut| cut % (bytes.len() + 1)).collect();
        points.sort_unstable();
        let mut from = 0;
        for point in points.into_iter().chain([bytes.len()]) {
            inspector.feed(&bytes[from..point.max(from)]);
            from = point.max(from);
            let so_far = inspector.so_far().unwrap();
            proptest::prop_assert!(stored.reply.starts_with(&so_far.reply), "{:?} is not a prefix of {:?}", so_far.reply, stored.reply);
            proptest::prop_assert!(so_far.tool_uses.len() <= stored.tool_uses.len());
        }
        let streamed = inspector.finish(crate::inspect::pipeline::BodyEnd::Complete).activity;
        proptest::prop_assert_eq!(streamed, stored);
    }
}

proptest::proptest! {
    /// A request gathered in any pieces reads as it does in one piece.
    #[test]
    fn a_gathered_request_reads_as_the_stored_one(
        gzip in proptest::bool::ANY,
        cuts in proptest::collection::vec(proptest::num::usize::ANY, 0..16),
    ) {
        let plain = json!({
            "messages": [
                { "role": "user", "content": "Fix the failing test" },
                { "role": "assistant", "content": [{ "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {} }] },
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu_1", "is_error": true, "content": "exit 1" },
                    { "type": "text", "text": "Now try «this»" },
                ] },
            ],
        })
        .to_string()
        .into_bytes();
        let bytes = if gzip {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(&plain).unwrap();
            encoder.finish().unwrap()
        } else {
            plain
        };
        let stored = request(Protocol::Messages, &bytes);
        let mut gathered = RequestReader::new(Protocol::Messages, ActivityLimits::default());
        let mut points: Vec<usize> = cuts.iter().map(|cut| cut % (bytes.len() + 1)).collect();
        points.sort_unstable();
        let mut from = 0;
        for point in points.into_iter().chain([bytes.len()]) {
            gathered.feed(&bytes[from..point.max(from)]);
            from = point.max(from);
        }
        let gathered = gathered.finish(crate::inspect::pipeline::BodyEnd::Complete);
        proptest::prop_assert_eq!(&gathered.user_text, &["Now try «this»".to_owned()]);
        proptest::prop_assert_eq!(gathered, stored);
    }
}

#[test]
fn a_gathered_request_says_what_it_missed() {
    let request = json!({ "messages": [{ "role": "user", "content": "hi" }] }).to_string();
    let limits = ActivityLimits {
        max_request_bytes: 10,
        ..ActivityLimits::default()
    };
    let mut over = RequestReader::new(Protocol::Messages, limits);
    over.feed(request.as_bytes());
    let over = over.finish(crate::inspect::pipeline::BodyEnd::Complete);
    assert!(over.limitations.contains(&ActivityLimitation::Truncated));
    assert!(over.user_text.is_empty());

    let mut cut = RequestReader::new(Protocol::Messages, ActivityLimits::default());
    cut.feed(&request.as_bytes()[..5]);
    cut.gap();
    cut.feed(&request.as_bytes()[5..]);
    let cut = cut.finish(crate::inspect::pipeline::BodyEnd::Complete);
    assert!(cut.limitations.contains(&ActivityLimitation::CaptureGap));

    // A WebSocket call's request is its one message.
    let create = json!({ "type": "response.create", "input": [
        { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "Run it" }] },
    ] })
    .to_string();
    let mut message = RequestReader::new(Protocol::Responses, ActivityLimits::default());
    message.message();
    message.feed(create.as_bytes());
    let message = message.finish(crate::inspect::pipeline::BodyEnd::Complete);
    assert_eq!(message.user_text, ["Run it"]);
}

#[test]
fn chat_requests_yield_the_newest_user_input_and_tool_results() {
    let first = json!({
        "model": "m",
        "max_tokens": 512,
        "messages": [
            { "role": "system", "content": "Be terse." },
            { "role": "user", "content": [{ "type": "text", "text": "Fix the build" }, { "type": "image_url", "image_url": { "url": "data:," } }] },
        ],
    })
    .to_string();
    let activity = request(Protocol::ChatCompletions, first.as_bytes());
    assert_eq!(activity.user_text, ["Fix the build"]);
    assert_eq!(activity.max_output_tokens, Some(512));
    assert!(activity.limitations.is_empty());

    let later = json!({
        "max_completion_tokens": 1024,
        "max_tokens": 512,
        "messages": [
            { "role": "user", "content": "Fix the build" },
            { "role": "assistant", "content": null, "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": "run", "arguments": "{}" } }] },
            { "role": "tool", "tool_call_id": "call_1", "content": "exit 1" },
            { "role": "tool", "content": "no id" },
            { "role": "narrator", "content": "?" },
        ],
    })
    .to_string();
    let activity = request(Protocol::ChatCompletions, later.as_bytes());
    assert!(activity.user_text.is_empty());
    assert_eq!(activity.max_output_tokens, Some(1024));
    assert_eq!(
        activity.tool_results,
        [ToolResult {
            id: "call_1".into(),
            status: ResultStatus::Unreported,
            output: "exit 1".into(),
        }]
    );
    assert_eq!(activity.limitations, [ActivityLimitation::UnexpectedShape]);

    let shapeless = request(Protocol::ChatCompletions, br#"{"prompt":"legacy"}"#);
    assert_eq!(shapeless.limitations, [ActivityLimitation::UnexpectedShape]);
}

#[test]
fn chat_responses_keep_choices_apart_and_bound_tool_calls() {
    let chunk = |choices: Value| json!({ "object": "chat.completion.chunk", "choices": choices });
    let mut stream = Vec::new();
    for event in [
        chunk(
            json!([{ "index": 1, "delta": { "content": "second" } }, { "index": 0, "delta": { "content": "first" } }]),
        ),
        chunk(
            json!([{ "index": 0, "delta": { "tool_calls": [{ "index": 0, "id": "a", "function": { "name": "one", "arguments": "{}" } }] } }]),
        ),
        chunk(
            json!([{ "index": 0, "delta": { "tool_calls": [{ "index": 1, "id": "b", "function": { "name": "two" } }] }, "finish_reason": "tool_calls" }]),
        ),
        chunk(json!([{ "index": 1, "delta": {}, "finish_reason": "stop" }])),
    ] {
        stream.extend_from_slice(format!("data: {event}\n\n").as_bytes());
    }
    stream.extend_from_slice(b"data: [DONE]\n\n");
    let limited = read(
        Protocol::ChatCompletions,
        ResponseFormat::EventStream,
        ResponseEncoding::Identity,
        ActivityLimits {
            max_tool_uses: 1,
            ..ActivityLimits::default()
        },
        &[&stream],
    )
    .activity;
    assert_eq!(limited.reply, "first\nsecond");
    assert_eq!(limited.tool_uses.len(), 1);
    assert_eq!(limited.tool_uses[0].input, ToolInput::Json(json!({})));
    assert_eq!(limited.limitations, [ActivityLimitation::TooManyToolCalls]);

    let whole = response(
        Protocol::ChatCompletions,
        ResponseFormat::EventStream,
        &stream,
    );
    assert_eq!(whole.tool_uses[1].name, "two");
    assert_eq!(whole.tool_uses[1].input, ToolInput::Json(Value::Null));
    assert!(whole.limitations.is_empty());
}

#[test]
fn a_response_is_thinking_while_its_newest_thinking_block_or_reasoning_item_is_open() {
    let watch = |protocol: Protocol, events: &[Value]| -> Vec<bool> {
        let mut inspector = CallInspector::new(
            protocol,
            ResponseFormat::EventStream,
            ResponseEncoding::Identity,
            InspectionLimits::default(),
        );
        events
            .iter()
            .map(|event| {
                inspector.feed(&sse(std::slice::from_ref(event)));
                inspector.thinking()
            })
            .collect()
    };
    let messages = watch(
        Protocol::Messages,
        &[
            json!({ "type": "message_start", "message": { "content": [] } }),
            json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "thinking", "thinking": "" } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "thinking_delta", "thinking": "Let me look." } }),
            json!({ "type": "content_block_stop", "index": 0 }),
            json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "text", "text": "" } }),
        ],
    );
    assert_eq!(messages, [false, true, true, false, false]);
    let responses = watch(
        Protocol::Responses,
        &[
            json!({ "type": "response.created", "response": {} }),
            json!({ "type": "response.output_item.added", "output_index": 0, "item": { "type": "reasoning" } }),
            json!({ "type": "response.reasoning_summary_text.delta", "output_index": 0, "delta": "Checking." }),
            json!({ "type": "response.output_item.done", "output_index": 0, "item": { "type": "reasoning" } }),
            json!({ "type": "response.output_item.added", "output_index": 1, "item": { "type": "message" } }),
        ],
    );
    assert_eq!(responses, [false, true, true, false, false]);
}

#[test]
fn a_responses_reply_streams_from_its_text_deltas() {
    let mut inspector = CallInspector::new(
        Protocol::Responses,
        ResponseFormat::EventStream,
        ResponseEncoding::Identity,
        InspectionLimits::default(),
    );
    let mut seen = Vec::new();
    for event in [
        json!({ "type": "response.output_item.added", "output_index": 0, "item": { "type": "reasoning", "id": "rs_1" } }),
        json!({ "type": "response.output_item.done", "output_index": 0, "item": { "type": "reasoning", "id": "rs_1", "summary": [] } }),
        json!({ "type": "response.output_item.added", "output_index": 1, "item": { "type": "message", "id": "msg_1" } }),
        json!({ "type": "response.output_text.delta", "item_id": "msg_1", "output_index": 1, "delta": "Sorted " }),
        json!({ "type": "response.output_text.delta", "item_id": "msg_1", "output_index": 1, "delta": "newest first." }),
        json!({ "type": "response.output_item.done", "output_index": 1, "item": { "type": "message", "id": "msg_1",
            "content": [{ "type": "output_text", "text": "Sorted newest first." }] } }),
    ] {
        inspector.feed(&sse(&[event]));
        seen.push(inspector.so_far().unwrap().reply);
    }
    assert_eq!(
        seen,
        [
            "",
            "",
            "",
            "Sorted ",
            "Sorted newest first.",
            "Sorted newest first."
        ]
    );
}
