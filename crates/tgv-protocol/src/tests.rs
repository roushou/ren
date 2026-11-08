//! Scenario tests for the whole inspection pipeline, using synthetic payloads.

use super::{
    CallInspector,
    encoding::{ResponseEncoding, ResponseFormat},
    inspect::{
        Coverage, FormatSource, InputAccounting, InspectionReport, KindFamily, Limitation,
        ModelOutcome, TerminalEvent,
        pipeline::{BodyEnd, InspectionLimits, Protocol},
    },
    text::WireText,
};
use proptest::prelude::*;
use serde_json::{Value, json};
use std::io::Write;

fn sse(events: &[Value]) -> Vec<u8> {
    events
        .iter()
        .flat_map(|event| format!("data: {event}\n\n").into_bytes())
        .collect()
}

fn named(name: &str, data: &str) -> Vec<u8> {
    format!("event: {name}\ndata: {data}\n\n").into_bytes()
}

fn inspect_chunks(
    protocol: Protocol,
    format: ResponseFormat,
    encoding: ResponseEncoding,
    chunks: &[&[u8]],
    end: BodyEnd,
) -> InspectionReport {
    let mut inspector = CallInspector::new(protocol, format, encoding, InspectionLimits::default());
    for chunk in chunks {
        inspector.feed(chunk);
    }
    inspector.finish(end).inspection
}

fn inspect(
    protocol: Protocol,
    format: ResponseFormat,
    body: &[u8],
    end: BodyEnd,
) -> InspectionReport {
    inspect_chunks(protocol, format, ResponseEncoding::Identity, &[body], end)
}

fn text(report_text: Option<&WireText>) -> Option<&str> {
    report_text.map(WireText::as_str)
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn anthropic_stream() -> Vec<u8> {
    sse(&[
        json!({"type":"message_start","message":{"model":"fixture-claude","stop_reason":null,"usage":{"input_tokens":12,"output_tokens":1,"cache_read_input_tokens":100,"cache_creation_input_tokens":20}}}),
        json!({"type":"ping"}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_fixture","name":"read_file","input":{}}}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":9}}),
        json!({"type":"message_stop"}),
    ])
}

#[test]
fn anthropic_stream_reports_model_usage_tools_and_outcome() {
    let report = inspect(
        Protocol::Messages,
        ResponseFormat::EventStream,
        &anthropic_stream(),
        BodyEnd::Complete,
    );
    assert_eq!(report.coverage(), Coverage::Complete);
    assert_eq!(report.limitations(), []);
    assert_eq!(report.outcome(), Some(ModelOutcome::Completed));
    assert_eq!(report.terminal_event(), TerminalEvent::Seen);
    assert_eq!(text(report.model()), Some("fixture-claude"));
    assert_eq!(text(report.stop_reason()), Some("tool_use"));
    let usage = report.usage().unwrap();
    assert_eq!(usage.accounting, InputAccounting::ExcludesCache);
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_creation_tokens
        ),
        (Some(12), Some(9), Some(100), Some(20))
    );
    let [call] = report.tool_calls() else {
        panic!("expected one tool call")
    };
    assert_eq!(text(call.call_id.as_ref()), Some("toolu_fixture"));
    assert_eq!(call.kind.as_str(), "tool_use");
    assert_eq!(text(call.name.as_ref()), Some("read_file"));
    assert_eq!(report.events().understood, 8);
}

#[test]
fn stop_reasons_map_to_deliberate_outcomes_and_unknown_reasons_stay_unknown() {
    for (reason, outcome) in [
        ("end_turn", ModelOutcome::Completed),
        ("max_tokens", ModelOutcome::Incomplete),
        ("pause_turn", ModelOutcome::Incomplete),
        ("refusal", ModelOutcome::Refused),
        ("a_future_reason", ModelOutcome::Unknown),
    ] {
        let body = sse(&[
            json!({"type":"message_delta","delta":{"stop_reason":reason}}),
            json!({"type":"message_stop"}),
        ]);
        let report = inspect(
            Protocol::Messages,
            ResponseFormat::EventStream,
            &body,
            BodyEnd::Complete,
        );
        assert_eq!(report.outcome(), Some(outcome), "{reason}");
        assert_eq!(text(report.stop_reason()), Some(reason));
    }
}

