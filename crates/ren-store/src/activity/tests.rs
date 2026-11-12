use super::*;
use crate::view::{AttributionView, SessionHintView};
use ren_harness::tools::InterpretedCall;
use ren_protocol::activity::{Reading, RequestActivity, ResponseActivity};
use ren_recorder::digest::{CallDigest, DigestBudget};
use ren_recorder::secrets::SecretScanner;

/// A call as its bodies read, sealed into the digest the recorder would
/// store for it.
#[derive(Clone)]
struct ReadCall {
    view: CallView,
    request: RequestActivity,
    response: ResponseActivity,
}

impl ReadCall {
    fn read(self) -> super::ReadCall {
        let harness = self
            .view
            .attribution
            .hint
            .as_ref()
            .and_then(|hint| hint.source.known())
            .map(|source| source.harness());
        let digest = CallDigest::seal(
            InterpretedCall::new(
                harness,
                Reading {
                    request: self.request,
                    response: self.response,
                },
            ),
            &SecretScanner::new().unwrap(),
            DigestBudget::STORED,
        );
        super::ReadCall {
            view: self.view,
            digest: Some(digest),
        }
    }

    fn activity(self, budget: ActivityBudget) -> CallActivity {
        self.read().activity(budget)
    }
}

/// A turn from calls as their bodies read.
fn assemble(
    head: TurnHead,
    calls: Vec<ReadCall>,
    omitted: u64,
    budget: ActivityBudget,
    _scanner: &SecretScanner,
) -> TurnActivity {
    TurnActivity::assemble(
        head,
        calls.into_iter().map(ReadCall::read).collect(),
        omitted,
        budget,
    )
}
use ren_harness::attribution::HintSource;
use ren_protocol::activity::ToolInput;
use ren_protocol::{
    activity::{ToolResult, ToolUse},
    route::{Endpoint, Method, Provider},
};
use ren_recorder::{capture::CallState, id::ChannelId};
use serde_json::json;
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
        wire: Some(Provider::Anthropic.native_api().wire().into()),
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
        caller: None,
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
        purpose: None,
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

/// A Claude Code turn: a prompt, reads, a failed command, an edit, and a
/// tool call left unanswered.
fn worked_calls() -> Vec<ReadCall> {
    let first = ReadCall {
        view: view(1, HintSource::ClaudeCodeHeaders),
        request: RequestActivity {
            max_output_tokens: None,
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
            max_output_tokens: None,
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
            max_output_tokens: None,
            user_text: Vec::new(),
            tool_results: vec![result("c", ResultStatus::Succeeded, "ok")],
            limitations: Vec::new(),
        },
        response: ResponseActivity::default(),
    };
    vec![first, second, third]
}

#[test]
fn a_turn_joins_prompt_tool_calls_results_and_effects() {
    let turn = assemble(
        head(),
        worked_calls(),
        0,
        ActivityBudget::DEFAULT,
        &ren_recorder::secrets::SecretScanner::new().unwrap(),
    );
    assert_eq!(turn.client, Some(Harness::ClaudeCode.into()));
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
    assert_eq!(bash.effects[0].kind, EffectKind::RunCommand.into());
    let failed = bash.result.as_ref().unwrap();
    assert_eq!(failed.status, ResultStatus::Failed.into());
    assert_eq!(failed.output.text.chars().count(), 400);
    assert_eq!(failed.output.omitted_chars, 800);
    assert!(calls[1].tool_calls[1].result.is_none());
    assert_eq!(
        calls[1].limitations,
        [ActivityLimitation::UnfinishedToolCall.into()]
    );
    assert_eq!(calls[0].reply.as_ref().unwrap().text, "Looking.");
    assert!(calls[1].reply.is_none());
}

