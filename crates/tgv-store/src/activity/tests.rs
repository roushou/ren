use super::*;
use crate::view::{AttributionView, SessionHintView};
use serde_json::json;
use tgv_harness::attribution::HintSource;
use tgv_protocol::{
    activity::{ToolResult, ToolUse},
    route::{Endpoint, Method, Provider},
};
use tgv_recorder::{capture::CallState, id::ChannelId};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn view(n: u128, source: HintSource) -> CallView {
    CallView {
        id: CallId::restore(id(n)),
        channel: ChannelId::restore(id(100 + n)),
        started_at: UnixMillis::from_millis(u64::try_from(n).unwrap() * 1000),
        ended_at: Some(UnixMillis::from_millis(
            u64::try_from(n).unwrap() * 1000 + 500,
        )),
        provider: Provider::Anthropic.into(),
        endpoint: Endpoint::Messages.into(),
        method: Method::Post.into(),
        config_revision: String::new(),
        session: None,
        turn: None,
        attribution: AttributionView {
            hint: Some(SessionHintView {
                source: source.into(),
                session: "s".to_owned(),
                turn: Some("t".to_owned()),
                thread: None,
            }),
            issues: Vec::new(),
        },
        state: CallState::Finished.into(),
        terminal: None,
        status: Some(200),
        model: Some("model".to_owned()),
        outcome: None,
        stop_reason: None,
        tool_names: Vec::new(),
        usage: None,
        capture_complete: true,
        request: None,
        response: None,
        rejection: None,
        operator_revision: None,
        plan: None,
    }
}

fn tool(id: &str, name: &str, input: serde_json::Value) -> ToolUse {
    ToolUse {
        id: Some(id.to_owned()),
        kind: "tool_use".to_owned(),
        name: name.to_owned(),
        input: ToolInput::Json(input),
    }
}

fn result(id: &str, status: ResultStatus, output: &str) -> ToolResult {
    ToolResult {
        id: id.to_owned(),
        status,
        output: output.to_owned(),
    }
}

fn head() -> TurnHead {
    TurnHead {
        turn: TurnId::restore(id(7)),
        session: SessionId::restore(id(8)),
        native_id: "prompt-1".to_owned(),
    }
}

#[test]
fn a_turn_joins_prompt_tool_calls_results_and_effects() {
    let first = ReadCall {
        view: view(1, HintSource::ClaudeCodeHeaders),
        request: RequestActivity {
            user_text: vec![
                "<system-reminder>context</system-reminder>".to_owned(),
                "Make the tests pass".to_owned(),
            ],
            // Answers a tool call from an earlier turn.
            tool_results: vec![result("old", ResultStatus::Succeeded, "x")],
            limitations: Vec::new(),
        },
        response: ResponseActivity {
            tool_uses: vec![
                tool("a", "Read", json!({ "file_path": "src/lib.rs" })),
                tool("b", "Bash", json!({ "command": "cargo test" })),
            ],
            reply: "Looking.".to_owned(),
            limitations: Vec::new(),
        },
    };
    let second = ReadCall {
        view: view(2, HintSource::ClaudeCodeHeaders),
        request: RequestActivity {
            user_text: vec!["<system-reminder>more</system-reminder>".to_owned()],
            tool_results: vec![
                result("a", ResultStatus::Succeeded, "fn main() {}"),
                result("b", ResultStatus::Failed, &"error ".repeat(200)),
            ],
            limitations: Vec::new(),
        },
        response: ResponseActivity {
            tool_uses: vec![
                tool("c", "Edit", json!({ "file_path": "src/lib.rs" })),
                tool("d", "Read", json!({ "file_path": "src/lib.rs" })),
            ],
            reply: String::new(),
            limitations: vec![ActivityLimitation::UnfinishedToolCall],
        },
    };
    let third = ReadCall {
        view: view(3, HintSource::ClaudeCodeHeaders),
        request: RequestActivity {
            user_text: Vec::new(),
            tool_results: vec![result("c", ResultStatus::Succeeded, "ok")],
            limitations: Vec::new(),
        },
        response: ResponseActivity::default(),
    };
    let turn = TurnActivity::assemble(
        head(),
        vec![first, second, third],
        0,
        ActivityBudget::DEFAULT,
    );
    assert_eq!(turn.client, Some(Harness::ClaudeCode));
    assert_eq!(
        turn.prompt.as_ref().map(|prompt| prompt.text.as_str()),
        Some("Make the tests pass")
    );
    assert_eq!(turn.started_at, Some(UnixMillis::from_millis(1000)));
    assert_eq!(turn.ended_at, Some(UnixMillis::from_millis(3500)));
    assert_eq!(turn.files_read, ["src/lib.rs"]);
    assert_eq!(turn.files_written, ["src/lib.rs"]);
    assert_eq!(turn.commands, ["cargo test"]);
    assert_eq!(turn.tool_calls, 4);
    assert_eq!(turn.failed_tool_calls, 1);
    assert_eq!(turn.unanswered_tool_calls, 1);

    let calls = &turn.calls;
    assert_eq!(calls[0].earlier_results, 1);
    assert_eq!(calls[1].earlier_results, 0);
    assert!(calls[1].prompt.is_none());
    let bash = &calls[0].tool_calls[1];
    assert_eq!(bash.effects[0].kind, EffectKind::RunCommand);
    let failed = bash.result.as_ref().unwrap();
    assert_eq!(failed.status, ResultStatus::Failed);
    assert_eq!(failed.output.text.chars().count(), 400);
    assert_eq!(failed.output.omitted_chars, 800);
    assert!(calls[1].tool_calls[1].result.is_none());
    assert_eq!(
        calls[1].limitations,
        [ActivityLimitation::UnfinishedToolCall]
    );
    assert_eq!(calls[0].reply.as_ref().unwrap().text, "Looking.");
    assert!(calls[1].reply.is_none());
}