#[test]
fn unknown_events_blocks_and_items_are_kept_and_counted() {
    let body = sse(&[
        json!({"type":"a_future_event","payload":1}),
        json!({"type":"a_future_event"}),
        json!({"type":"content_block_start","content_block":{"type":"a_future_block","id":"x"}}),
        json!({"type":"message_stop"}),
    ]);
    let report = inspect(
        Protocol::Messages,
        ResponseFormat::EventStream,
        &body,
        BodyEnd::Complete,
    );
    let unknown: Vec<_> = report
        .unknown_kinds()
        .iter()
        .map(|kind| (kind.family, kind.kind.as_str(), kind.count))
        .collect();
    assert_eq!(
        unknown,
        [
            (KindFamily::Event, "a_future_event", 2),
            (KindFamily::ContentBlock, "a_future_block", 1),
        ]
    );
    assert_eq!(report.events().unknown, 2);
    assert!(
        report.tool_calls().is_empty(),
        "unknown blocks are never guessed to be tools"
    );
    // A stop marker without a stop reason is terminal, but its outcome unknown.
    assert_eq!(report.outcome(), None);
    assert_eq!(report.terminal_event(), TerminalEvent::Seen);
}

#[test]
fn non_object_payloads_are_malformed_and_never_terminal_facts() {
    for (protocol, name) in [
        (Protocol::Messages, "message_stop"),
        (Protocol::Responses, "response.completed"),
    ] {
        let mut body = named(name, "42");
        body.extend(named(name, "not json"));
        body.extend(named(name, r#"{"type":["message_stop"]}"#));
        let report = inspect(
            protocol,
            ResponseFormat::EventStream,
            &body,
            BodyEnd::Complete,
        );
        assert_eq!(report.outcome(), None);
        assert_eq!(report.terminal_event(), TerminalEvent::Missing);
        assert_eq!(report.events().malformed, 3);
        assert_eq!(
            report.limitations(),
            [Limitation::MalformedEvent, Limitation::NoTerminalEvent]
        );
    }
}

#[test]
fn sse_event_names_are_only_fallbacks_and_done_is_not_terminal() {
    let body = [
        named("message_stop", r#"{"type":"a_future_event"}"#),
        b"data: [DONE]\n\n".to_vec(),
    ]
    .concat();
    let report = inspect(
        Protocol::Messages,
        ResponseFormat::EventStream,
        &body,
        BodyEnd::Complete,
    );
    assert_eq!(report.terminal_event(), TerminalEvent::Missing);
    let body = named("message_stop", "{}");
    let report = inspect(
        Protocol::Messages,
        ResponseFormat::EventStream,
        &body,
        BodyEnd::Complete,
    );
    assert_eq!(report.terminal_event(), TerminalEvent::Seen);
}

#[test]
fn responses_terminal_event_is_independent_of_http_completion() {
    let body = sse(&[
        json!({"type":"response.created","response":{"model":"fixture-gpt","status":"in_progress"}}),
        json!({"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","name":"lookup"}}),
        json!({"type":"response.function_call_arguments.delta","delta":"{}"}),
        json!({"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"lookup"}}),
        json!({"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":30,"output_tokens":4,"input_tokens_details":{"cached_tokens":10},"output_tokens_details":{"reasoning_tokens":2}}}}),
    ]);
    let report = inspect(
        Protocol::Responses,
        ResponseFormat::EventStream,
        &body,
        BodyEnd::Unfinished,
    );
    assert_eq!(report.outcome(), Some(ModelOutcome::Completed));
    assert_eq!(report.terminal_event(), TerminalEvent::Seen);
    assert_eq!(report.limitations(), [Limitation::BodyUnfinished]);
    assert_eq!(report.coverage(), Coverage::Partial);
    let [call] = report.tool_calls() else {
        panic!("added and done describe one call")
    };
    assert_eq!(text(call.item_id.as_ref()), Some("fc_1"));
    assert_eq!(text(call.call_id.as_ref()), Some("call_1"));
    let usage = report.usage().unwrap();
    assert_eq!(usage.accounting, InputAccounting::IncludesCache);
    assert_eq!(
        (
            usage.input_tokens,
            usage.cache_read_tokens,
            usage.reasoning_tokens
        ),
        (Some(30), Some(10), Some(2))
    );
}

#[test]
fn conservative_outcomes_win_over_later_or_outer_success() {
    let responses = sse(&[json!({"type":"response.completed","response":{"status":"failed"}})]);
    let report = inspect(
        Protocol::Responses,
        ResponseFormat::EventStream,
        &responses,
        BodyEnd::Complete,
    );
    assert_eq!(report.outcome(), Some(ModelOutcome::Failed));
    let messages = sse(&[
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
        json!({"type":"error","error":{"type":"overloaded_error","message":"fixture"}}),
    ]);
    let report = inspect(
        Protocol::Messages,
        ResponseFormat::EventStream,
        &messages,
        BodyEnd::Complete,
    );
    assert_eq!(report.outcome(), Some(ModelOutcome::Failed));
    let cancelled = br#"{"status":"cancelled","model":"fixture-gpt"}"#;
    let report = inspect(
        Protocol::Responses,
        ResponseFormat::Json,
        cancelled,
        BodyEnd::Complete,
    );
    assert_eq!(report.outcome(), Some(ModelOutcome::Cancelled));
}

#[test]
fn documents_are_parsed_whole_and_errors_are_failures() {
    let message = json!({"type":"message","model":"fixture-claude","stop_reason":"end_turn","usage":{"input_tokens":3},"content":[{"type":"text","text":"x"},null,{"type":"tool_use","id":"toolu_1","name":"run"}]});
    let report = inspect(
        Protocol::Messages,
        ResponseFormat::Json,
        &serde_json::to_vec(&message).unwrap(),
        BodyEnd::Complete,
    );
    assert_eq!(report.outcome(), Some(ModelOutcome::Completed));
    assert_eq!(report.tool_calls().len(), 1);
    assert_eq!(report.limitations(), [Limitation::MalformedField]);
    let error = br#"{"type":"error","error":{"type":"rate_limit_error","message":"fixture"}}"#;
    let report = inspect(
        Protocol::Messages,
        ResponseFormat::Json,
        error,
        BodyEnd::Complete,
    );
    assert_eq!(report.outcome(), Some(ModelOutcome::Failed));
    assert_eq!(report.terminal_event(), TerminalEvent::Seen);
}

#[test]
fn large_documents_parse_beyond_the_sse_event_limit() {
    let padding = "x".repeat(2 * 1024 * 1024);
    let body = serde_json::to_vec(
        &json!({"model":"fixture-gpt","status":"completed","instructions":padding}),
    )
    .unwrap();
    let report = inspect(
        Protocol::Responses,
        ResponseFormat::Json,
        &body,
        BodyEnd::Complete,
    );
    assert_eq!(report.outcome(), Some(ModelOutcome::Completed));
    assert_eq!(report.limitations(), []);
}

#[test]
fn truncated_documents_are_not_parsed() {
    let body = br#"{"model":"fixture-gpt","status":"completed"}"#;
    let report = inspect(
        Protocol::Responses,
        ResponseFormat::Json,
        body,
        BodyEnd::Unfinished,
    );
    assert_eq!(report.outcome(), None);
    assert_eq!(
        report.limitations(),
        [Limitation::BodyUnfinished, Limitation::NoTerminalEvent]
    );
}

#[test]
fn malformed_fields_are_reported_without_hiding_siblings() {
    let body = sse(&[
        json!({"type":"message_start","message":{"model":["bad"],"usage":{"input_tokens":"bad","output_tokens":4}}}),
        json!({"type":"message_stop"}),
    ]);
    let report = inspect(
        Protocol::Messages,
        ResponseFormat::EventStream,
        &body,
        BodyEnd::Complete,
    );
    assert_eq!(report.model(), None);
    assert_eq!(report.usage().unwrap().output_tokens, Some(4));
    assert_eq!(report.limitations(), [Limitation::MalformedField]);
}

#[test]
fn compressed_streams_after_a_gap_report_the_gap_only() {
    let compressed = gzip(&anthropic_stream());
    let (head, tail) = compressed.split_at(compressed.len() / 2);
    let mut inspector = CallInspector::new(
        Protocol::Messages,
        ResponseFormat::EventStream,
        ResponseEncoding::Gzip,
        InspectionLimits::default(),
    );
    inspector.feed(head);
    inspector.gap();
    inspector.feed(tail);
    let report = inspector.finish(BodyEnd::Complete).inspection;
    assert_eq!(
        report.limitations(),
        [Limitation::CaptureGap, Limitation::NoTerminalEvent]
    );
    assert_eq!(report.outcome(), None);
}

#[test]
fn truncated_compression_is_unfinished_not_corrupt() {
    let compressed = gzip(&anthropic_stream());
    let head = &compressed[..compressed.len() - 4];
    let unfinished = inspect_chunks(
        Protocol::Messages,
        ResponseFormat::EventStream,
        ResponseEncoding::Gzip,
        &[head],
        BodyEnd::Unfinished,
    );
    assert!(
        !unfinished
            .limitations()
            .contains(&Limitation::DecodingError)
    );
    assert!(
        unfinished
            .limitations()
            .contains(&Limitation::BodyUnfinished)
    );
    let complete = inspect_chunks(
        Protocol::Messages,
        ResponseFormat::EventStream,
        ResponseEncoding::Gzip,
        &[head],
        BodyEnd::Complete,
    );
    assert!(complete.limitations().contains(&Limitation::DecodingError));
}

#[test]
fn decoded_size_is_bounded() {
    let body = gzip(&vec![b' '; 4096]);
    let limits = InspectionLimits {
        max_decoded_bytes: 1024,
        ..InspectionLimits::default()
    };
    let mut inspector = CallInspector::new(
        Protocol::Responses,
        ResponseFormat::EventStream,
        ResponseEncoding::Gzip,
        limits,
    );
    inspector.feed(&body);
    let report = inspector.finish(BodyEnd::Complete).inspection;
    assert_eq!(
        report.limitations(),
        [Limitation::DecodedTooLarge, Limitation::NoTerminalEvent]
    );
}

#[test]
fn formats_and_encodings_that_cannot_be_read_are_explicit() {
    let missing_type =
        sse(&[json!({"type":"response.completed","response":{"status":"completed"}})]);
    let codex = inspect(
        Protocol::Responses,
        ResponseFormat::Unspecified,
        &missing_type,
        BodyEnd::Complete,
    );
    assert_eq!(codex.format(), FormatSource::Inferred);
    assert_eq!(codex.outcome(), Some(ModelOutcome::Completed));
    for (protocol, format, encoding, reason) in [
        (
            Protocol::Messages,
            ResponseFormat::Unspecified,
            ResponseEncoding::Identity,
            Limitation::UnsupportedFormat,
        ),
        (
            Protocol::Responses,
            ResponseFormat::Other,
            ResponseEncoding::Identity,
            Limitation::UnsupportedFormat,
        ),
        (
            Protocol::Messages,
            ResponseFormat::EventStream,
            ResponseEncoding::Unsupported,
            Limitation::UnsupportedEncoding,
        ),
    ] {
        let report = inspect_chunks(
            protocol,
            format,
            encoding,
            &[&anthropic_stream()],
            BodyEnd::Complete,
        );
        assert_eq!(report.coverage(), Coverage::Unavailable);
        assert_eq!(report.limitations(), [reason]);
        assert_eq!(report.model(), None);
    }
}

#[test]
fn incomplete_trailing_events_are_reported() {
    let mut body = anthropic_stream();
    body.extend_from_slice(b"data: {\"type\":\"ping\"}");
    let report = inspect(
        Protocol::Messages,
        ResponseFormat::EventStream,
        &body,
        BodyEnd::Complete,
    );
    assert_eq!(report.limitations(), [Limitation::IncompleteEvent]);
    assert_eq!(report.outcome(), Some(ModelOutcome::Completed));
}

#[test]
fn sources_stay_free_of_runtime_and_io() {
    let sources = [
        ("lib.rs", include_str!("lib.rs")),
        ("activity/mod.rs", include_str!("activity/mod.rs")),
        (
            "activity/anthropic.rs",
            include_str!("activity/anthropic.rs"),
        ),
        ("activity/openai.rs", include_str!("activity/openai.rs")),
        ("direction.rs", include_str!("direction.rs")),
        ("encoding.rs", include_str!("encoding.rs")),
        ("inspect/mod.rs", include_str!("inspect/mod.rs")),
        ("inspect/pipeline.rs", include_str!("inspect/pipeline.rs")),
        ("json.rs", include_str!("json.rs")),
        ("label.rs", include_str!("label.rs")),
        ("messages.rs", include_str!("messages.rs")),
        ("native.rs", include_str!("native.rs")),
        ("responses.rs", include_str!("responses.rs")),
        ("route.rs", include_str!("route.rs")),
        ("sse.rs", include_str!("sse.rs")),
        ("text.rs", include_str!("text.rs")),
        ("websocket.rs", include_str!("websocket.rs")),
    ];
    let forbidden = [
        "tokio",
        "axum",
        "tower",
        "hyper",
        "reqwest",
        "sqlx",
        "clap",
        "tracing",
        "std::fs",
        "std::net",
        "std::env",
        "std::process",
    ];
    for (file, source) in sources {
        for line in source.lines().map(str::trim_start) {
            if line.starts_with("//") {
                continue;
            }
            for dependency in forbidden {
                assert!(
                    !line.contains(&format!("{dependency}::"))
                        && !line.contains(&format!("use {dependency}")),
                    "{file} depends on {dependency}: {line}"
                );
            }
        }
    }
}

fn encode(encoding: ResponseEncoding, body: &[u8]) -> Vec<u8> {
    match encoding {
        ResponseEncoding::Identity => body.to_vec(),
        ResponseEncoding::Gzip => gzip(body),
        ResponseEncoding::Zstd => zstd::stream::encode_all(body, 3).unwrap(),
        ResponseEncoding::Brotli => {
            let mut encoded = Vec::new();
            {
                let mut encoder = brotli::CompressorWriter::new(&mut encoded, 4096, 5, 22);
                encoder.write_all(body).unwrap();
            }
            encoded
        }
        ResponseEncoding::Unsupported => unreachable!("not generated"),
    }
}

fn split<'a>(bytes: &'a [u8], cuts: &[usize]) -> Vec<&'a [u8]> {
    let mut positions: Vec<usize> = cuts.iter().map(|cut| cut % (bytes.len() + 1)).collect();
    positions.sort_unstable();
    let mut chunks = Vec::new();
    let mut start = 0;
    for position in positions {
        chunks.push(&bytes[start..position]);
        start = position;
    }
    chunks.push(&bytes[start..]);
    chunks
}

