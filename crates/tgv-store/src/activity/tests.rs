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

mod report {
    use super::*;
    use crate::activity::report::{Finding, SessionReport};
    use tgv_protocol::inspect::InputAccounting;
    use tgv_recorder::inspection::UsageRecord;

    /// A call at `second`, lasting one second, in `turn`, with Anthropic
    /// usage: new input, cache read, cache written, output.
    fn call(second: u64, turn: Option<u128>, usage: Option<(u64, u64, u64, u64)>) -> CallView {
        let mut call = view(u128::from(second), HintSource::ClaudeCodeHeaders);
        call.started_at = UnixMillis::from_millis(second * 1000);
        call.ended_at = Some(UnixMillis::from_millis(second * 1000 + 1000));
        call.turn = turn.map(|turn| TurnId::restore(id(turn)));
        call.usage = usage.map(|(input, read, written, output)| UsageRecord {
            accounting: InputAccounting::ExcludesCache.into(),
            input_tokens: Some(input),
            output_tokens: Some(output),
            cache_read_tokens: Some(read),
            cache_creation_tokens: Some(written),
            reasoning_tokens: None,
        });
        call
    }

    #[test]
    fn rebuilds_after_idle_and_growth_are_found_but_subagents_are_not() {
        let calls = [
            // A small side call opens the session; the first real call's
            // cache write after it is neither a rebuild nor growth.
            call(0, None, Some((1_200, 0, 0, 20))),
            // Turn 1: the cache is first written, then read.
            call(1, Some(1), Some((2, 0, 50_000, 100))),
            call(10, Some(1), Some((2, 50_000, 1_000, 100))),
            // A subagent's first call: small, its own cache.
            call(12, Some(1), Some((2, 4_000, 15_000, 50))),
            call(20, Some(1), Some((2, 51_000, 30_000, 300))),
            // A side call outside any turn, without usage.
            call(30, None, None),
            // Turn 2, an hour later: everything is written again.
            call(3_600, Some(2), Some((4, 0, 82_000, 200))),
            call(3_610, Some(2), Some((2, 82_000, 1_000, 200))),
        ];
        let report = SessionReport::build(SessionId::restore(id(9)), &calls, 0);
        assert_eq!(report.totals.calls, 8);
        assert_eq!(report.totals.calls_with_usage, 7);
        assert_eq!(report.totals.peak_context_tokens, 83_002);
        let kinds: Vec<_> = report.findings.iter().map(Finding::kind).collect();
        assert_eq!(
            kinds,
            [
                report::FindingKind::ContextGrowth,
                report::FindingKind::CacheRebuild
            ]
        );
        let Finding::ContextGrowth(growth) = &report.findings[0] else {
            panic!("{:?}", report.findings[0]);
        };
        assert_eq!(growth.earlier_peak_tokens, 51_002);
        assert_eq!(growth.context_tokens, 81_002);
        let Finding::CacheRebuild(rebuild) = &report.findings[1] else {
            panic!("{:?}", report.findings[1]);
        };
        assert_eq!(rebuild.call, CallId::restore(id(3_600)));
        assert_eq!(rebuild.cache_written_tokens, 82_000);
        assert_eq!(rebuild.earlier_peak_tokens, 81_002);
        assert_eq!(rebuild.idle_before_ms, Some(3_569_000));

        assert_eq!(report.turns.len(), 2);
        let first = &report.turns[0];
        assert_eq!(first.call_count, 4);
        // It starts as the side call ends.
        assert_eq!(first.idle_before_ms, Some(0));
        assert_eq!(first.context_start_tokens, Some(50_002));
        assert_eq!(first.context_end_tokens, Some(81_002));
        assert_eq!(first.cache_written_tokens, 96_000);
        let second = &report.turns[1];
        assert_eq!(second.idle_before_ms, Some(3_569_000));
        // 82,000 read of 82,004 + 83,002 seen.
        assert_eq!(second.cache_hit_percent, Some(49));

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["findings"][1]["kind"], "cache_rebuild");
        assert_eq!(json["findings"][1]["cache_written_tokens"], 82_000);
    }

    #[test]
    fn openai_input_already_counts_cached_tokens() {
        let mut call = call(0, Some(1), None);
        call.usage = Some(UsageRecord {
            accounting: InputAccounting::IncludesCache.into(),
            input_tokens: Some(10_000),
            output_tokens: Some(5),
            cache_read_tokens: Some(9_000),
            cache_creation_tokens: None,
            reasoning_tokens: None,
        });
        let metrics = report::CallMetrics::of(&call);
        assert_eq!(metrics.context_tokens, Some(10_000));
        assert_eq!(metrics.duration_ms, Some(1_000));
        let report = SessionReport::build(SessionId::restore(id(9)), &[call], 2);
        assert_eq!(report.totals.cache_hit_percent, Some(90));
        assert_eq!(report.omitted_calls, 2);
    }

    #[test]
    fn turns_report_large_outputs_and_repeated_failures() {
        let tools: Vec<_> = (0..3)
            .map(|n| tool(&format!("t{n}"), "Bash", json!({ "command": "cargo test" })))
            .chain([tool("big", "Read", json!({ "file_path": "big.log" }))])
            .collect();
        let first = ReadCall {
            view: view(1, HintSource::ClaudeCodeHeaders),
            request: RequestActivity::default(),
            response: ResponseActivity {
                tool_uses: tools,
                reply: String::new(),
                limitations: Vec::new(),
            },
        };
        let second = ReadCall {
            view: view(2, HintSource::ClaudeCodeHeaders),
            request: RequestActivity {
                user_text: Vec::new(),
                tool_results: vec![
                    result("t0", ResultStatus::Failed, "boom"),
                    result("t1", ResultStatus::Failed, "boom"),
                    result("t2", ResultStatus::Failed, "boom"),
                    result("big", ResultStatus::Succeeded, &"x".repeat(25_000)),
                ],
                limitations: Vec::new(),
            },
            response: ResponseActivity::default(),
        };
        let turn = TurnActivity::assemble(head(), vec![first, second], 0, ActivityBudget::DEFAULT);
        let json = serde_json::to_value(&turn.findings).unwrap();
        let first_call = CallId::restore(id(1)).to_string();
        let failed_calls = vec![first_call.clone(); 3];
        assert_eq!(
            json,
            json!([
                {
                    "kind": "large_tool_output",
                    "call": first_call,
                    "tool": "Read",
                    "tool_call_id": "big",
                    "target": "big.log",
                    "output_chars": 25_000,
                },
                {
                    "kind": "repeated_failure",
                    "tool": "Bash",
                    "target": "cargo test",
                    "failures": 3,
                    "calls": failed_calls,
                },
            ])
        );
        assert_eq!(turn.calls[0].metrics.duration_ms, Some(500));
    }
}
