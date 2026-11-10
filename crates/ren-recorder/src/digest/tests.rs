use super::*;
use ren_harness::Harness;
use ren_protocol::activity::{
    ActivityLimitation, Reading, RequestActivity, ResponseActivity, ResultStatus, ToolInput,
    ToolResult, ToolUse,
};
use serde_json::json;

/// A synthetic key, assembled at run time so no token-shaped literal is in
/// the source.
fn key(tail: &str) -> String {
    format!("sk-ant-{}{tail}", "api03xY9z".repeat(5))
}

fn reading(secret: &str) -> Reading {
    Reading {
        request: RequestActivity {
            user_text: vec![
                format!("<system-reminder>ANTHROPIC_API_KEY={secret}</system-reminder>"),
                format!("Deploy with {secret} please"),
            ],
            tool_results: vec![ToolResult {
                id: "a".to_owned(),
                status: ResultStatus::Failed,
                output: format!("export K={secret}\n{}", "x".repeat(5_000)),
            }],
            max_output_tokens: None,
            limitations: vec![ActivityLimitation::Truncated],
        },
        response: ResponseActivity {
            tool_uses: vec![ToolUse {
                id: Some("b".to_owned()),
                kind: "tool_use".to_owned(),
                name: "Bash".to_owned(),
                input: ToolInput::Json(
                    json!({ "command": format!("curl -H 'x-api-key: {secret}' api") }),
                ),
            }],
            reply: format!("I used {secret}."),
            limitations: Vec::new(),
        },
    }
}

fn seal(harness: Option<Harness>, reading: Reading) -> CallDigest {
    let scanner = SecretScanner::new().unwrap();
    CallDigest::seal(
        InterpretedCall::new(harness, reading),
        &scanner,
        DigestBudget::STORED,
    )
}

#[test]
fn a_sealed_digest_never_holds_a_secret() {
    let secret = key("a1b2");
    let digest = seal(Some(Harness::ClaudeCode), reading(&secret));
    let stored = serde_json::to_string(&digest).unwrap();
    assert!(!stored.contains("api03"), "{stored}");
    // What each secret leaves: per field, the kind, a hint, a fingerprint.
    let fields: Vec<_> = digest
        .exposures
        .iter()
        .map(|exposure| (exposure.field, exposure.count))
        .collect();
    assert_eq!(
        fields,
        [
            // In the client's context and in the user's words.
            (SearchField::Prompt, 2),
            (SearchField::Reply, 1),
            // Once in the input; the command target repeats it uncounted.
            (SearchField::ToolInput, 1),
            (SearchField::ToolOutput, 1),
        ]
    );
    assert!(
        digest
            .exposures
            .iter()
            .all(|exposure| exposure.hint == "sk-ant-…a1b2")
    );
    assert_eq!(
        digest.prompt.as_ref().unwrap().text,
        "Deploy with «redacted: anthropic_key» please"
    );
    assert_eq!(
        digest.tool_uses[0].effects[0].target,
        "curl -H 'x-api-key: «redacted: anthropic_key»' api"
    );
}

#[test]
fn texts_are_cut_to_the_budget_and_say_how_much_was_left() {
    let digest = seal(Some(Harness::ClaudeCode), reading(&key("c3d4")));
    let output = &digest.tool_results[0].output;
    assert_eq!(output.text.chars().count(), 2_000);
    assert!(output.omitted_chars > 3_000);
    assert_eq!(digest.tool_results[0].status, ResultStatus::Failed);
    assert_eq!(digest.version, CallDigest::VERSION);
    assert_eq!(digest.limitations, [ActivityLimitation::Truncated]);
    let fields: Vec<_> = digest.search.iter().map(|text| text.field).collect();
    assert_eq!(
        fields,
        [
            SearchField::Prompt,
            SearchField::Reply,
            SearchField::ToolInput,
            SearchField::ToolOutput
        ]
    );
    // The searchable tool input names the tool and its targets.
    assert!(digest.search[2].text.starts_with("Bash curl -H"));

    // Nothing to search or show is left out rather than stored empty.
    let empty = seal(None, Reading::default());
    assert_eq!(empty.prompt, None);
    assert_eq!(empty.reply, None);
    assert!(empty.search.is_empty());
    assert!(empty.exposures.is_empty());
}

#[test]
fn a_digest_round_trips_through_its_stored_form() {
    let digest = seal(Some(Harness::Codex), reading(&key("e5f6")));
    let stored = serde_json::to_string(&digest).unwrap();
    assert_eq!(serde_json::from_str::<CallDigest>(&stored).unwrap(), digest);
}

#[test]
fn narrowing_an_excerpt_keeps_count_of_what_was_left_out() {
    let stored = Excerpt::cut(&"a".repeat(50), 20);
    let narrow = stored.narrow(5);
    assert_eq!(narrow.text, "aaaaa");
    assert_eq!(narrow.omitted_chars, 45);
    assert_eq!(narrow.total_chars(), 50);
    assert_eq!(stored.narrow(100), stored);
}