proptest! {
    #[test]
    fn reports_are_independent_of_chunk_boundaries(
        encoding in prop_oneof![
            Just(ResponseEncoding::Identity),
            Just(ResponseEncoding::Gzip),
            Just(ResponseEncoding::Zstd),
            Just(ResponseEncoding::Brotli),
        ],
        crlf in any::<bool>(),
        cuts in proptest::collection::vec(any::<usize>(), 0..32),
    ) {
        let mut body = anthropic_stream();
        if crlf {
            body = String::from_utf8(body).unwrap().replace('\n', "\r\n").into_bytes();
        }
        let encoded = encode(encoding, &body);
        let whole = inspect_chunks(Protocol::Messages, ResponseFormat::EventStream, encoding, &[&encoded], BodyEnd::Complete);
        let fragmented = inspect_chunks(
            Protocol::Messages,
            ResponseFormat::EventStream,
            encoding,
            &split(&encoded, &cuts),
            BodyEnd::Complete,
        );
        prop_assert_eq!(&whole, &fragmented);
        prop_assert_eq!(whole.outcome(), Some(ModelOutcome::Completed));
        prop_assert_eq!(whole.limitations(), &[][..]);
    }
}

fn websocket_messages() -> Vec<Vec<u8>> {
    [
        json!({"type":"response.created","response":{"id":"resp_1","status":"in_progress","model":"fixture-gpt"}}),
        json!({"type":"response.output_text.delta","delta":"hi"}),
        json!({"type":"response.interrupt.accepted","response_id":"resp_1"}),
        json!({"type":"response.completed","response":{"status":"completed","model":"fixture-gpt","usage":{"input_tokens":3,"output_tokens":1}}}),
    ]
    .iter()
    .map(|message| message.to_string().into_bytes())
    .collect()
}