#[test]
fn codex_exit_codes_and_unattributed_calls() {
    let codex = ReadCall {
        view: view(1, HintSource::CodexHeaders),
        request: RequestActivity {
            max_output_tokens: None,
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
            max_output_tokens: None,
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
    let turn = assemble(
        head(),
        vec![codex, answer],
        3,
        ActivityBudget::DEFAULT,
        &ren_recorder::secrets::SecretScanner::new().unwrap(),
    );
    assert_eq!(turn.commands, ["make"]);
    assert_eq!(turn.failed_tool_calls, 1);
    assert_eq!(turn.omitted_calls, 3);

    // Without attribution, text is kept as sent and tools have no effects.
    let mut bare = view(1, HintSource::PiExtensionHeaders);
    bare.attribution = AttributionView::default();
    let call = ReadCall {
        view: bare,
        request: RequestActivity {
            max_output_tokens: None,
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

#[test]
fn a_turn_with_a_call_still_open_has_not_ended() {
    let call = |n: u128, ended: Option<u64>| {
        let mut view = view(n, HintSource::ClaudeCodeHeaders);
        view.ended_at = ended.map(UnixMillis::from_millis);
        ReadCall {
            view,
            request: RequestActivity::default(),
            response: ResponseActivity::default(),
        }
    };
    let scanner = ren_recorder::secrets::SecretScanner::new().unwrap();
    let assemble = |calls| assemble(head(), calls, 0, ActivityBudget::DEFAULT, &scanner).ended_at;
    assert_eq!(
        assemble(vec![call(1, Some(1500)), call(2, Some(2500))]),
        Some(UnixMillis::from_millis(2500))
    );
    // The last call is still running: the turn's end is unknown, not the
    // first call's end.
    assert_eq!(assemble(vec![call(1, Some(1500)), call(2, None)]), None);
}

mod report {
    use super::*;
    use crate::activity::report::{Finding, SessionReport};
    use ren_protocol::inspect::InputAccounting;
    use ren_recorder::inspection::UsageRecord;

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
        let kinds: Vec<_> = report
            .findings
            .iter()
            .map(|finding| finding.kind().known())
            .collect();
        assert_eq!(
            kinds,
            [
                Some(report::FindingKind::ContextGrowth),
                Some(report::FindingKind::CacheRebuild)
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
    fn a_turn_row_with_a_call_still_open_has_not_ended() {
        let mut open = call(2, Some(1), None);
        open.ended_at = None;
        let calls = [call(1, Some(1), None), open, call(5, Some(2), None)];
        let report = SessionReport::build(SessionId::restore(id(9)), &calls, 0);
        assert_eq!(report.turns[0].ended_at, None);
        assert_eq!(
            report.turns[1].ended_at,
            Some(UnixMillis::from_millis(6_000))
        );
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
        let turn = assemble(
            head(),
            super::failing_calls(),
            0,
            ActivityBudget::DEFAULT,
            &ren_recorder::secrets::SecretScanner::new().unwrap(),
        );
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

#[test]
fn secrets_a_turn_carried_are_findings_without_the_secret() {
    use crate::activity::report::Finding;
    // Assembled at run time: no token-shaped literal in the source.
    let key = format!("sk-ant-{}a1b2", "api03xY9z".repeat(5));
    let call = ReadCall {
        view: view(1, HintSource::ClaudeCodeHeaders),
        request: RequestActivity {
            max_output_tokens: None,
            // Client context is sent to the provider too: it is scanned.
            user_text: vec![
                format!("<system-reminder>ANTHROPIC_API_KEY={key}</system-reminder>"),
                "Deploy it".to_owned(),
            ],
            tool_results: vec![result(
                "a",
                ResultStatus::Succeeded,
                &format!("export K={key}"),
            )],
            limitations: Vec::new(),
        },
        response: ResponseActivity::default(),
    };
    let call = call.read();
    let turn = TurnActivity::assemble(head(), vec![call], 0, ActivityBudget::DEFAULT);
    let json = serde_json::to_string(&turn.findings).unwrap();
    assert!(!json.contains("api03"), "{json}");
    let fields: Vec<_> = turn
        .findings
        .iter()
        .map(|finding| match finding {
            Finding::SecretExposure(exposure) => exposure.field.label().to_owned(),
            other @ (Finding::CacheRebuild(_)
            | Finding::ContextGrowth(_)
            | Finding::LargeToolOutput(_)
            | Finding::RepeatedFailure(_)
            | Finding::Unknown(_)) => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(fields, ["prompt", "tool_output"]);
}

/// A turn whose command fails three times and whose read returns a large
/// output.
fn failing_calls() -> Vec<ReadCall> {
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
            max_output_tokens: None,
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
    vec![first, second]
}

/// The turn view's JSON is what the read API returns, and reads back into
/// the same view: pinned, so a change to the view's types cannot change it
/// unnoticed.
#[test]
fn turn_views_round_trip_through_their_json() {
    let scanner = SecretScanner::new().unwrap();
    let turns = [worked_calls(), failing_calls()]
        .map(|calls| assemble(head(), calls, 0, ActivityBudget::DEFAULT, &scanner));
    let sent = include_str!("../../tests/fixtures/turn-activity.json");
    let json = serde_json::to_string_pretty(&turns).unwrap();
    assert_eq!(json.trim_end(), sent.trim_end());
    let read: Vec<TurnActivity> = serde_json::from_str(sent).unwrap();
    assert_eq!(read, turns);
}

/// Labels a newer daemon sends, and findings of a kind this binary does
/// not know, are kept as sent rather than failing the read.
#[test]
fn turn_views_keep_labels_they_do_not_know() {
    let mut sent: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/turn-activity.json")).unwrap();
    let turn = &mut sent[0];
    turn["client"] = json!("a_newer_client");
    turn["findings"] = json!([{ "kind": "a_newer_finding", "call": "c", "size": 3 }]);
    let call = &mut turn["calls"][0];
    call["limitations"] = json!(["a_newer_limitation"]);
    call["tool_calls"][0]["effects"][0]["kind"] = json!("a_newer_effect");
    call["tool_calls"][0]["result"]["status"] = json!("a_newer_status");

    let turns: Vec<TurnActivity> = serde_json::from_value(serde_json::json!(sent)).unwrap();
    let turn = &turns[0];
    assert_eq!(turn.client.as_ref().unwrap().label(), "a_newer_client");
    let Finding::Unknown(finding) = &turn.findings[0] else {
        panic!("{:?}", turn.findings[0]);
    };
    assert_eq!(finding.kind().label(), "a_newer_finding");
    assert_eq!(finding.detail()["size"], json!(3));
    let tool = &turn.calls[0].tool_calls[0];
    assert_eq!(tool.effects[0].kind.known(), None);
    assert_eq!(tool.result.as_ref().unwrap().status.known(), None);
    assert_eq!(turn.calls[0].limitations[0].known(), None);
    assert_eq!(serde_json::to_value(&turns).unwrap(), json!(sent));
}