#[test]
fn codex_exit_codes_and_unattributed_calls() {
    let codex = ReadCall {
        view: view(1, HintSource::CodexHeaders),
        request: RequestActivity {
            user_text: vec!["Run it".to_owned()],
            tool_results: Vec::new(),
            limitations: Vec::new(),
        },
        response: ResponseActivity {
            tool_uses: vec![tool(
                "x",
                "shell",
                json!({ "command": ["bash", "-lc", "make"] }),
            )],
            reply: String::new(),
            limitations: Vec::new(),
        },
    };
    let answer = ReadCall {
        view: view(2, HintSource::CodexHeaders),
        request: RequestActivity {
            user_text: Vec::new(),
            tool_results: vec![result(
                "x",
                ResultStatus::Unreported,
                "Exit code: 2\nOutput:\nfail",
            )],
            limitations: Vec::new(),
        },
        response: ResponseActivity::default(),
    };
    let turn = TurnActivity::assemble(head(), vec![codex, answer], 3, ActivityBudget::DEFAULT);
    assert_eq!(turn.commands, ["make"]);
    assert_eq!(turn.failed_tool_calls, 1);
    assert_eq!(turn.omitted_calls, 3);

    // Without attribution, text is kept as sent and tools have no effects.
    let mut bare = view(1, HintSource::PiExtensionHeaders);
    bare.attribution = AttributionView::default();
    let call = ReadCall {
        view: bare,
        request: RequestActivity {
            user_text: vec!["<system-reminder>kept</system-reminder>".to_owned()],
            tool_results: Vec::new(),
            limitations: Vec::new(),
        },
        response: ResponseActivity {
            tool_uses: vec![tool("y", "Read", json!({ "file_path": "a" }))],
            reply: String::new(),
            limitations: Vec::new(),
        },
    }
    .activity(ActivityBudget::DEFAULT);
    assert_eq!(
        call.prompt.unwrap().text,
        "<system-reminder>kept</system-reminder>"
    );
    assert!(call.tool_calls[0].effects.is_empty());
}

#[test]
fn excerpts_count_what_they_leave_out() {
    assert_eq!(
        Excerpt::cut("héllo", 2),
        Excerpt {
            text: "hé".to_owned(),
            omitted_chars: 3
        }
    );
    assert_eq!(Excerpt::cut("", 2).omitted_chars, 0);
}