/// Feed messages back to back, announcing each, with the bytes cut at `cuts`.
fn inspect_messages(messages: &[Vec<u8>], cuts: &[usize], end: BodyEnd) -> InspectionReport {
    let mut inspector = CallInspector::new(
        Protocol::Responses,
        ResponseFormat::WebSocketMessages,
        ResponseEncoding::Identity,
        InspectionLimits::default(),
    );
    for message in messages {
        inspector.message(u64::try_from(message.len()).unwrap());
        for piece in split(message, cuts) {
            inspector.feed(piece);
        }
    }
    inspector.finish(end).inspection
}

#[test]
fn websocket_messages_are_inspected_as_events() {
    let report = inspect_messages(&websocket_messages(), &[], BodyEnd::Complete);
    assert_eq!(report.outcome(), Some(ModelOutcome::Completed));
    assert_eq!(report.terminal_event(), TerminalEvent::Seen);
    assert_eq!(text(report.model()), Some("fixture-gpt"));
    assert_eq!(report.limitations(), &[][..]);

    // A message cut short is reported, and nothing after it is invented.
    let mut inspector = CallInspector::new(
        Protocol::Responses,
        ResponseFormat::WebSocketMessages,
        ResponseEncoding::Identity,
        InspectionLimits::default(),
    );
    let messages = websocket_messages();
    let last = messages.last().unwrap();
    inspector.message(u64::try_from(last.len()).unwrap());
    inspector.feed(&last[..10]);
    let report = inspector.finish(BodyEnd::Unfinished).inspection;
    assert_eq!(report.outcome(), None);
    assert!(report.limitations().contains(&Limitation::IncompleteEvent));

    // Bytes without an announced message lose the framing.
    let mut inspector = CallInspector::new(
        Protocol::Responses,
        ResponseFormat::WebSocketMessages,
        ResponseEncoding::Identity,
        InspectionLimits::default(),
    );
    inspector.feed(last);
    let report = inspector.finish(BodyEnd::Complete).inspection;
    assert_eq!(report.outcome(), None);
    assert!(report.limitations().contains(&Limitation::IncompleteEvent));
}

proptest! {
    #[test]
    fn websocket_reports_are_independent_of_piece_boundaries(
        cuts in proptest::collection::vec(any::<usize>(), 0..16),
    ) {
        let messages = websocket_messages();
        let whole = inspect_messages(&messages, &[], BodyEnd::Complete);
        let fragmented = inspect_messages(&messages, &cuts, BodyEnd::Complete);
        prop_assert_eq!(&whole, &fragmented);
    }
}

/// A Chat Completions stream with usage requested: text, then two tool
/// calls whose arguments arrive in fragments, interleaved by `index`.
fn chat_stream() -> Vec<u8> {
    let chunk = |choices: Value, usage: Value| json!({"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"fixture-gpt","choices":choices,"usage":usage});
    let mut body = sse(&[
        chunk(
            json!([{"index":0,"delta":{"role":"assistant","content":"Look"},"finish_reason":null}]),
            Value::Null,
        ),
        chunk(
            json!([{"index":0,"delta":{"content":"ing."},"finish_reason":null}]),
            Value::Null,
        ),
        chunk(
            json!([{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"read_file","arguments":""}}]},"finish_reason":null}]),
            Value::Null,
        ),
        chunk(
            json!([{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"list_dir","arguments":"{\"pa"}}]},"finish_reason":null}]),
            Value::Null,
        ),
        chunk(
            json!([{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]},"finish_reason":null}]),
            Value::Null,
        ),
        chunk(
            json!([{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"a.rs\"}"}},{"index":1,"function":{"arguments":"th\":\".\"}"}}]},"finish_reason":null}]),
            Value::Null,
        ),
        chunk(
            json!([{"index":0,"delta":{},"finish_reason":"tool_calls"}]),
            Value::Null,
        ),
        chunk(
            json!([]),
            json!({"prompt_tokens":120,"completion_tokens":30,"total_tokens":150,"prompt_tokens_details":{"cached_tokens":100},"completion_tokens_details":{"reasoning_tokens":8}}),
        ),
    ]);
    body.extend_from_slice(b"data: [DONE]\n\n");
    body
}

fn read_chunks(
    format: ResponseFormat,
    encoding: ResponseEncoding,
    chunks: &[&[u8]],
    end: BodyEnd,
) -> super::inspect::pipeline::ResponseReading {
    let mut inspector = CallInspector::new(
        Protocol::ChatCompletions,
        format,
        encoding,
        InspectionLimits::default(),
    );
    for chunk in chunks {
        inspector.feed(chunk);
    }
    inspector.finish(end)
}

#[test]
fn chat_stream_reports_model_usage_tools_and_outcome() {
    let reading = read_chunks(
        ResponseFormat::EventStream,
        ResponseEncoding::Identity,
        &[&chat_stream()],
        BodyEnd::Complete,
    );
    let report = reading.inspection;
    assert_eq!(report.coverage(), Coverage::Complete, "{report:?}");
    assert_eq!(report.terminal_event(), TerminalEvent::Seen);
    assert_eq!(report.outcome(), Some(ModelOutcome::Completed));
    assert_eq!(text(report.model()), Some("fixture-gpt"));
    assert_eq!(text(report.stop_reason()), Some("tool_calls"));
    let usage = report.usage().unwrap();
    assert_eq!(usage.accounting, InputAccounting::IncludesCache);
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.reasoning_tokens
        ),
        (Some(120), Some(30), Some(100), Some(8))
    );
    let calls: Vec<_> = report
        .tool_calls()
        .iter()
        .map(|call| {
            (
                text(call.call_id.as_ref()),
                call.kind.as_str(),
                text(call.name.as_ref()),
            )
        })
        .collect();
    assert_eq!(
        calls,
        [
            (Some("call_a"), "function", Some("read_file")),
            (Some("call_b"), "function", Some("list_dir")),
        ]
    );
    assert_eq!(report.events().understood, 9);
    let activity = reading.activity;
    assert_eq!(activity.reply, "Looking.");
    let uses: Vec<_> = activity
        .tool_uses
        .iter()
        .map(|call| (call.id.as_deref(), call.name.as_str(), call.input.clone()))
        .collect();
    assert_eq!(
        uses,
        [
            (
                Some("call_a"),
                "read_file",
                crate::activity::ToolInput::Json(json!({"path":"a.rs"}))
            ),
            (
                Some("call_b"),
                "list_dir",
                crate::activity::ToolInput::Json(json!({"path":"."}))
            ),
        ]
    );
    assert!(activity.limitations.is_empty());
}

#[test]
fn chat_streams_without_done_or_usage_say_so() {
    // Usage was not requested and the stream was cut before `[DONE]`: the
    // finish reason is reported, but no usage and no terminal marker.
    let body = sse(&[
        json!({"object":"chat.completion.chunk","model":"local","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"run","arguments":"{\"cmd\""}}]},"finish_reason":null}]}),
    ]);
    let reading = read_chunks(
        ResponseFormat::EventStream,
        ResponseEncoding::Identity,
        &[&body],
        BodyEnd::Unfinished,
    );
    let report = reading.inspection;
    assert_eq!(report.usage(), None);
    assert_eq!(report.outcome(), None);
    assert_eq!(report.terminal_event(), TerminalEvent::Missing);
    assert_eq!(
        report.limitations(),
        [Limitation::BodyUnfinished, Limitation::NoTerminalEvent]
    );
    assert_eq!(
        reading.activity.limitations,
        [crate::activity::ActivityLimitation::UnfinishedToolCall]
    );
    assert_eq!(
        reading.activity.tool_uses[0].input,
        crate::activity::ToolInput::Text("{\"cmd\"".into())
    );
}

#[test]
fn chat_errors_unknown_objects_and_reasons_are_kept() {
    let mut body = sse(&[
        json!({"object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_x","type":"future_tool","future_tool":{}}]},"finish_reason":"future_reason"}]}),
        json!({"object":"chat.completion.future"}),
        json!({"error":{"message":"overloaded","type":"server_error"}}),
    ]);
    body.extend_from_slice(b"data: [DONE]\n\n");
    let report = read_chunks(
        ResponseFormat::EventStream,
        ResponseEncoding::Identity,
        &[&body],
        BodyEnd::Complete,
    )
    .inspection;
    assert_eq!(report.outcome(), Some(ModelOutcome::Failed));
    assert_eq!(text(report.stop_reason()), Some("future_reason"));
    let unknown: Vec<_> = report
        .unknown_kinds()
        .iter()
        .map(|kind| (kind.family, kind.kind.as_str()))
        .collect();
    assert_eq!(
        unknown,
        [
            (KindFamily::OutputItem, "future_tool"),
            (KindFamily::StopReason, "future_reason"),
            (KindFamily::Event, "chat.completion.future"),
        ]
    );
    assert_eq!(report.tool_calls()[0].kind.as_str(), "future_tool");
    assert_eq!(report.events().unknown, 1);
}

#[test]
fn chat_documents_are_read_whole() {
    let document = json!({
        "id":"chatcmpl-2","object":"chat.completion","model":"deepseek-chat",
        "choices":[{"index":0,"finish_reason":"length","message":{"role":"assistant","content":"Partial","tool_calls":[
            {"id":"call_c","type":"custom","custom":{"name":"apply_patch","input":"*** Begin Patch"}}
        ]}}],
        "usage":{"prompt_tokens":50,"completion_tokens":5,"prompt_cache_hit_tokens":40,"prompt_cache_miss_tokens":10}
    })
    .to_string();
    let reading = read_chunks(
        ResponseFormat::Json,
        ResponseEncoding::Identity,
        &[document.as_bytes()],
        BodyEnd::Complete,
    );
    let report = reading.inspection;
    assert_eq!(report.coverage(), Coverage::Complete);
    assert_eq!(report.outcome(), Some(ModelOutcome::Incomplete));
    assert_eq!(text(report.model()), Some("deepseek-chat"));
    let usage = report.usage().unwrap();
    assert_eq!(
        (usage.input_tokens, usage.cache_read_tokens),
        (Some(50), Some(40))
    );
    assert_eq!(report.tool_calls()[0].kind.as_str(), "custom");
    assert_eq!(
        text(report.tool_calls()[0].name.as_ref()),
        Some("apply_patch")
    );
    assert_eq!(reading.activity.reply, "Partial");
    assert_eq!(
        reading.activity.tool_uses[0].input,
        crate::activity::ToolInput::Text("*** Begin Patch".into())
    );

    let error = read_chunks(
        ResponseFormat::Json,
        ResponseEncoding::Identity,
        &[br#"{"error":{"message":"bad key","type":"invalid_request_error"}}"#],
        BodyEnd::Complete,
    )
    .inspection;
    assert_eq!(error.outcome(), Some(ModelOutcome::Failed));
    assert_eq!(error.terminal_event(), TerminalEvent::Seen);
    // Chat Completions never streams over WebSocket or without a type.
    for format in [
        ResponseFormat::Unspecified,
        ResponseFormat::WebSocketMessages,
    ] {
        let report =
            read_chunks(format, ResponseEncoding::Identity, &[], BodyEnd::Complete).inspection;
        assert_eq!(report.limitations(), [Limitation::UnsupportedFormat]);
    }
}

proptest! {
    #[test]
    fn chat_readings_are_independent_of_chunk_boundaries(
        encoding in prop_oneof![
            Just(ResponseEncoding::Identity),
            Just(ResponseEncoding::Gzip),
        ],
        crlf in any::<bool>(),
        cuts in proptest::collection::vec(any::<usize>(), 0..32),
    ) {
        let mut body = chat_stream();
        if crlf {
            body = String::from_utf8(body).unwrap().replace('\n', "\r\n").into_bytes();
        }
        let encoded = encode(encoding, &body);
        let whole = read_chunks(ResponseFormat::EventStream, encoding, &[&encoded], BodyEnd::Complete);
        let fragmented = read_chunks(
            ResponseFormat::EventStream,
            encoding,
            &split(&encoded, &cuts),
            BodyEnd::Complete,
        );
        prop_assert_eq!(&whole, &fragmented);
        prop_assert_eq!(whole.inspection.outcome(), Some(ModelOutcome::Completed));
        prop_assert_eq!(whole.inspection.limitations(), &[][..]);
        prop_assert_eq!(whole.activity.tool_uses.len(), 2);
    }
}
