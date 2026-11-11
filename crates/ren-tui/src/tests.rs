//! The view against reads shaped as the management API sends them.

use super::*;
use ren_harness::Harness;
use ren_recorder::id::RunId;
use serde_json::{Value, json};

const T0: u64 = 1_800_000_000_000;

fn at(seconds: u64) -> UnixMillis {
    UnixMillis::from_millis(T0 + seconds * 1000)
}

/// An identifier whose first eight digits, the part a pane shows, tell
/// fixtures apart.
fn uuid(n: u128) -> String {
    uuid::Uuid::from_u128((n << 96) | n).to_string()
}

fn session_id(n: u128) -> SessionId {
    uuid(n).parse().unwrap()
}

fn turn_id(n: u128) -> TurnId {
    uuid(n).parse().unwrap()
}

fn call_id(n: u128) -> CallId {
    uuid(n).parse().unwrap()
}

/// A session, `source` naming its client, last seen at `seconds`.
fn session(n: u128, source: &str, seconds: u64) -> SessionView {
    serde_json::from_value(json!({
        "id": uuid(n),
        "provider": if source.starts_with("codex") { "openai" } else { "anthropic" },
        "source": source,
        "native_id": "native",
        "first_seen": T0,
        "last_seen": T0 + seconds * 1000,
        "call_count": 1,
        "turn_count": 1,
        "error_count": 0,
        "tokens": { "input": null, "output": 1_000 + n * 713, "cache_read": null }
    }))
    .unwrap()
}

/// A finished, completed model call of session `s` and turn `t`, from
/// `start` to `end` seconds, with `fields` laid over it.
fn call(n: u128, s: u128, t: Option<u128>, start: u64, end: u64, fields: Value) -> CallView {
    let mut call = json!({
        "id": uuid(n),
        "channel": uuid(1000 + n),
        "started_at": T0 + start * 1000,
        "ended_at": T0 + end * 1000,
        "provider": "anthropic",
        "endpoint": "messages",
        "method": "post",
        "config_revision": "r1",
        "session": uuid(s),
        "turn": t.map(uuid),
        "attribution": {
            "hint": { "source": "claude_code_headers", "session": "s", "turn": null, "thread": null },
            "issues": []
        },
        "state": "finished",
        "terminal": "body_complete",
        "status": 200,
        "model": "model-a",
        "outcome": "completed",
        "tool_names": [],
        "capture_complete": true
    });
    let (Value::Object(base), Value::Object(over)) = (&mut call, fields) else {
        panic!("objects");
    };
    base.extend(over);
    serde_json::from_value(call).unwrap()
}

/// An open call: sent, with a response head when `head` is set.
fn open(n: u128, s: u128, t: Option<u128>, start: u64, head: bool) -> CallView {
    call(
        n,
        s,
        t,
        start,
        start,
        json!({
            "ended_at": null, "state": "open", "terminal": null, "outcome": null,
            "status": if head { json!(200) } else { Value::Null }
        }),
    )
}

fn page(calls: Vec<CallView>) -> CallPage {
    CallPage { calls, next: None }
}

fn snapshot(seconds: u64, sessions: Vec<SessionView>, calls: Vec<CallView>) -> Input {
    let (open, recent): (Vec<_>, Vec<_>) = calls
        .into_iter()
        .partition(|call| call.state.known() == Some(CallState::Open));
    Input::Snapshot(Snapshot {
        read_at: at(seconds),
        sessions,
        open: page(open),
        recent: page(recent),
    })
}

/// A wide terminal: three panes across, one down.
fn tui() -> Tui {
    Tui::new(Size {
        width: 3 * Grid::PANE_WIDTH,
        height: Grid::PANE_HEIGHT + Grid::STATUS_LINES,
    })
}

fn pane(wall: &Wall, index: usize) -> &SessionPane {
    match &wall.panes[index] {
        Pane::Session(pane) => pane,
        Pane::Free => panic!("pane {index} is free"),
    }
}

/// A tool call as a turn's activity lists it.
fn tool(id: &str, name: &str, effect: Option<(&str, &str)>, result: Option<&str>) -> Value {
    json!({
        "id": id,
        "name": name,
        "effects": effect.map_or(json!([]), |(kind, target)| json!([{ "kind": kind, "target": target }])),
        "input": { "text": "{}", "omitted_chars": 0 },
        "result": result.map(|status| json!({
            "status": status,
            "output": { "text": format!("\n  {status} output\nmore"), "omitted_chars": 0 }
        }))
    })
}

fn activity_call(n: u128, reply: Option<&str>, tools: Vec<Value>, limitations: &[&str]) -> Value {
    json!({
        "call": uuid(n),
        "started_at": T0,
        "metrics": {},
        "prompt": null,
        "tool_calls": tools,
        "earlier_results": 0,
        "reply": reply.map(|text| json!({ "text": text, "omitted_chars": 0 })),
        "limitations": limitations
    })
}

/// `call` as a request that starts with the user's own words.
fn prompted(prompt: &str, mut call: Value) -> Value {
    call["prompt"] = json!({ "text": prompt, "omitted_chars": 0 });
    call
}

fn turn(t: u128, s: u128, commands: &[&str], calls: Vec<Value>) -> Input {
    Input::Turn(Box::new(
        serde_json::from_value(json!({
            "turn": uuid(t),
            "session": uuid(s),
            "native_id": "prompt",
            "client": "claude_code",
            "files_read": ["a"],
            "files_written": [],
            "commands": commands,
            "tool_calls": 3,
            "failed_tool_calls": 1,
            "unanswered_tool_calls": 0,
            "findings": [],
            "calls": calls,
            "omitted_calls": 0
        }))
        .unwrap(),
    ))
}

#[test]
fn nothing_read_yet_is_said_so() {
    let wall = tui().wall(at(0));
    assert_eq!(wall.status.reading, Reading::Pending);
    assert!(wall.panes.iter().all(|pane| *pane == Pane::Free));
}

#[test]
fn a_snapshot_reads_the_open_calls_and_the_window() {
    let query = Tui::snapshot_query(at(600));
    assert_eq!(query.open.filter.status, Some(StatusFilter::Active));
    assert_eq!(query.open.filter.kind, Some(KindFilter::Model));
    assert_eq!(query.recent.filter.time.from(), Some(at(300)));
    assert_eq!(query.recent.limit, PageLimit::LARGEST);
}

#[test]
fn sessions_with_open_calls_come_first_and_keep_their_panes() {
    let mut tui = tui();
    let sessions = vec![
        session(1, "claude_code_headers", 100),
        session(2, "codex_headers", 200),
        session(3, "pi_extension_headers", 50),
        session(4, "opencode_headers", 300),
    ];
    let calls = vec![open(31, 3, None, 40, true)];
    let _ = tui.apply(snapshot(300, sessions.clone(), calls));
    let wall = tui.wall(at(300));
    let shown: Vec<SessionId> = (0..3).map(|index| pane(&wall, index).session).collect();
    // Open first, then the most recently seen; one is left out.
    assert_eq!(shown, [session_id(3), session_id(4), session_id(2)]);
    assert_eq!(wall.status.hidden, 1);
    assert_eq!(wall.status.sessions, 4);

    // Session 3's call ends and it goes quiet: 4 and 2 stay where they are.
    let _ = tui.apply(snapshot(
        400,
        sessions,
        vec![call(31, 3, None, 40, 41, json!({}))],
    ));
    let wall = tui.wall(at(400));
    assert_eq!(pane(&wall, 0).session, session_id(1));
    assert_eq!(pane(&wall, 1).session, session_id(4));
    assert_eq!(pane(&wall, 2).session, session_id(2));
    assert_eq!(wall.status.hidden, 0);
}

#[test]
fn the_grid_is_sized_to_the_active_sessions() {
    let mut tui = tui();
    let _ = tui.apply(snapshot(
        1000,
        vec![
            session(1, "claude_code_headers", 990),
            session(2, "codex_headers", 10),
        ],
        vec![],
    ));
    // One active session fills the screen, however wide the terminal.
    let wall = tui.wall(at(1000));
    assert_eq!(
        wall.grid,
        Grid {
            columns: 1,
            rows: 1
        }
    );
    assert_eq!(pane(&wall, 0).session, session_id(1));
    // None: one pane, free.
    let _ = tui.apply(snapshot(
        5000,
        vec![session(1, "claude_code_headers", 990)],
        vec![],
    ));
    let wall = tui.wall(at(5000));
    assert_eq!(wall.panes, [Pane::Free]);
}

#[test]
fn a_grid_leaves_as_few_panes_empty_as_it_can() {
    let size = Size {
        width: 240,
        height: 61,
    };
    let shapes: Vec<(u16, u16)> = [1, 2, 3, 4, 5, 6, 9, 40]
        .map(|sessions| {
            let grid = Grid::for_sessions(size, sessions);
            (grid.columns, grid.rows)
        })
        .into();
    assert_eq!(
        shapes,
        [
            (1, 1),
            (2, 1),
            (3, 1),
            (2, 2),
            (3, 2),
            (3, 2),
            (3, 3),
            (4, 4)
        ]
    );
}

#[test]
fn parallel_open_calls_are_counted_and_the_newest_sets_the_phase() {
    let mut tui = tui();
    let _ = tui.apply(snapshot(
        100,
        vec![session(1, "claude_code_headers", 100)],
        vec![
            open(11, 1, Some(5), 90, true),
            open(12, 1, Some(5), 95, false),
        ],
    ));
    let wall = tui.wall(at(100));
    let pane = pane(&wall, 0);
    assert_eq!(pane.open, 2);
    assert_eq!(pane.phase, Phase::Awaiting { since: at(95) });
    assert_eq!(wall.status.responding, 1);
    assert_eq!(wall.status.awaiting, 1);
}

#[test]
fn how_the_last_call_ended_sets_the_phase() {
    let phase = |fields: Value, now: u64| {
        let mut tui = tui();
        let _ = tui.apply(snapshot(
            now,
            vec![session(1, "claude_code_headers", now)],
            vec![call(11, 1, None, 10, 20, fields)],
        ));
        pane(&tui.wall(at(now)), 0).phase.clone()
    };
    assert_eq!(
        phase(json!({ "tool_names": ["Read"] }), 30),
        Phase::ClientsTurn { since: at(20) }
    );
    assert_eq!(phase(json!({}), 30), Phase::YourTurn { since: at(20) });
    assert_eq!(
        phase(json!({}), 200),
        Phase::Idle {
            since: Some(at(20))
        }
    );
    // A restart's recovery and a state this binary does not know stay
    // unknown: never success, never failure.
    let recovered = phase(json!({ "state": "recovered", "outcome": null }), 30);
    assert_eq!(
        recovered,
        Phase::Stopped {
            since: at(20),
            end: CallEnd::Recovered,
            status: Some(200)
        }
    );
    let unknown = phase(json!({ "state": "a_newer_state", "outcome": null }), 30);
    assert!(matches!(
        unknown,
        Phase::Stopped {
            end: CallEnd::Unknown,
            ..
        }
    ));
    let failed = phase(json!({ "status": 529, "outcome": null }), 30);
    let Phase::Stopped { end, status, .. } = failed else {
        panic!("{failed:?}");
    };
    assert_eq!(end.label(status), "error 529");
    assert_eq!(end.tone(), Tone::Error);
}

#[test]
fn only_how_the_newest_call_went_is_an_alert() {
    let alerts = |calls: Vec<CallView>| {
        let mut tui = tui();
        let _ = tui.apply(snapshot(
            100,
            vec![session(1, "claude_code_headers", 100)],
            calls,
        ));
        let wall = tui.wall(at(100));
        pane(&wall, 0)
            .alerts
            .iter()
            .map(|alert| format!("{} | {}", alert.label, alert.detail))
            .collect::<Vec<_>>()
    };
    let limited = |n, start| {
        call(
            n,
            1,
            None,
            start,
            start + 1,
            json!({ "status": 429, "outcome": null }),
        )
    };
    // Failures the session has moved past are not shown again; a head
    // that has not come for a while is.
    assert_eq!(
        alerts(vec![
            limited(11, 10),
            call(
                12,
                1,
                None,
                30,
                31,
                json!({ "terminal": "interrupted", "outcome": null })
            ),
            call(13, 1, None, 40, 41, json!({})),
            open(14, 1, None, 50, false),
        ]),
        ["no response head | waiting 50 s"]
    );
    // Rate limits come in bursts: one alert, with the count.
    assert_eq!(
        alerts(vec![limited(11, 10), limited(12, 20)]),
        ["429 rate limited | 2 in the window"]
    );
    assert_eq!(
        alerts(vec![call(
            11,
            1,
            None,
            30,
            31,
            json!({ "terminal": "interrupted", "outcome": null })
        )]),
        ["interrupted | model-a"]
    );
    assert_eq!(
        alerts(vec![call(
            11,
            1,
            None,
            30,
            31,
            json!({ "status": 529, "outcome": null })
        )]),
        ["error 529 | model-a"]
    );
}

#[test]
fn a_turn_is_read_once_and_again_only_when_it_changes() {
    let mut tui = tui();
    let sessions = vec![session(1, "claude_code_headers", 100)];
    let first = vec![call(
        11,
        1,
        Some(5),
        10,
        20,
        json!({ "tool_names": ["Read"] }),
    )];
    assert_eq!(
        tui.apply(snapshot(100, sessions.clone(), first.clone())),
        [Read::Turn(turn_id(5))]
    );
    let _ = tui.apply(turn(
        5,
        1,
        &[],
        vec![activity_call(11, Some("Looking."), vec![], &[])],
    ));
    assert_eq!(
        tui.apply(snapshot(102, sessions.clone(), first.clone())),
        []
    );

    // A new call in the turn changes it.
    let mut second = first;
    second.push(open(12, 1, Some(5), 30, true));
    assert_eq!(
        tui.apply(snapshot(104, sessions.clone(), second.clone())),
        [Read::Turn(turn_id(5))]
    );
    // Read while the new call's text is still being derived: read again.
    let _ = tui.apply(turn(
        5,
        1,
        &[],
        vec![
            activity_call(11, Some("Looking."), vec![], &[]),
            activity_call(12, None, vec![], &["not_yet_derived"]),
        ],
    ));
    assert_eq!(
        tui.apply(snapshot(106, sessions, second)),
        [Read::Turn(turn_id(5))]
    );
}

#[test]
fn a_turn_reads_as_replies_and_merged_tool_runs() {
    let mut tui = tui();
    let _ = tui.apply(snapshot(
        100,
        vec![session(1, "claude_code_headers", 100)],
        vec![
            call(11, 1, Some(5), 10, 20, json!({})),
            call(12, 1, Some(5), 21, 30, json!({})),
            open(13, 1, Some(5), 31, true),
        ],
    ));
    let _ = tui.apply(turn(
        5,
        1,
        &["cd /w/app && cargo test", "cd /w/app && cargo test", "ls"],
        vec![
            activity_call(
                11,
                Some("  Looking.  "),
                vec![
                    tool(
                        "a",
                        "Read",
                        Some(("read_file", "/w/app/src/lib.rs")),
                        Some("succeeded"),
                    ),
                    tool(
                        "b",
                        "Read",
                        Some(("read_file", "/w/app/src/main.rs")),
                        Some("succeeded"),
                    ),
                    tool(
                        "c",
                        "Bash",
                        Some(("run_command", "cd /w/app && cargo test")),
                        Some("failed"),
                    ),
                    tool(
                        "d",
                        "Bash",
                        Some(("run_command", "cd /w/app && cargo test")),
                        Some("succeeded"),
                    ),
                ],
                &[],
            ),
            activity_call(
                12,
                None,
                vec![tool(
                    "e",
                    "Edit",
                    Some(("write_file", "/w/app/src/lib.rs")),
                    None,
                )],
                &[],
            ),
            activity_call(13, None, vec![], &["not_yet_derived"]),
        ],
    ));
    let wall = tui.wall(at(100));
    let pane = pane(&wall, 0);
    let tools = |line: &Line| match line {
        Line::Tools(tool) => Some((
            tool.name.clone(),
            tool.target.clone(),
            tool.result.clone(),
            tool.count,
        )),
        Line::Call { .. } | Line::Prompt(_) | Line::Result(_) | Line::Reply(_) | Line::Waiting => {
            None
        }
    };
    let results = |line: &Line| match line {
        Line::Result(tool) => Some((tool.name.clone(), tool.result.clone(), tool.count)),
        Line::Call { .. } | Line::Prompt(_) | Line::Tools(_) | Line::Reply(_) | Line::Waiting => {
            None
        }
    };
    assert_eq!(pane.lines[0], Line::Call { index: Some(1) });
    assert_eq!(pane.lines[1], Line::Reply("Looking.".to_owned()));
    assert_eq!(
        tools(&pane.lines[2]),
        Some((
            "Read".to_owned(),
            Some("src/main.rs".to_owned()),
            ResultMark::AllSucceeded(2),
            2
        ))
    );
    // Paths and a `cd` into the turn's working directory are left off.
    assert_eq!(
        tools(&pane.lines[3]),
        Some((
            "Bash".to_owned(),
            Some("cargo test".to_owned()),
            ResultMark::Failures(1),
            2
        ))
    );
    // The next call carries their results back.
    assert_eq!(pane.lines[4], Line::Call { index: Some(2) });
    assert_eq!(
        results(&pane.lines[5]),
        Some(("Read".to_owned(), ResultMark::AllSucceeded(2), 2))
    );
    assert_eq!(
        results(&pane.lines[6]),
        Some(("Bash".to_owned(), ResultMark::Failures(1), 2))
    );
    // The result rides in the next request, which is still being derived.
    assert_eq!(
        tools(&pane.lines[7]),
        Some((
            "Edit".to_owned(),
            Some("src/lib.rs".to_owned()),
            ResultMark::Pending,
            1
        ))
    );
    // That request is open, with its response streaming.
    assert_eq!(pane.lines[8], Line::Call { index: Some(3) });
    assert_eq!(pane.lines.len(), 9);
    assert_eq!(pane.turn_calls, 3);
    assert_eq!(
        pane.tally,
        Some(Tally {
            read: 1,
            wrote: 0,
            ran: 3,
            failed: 1
        })
    );
    let Line::Tools(read) = &pane.lines[2] else {
        panic!();
    };
    assert_eq!(read.output.as_deref(), Some("succeeded output"));
}

#[test]
fn a_tool_call_no_later_request_answered_is_missing() {
    let mut tui = tui();
    let _ = tui.apply(snapshot(
        100,
        vec![session(1, "claude_code_headers", 100)],
        vec![
            call(11, 1, Some(5), 10, 20, json!({})),
            call(12, 1, Some(5), 21, 30, json!({})),
        ],
    ));
    let _ = tui.apply(turn(
        5,
        1,
        &[],
        vec![
            activity_call(11, None, vec![tool("a", "Read", None, None)], &[]),
            activity_call(12, Some("Done."), vec![], &[]),
        ],
    ));
    let wall = tui.wall(at(100));
    let Line::Tools(read) = &pane(&wall, 0).lines[1] else {
        panic!("{:?}", pane(&wall, 0).lines);
    };
    assert_eq!(read.result, ResultMark::Missing);
}

#[test]
fn progress_streams_into_the_pane_until_its_call_ends() {
    let mut tui = tui();
    let sessions = vec![session(1, "opencode_headers", 100)];
    // OpenCode sends no turn: the pane has no lines and asks for no turn.
    assert_eq!(
        tui.apply(snapshot(
            100,
            sessions.clone(),
            vec![open(11, 1, None, 90, true)]
        )),
        []
    );
    let progress = Progress {
        call: call_id(11),
        reply: "Reading the".to_owned(),
        cut: false,
        tools: vec!["read".to_owned()],
        thinking: false,
    };
    assert_eq!(tui.apply(Input::Progress(progress.clone())), []);
    let wall = tui.wall(at(100));
    let pane = pane(&wall, 0);
    // No turn: only the open call's rule, then what it streamed.
    assert_eq!(pane.lines, [Line::Call { index: None }]);
    assert_eq!(pane.harness, Some(Harness::OpenCode.into()));
    let streaming = pane.streaming.as_ref().unwrap();
    assert_eq!(
        (streaming.reply.as_str(), streaming.index),
        ("Reading the", None)
    );

    // The call ends: its progress is let go and a snapshot is asked for.
    let finished: Observation = serde_json::from_value(json!({
        "seq": 9, "at": T0, "kind": "call_finished",
        "channel": uuid(1011), "call": uuid(11), "attempt": null
    }))
    .unwrap();
    assert_eq!(tui.apply(Input::Observation(finished)), [Read::Snapshot]);
    assert!(pane_of(&tui, 100).streaming.is_none());

    // A late update on a call already read as ended is dropped.
    let _ = tui.apply(snapshot(
        101,
        sessions,
        vec![call(11, 1, None, 90, 100, json!({}))],
    ));
    let _ = tui.apply(Input::Progress(progress));
    assert!(pane_of(&tui, 101).streaming.is_none());
}

fn pane_of(tui: &Tui, seconds: u64) -> SessionPane {
    pane(&tui.wall(at(seconds)), 0).clone()
}

#[test]
fn a_resize_places_waiting_sessions_and_asks_for_their_turns() {
    let mut tui = Tui::new(Size {
        width: Grid::PANE_WIDTH,
        height: Grid::PANE_HEIGHT + Grid::STATUS_LINES,
    });
    let sessions = vec![
        session(1, "claude_code_headers", 100),
        session(2, "codex_headers", 90),
    ];
    let calls = vec![
        call(11, 1, Some(5), 10, 20, json!({})),
        call(21, 2, Some(6), 10, 20, json!({})),
    ];
    assert_eq!(
        tui.apply(snapshot(100, sessions, calls)),
        [Read::Turn(turn_id(5))]
    );
    assert_eq!(tui.wall(at(100)).status.hidden, 1);
    let reads = tui.apply(Input::Resize(Size {
        width: 2 * Grid::PANE_WIDTH,
        height: Grid::PANE_HEIGHT + Grid::STATUS_LINES,
    }));
    assert_eq!(reads, [Read::Turn(turn_id(5)), Read::Turn(turn_id(6))]);
    let wall = tui.wall(at(100));
    assert_eq!(
        wall.grid,
        Grid {
            columns: 2,
            rows: 1
        }
    );
    assert_eq!(pane(&wall, 1).session, session_id(2));
    assert_eq!(wall.status.hidden, 0);
}

#[test]
fn output_is_binned_per_client_when_calls_end() {
    let mut tui = tui();
    let usage = |output: u64| {
        json!({ "usage": {
            "accounting": "excludes_cache", "input_tokens": 10, "output_tokens": output,
            "cache_read_tokens": 0, "cache_creation_tokens": 0, "reasoning_tokens": null
        }})
    };
    let codex = |n, end, output: u64| {
        let mut fields = usage(output);
        fields["attribution"] = json!({
            "hint": { "source": "codex_headers", "session": "s", "turn": null, "thread": null },
            "issues": []
        });
        call(n, 2, None, end - 1, end, fields)
    };
    let _ = tui.apply(snapshot(
        300,
        vec![
            session(1, "claude_code_headers", 300),
            session(2, "codex_headers", 300),
        ],
        vec![
            call(11, 1, None, 9, 10, usage(500)),
            call(12, 1, None, 298, 299, usage(100)),
            call(13, 1, None, 290, 295, usage(50)),
            codex(21, 200, 7),
            open(14, 1, None, 299, true),
        ],
    ));
    let wall = tui.wall(at(300));
    let output = &wall.status.output;
    // Claude Code's first call ended before the span; Pi and OpenCode had none.
    assert_eq!(output.len(), 2);
    assert_eq!(output[0].harness, Harness::ClaudeCode);
    assert_eq!(output[0].bins.len(), Tui::OUTPUT_BINS);
    assert_eq!(output[0].bins.last(), Some(&150));
    assert_eq!(output[0].bins.iter().sum::<u64>(), 150);
    assert_eq!(output[1].harness, Harness::Codex);
    assert_eq!(output[1].bins.iter().sum::<u64>(), 7);
    // Context comes from the newest call that reported usage.
    let pane = pane(&wall, 0);
    assert_eq!((pane.context, pane.output), (Some(10), Some(100)));
}

#[test]
fn a_full_page_of_calls_is_said_to_leave_some_out() {
    let mut tui = tui();
    let recent = call(11, 1, None, 10, 20, json!({}));
    let _ = tui.apply(Input::Snapshot(Snapshot {
        read_at: at(100),
        sessions: vec![session(1, "claude_code_headers", 100)],
        open: page(vec![]),
        recent: CallPage {
            next: Some(recent.cursor()),
            calls: vec![recent],
        },
    }));
    assert_eq!(tui.wall(at(100)).status.reading, Reading::Capped);
}

/// Three sessions: Claude Code mid-turn with a call streaming, Codex
/// stopped on a 529, and OpenCode streaming with no turns.
fn busy(size: Size) -> Tui {
    let mut tui = Tui::new(size);
    let usage = json!({ "usage": {
        "accounting": "excludes_cache", "input_tokens": 1_200, "output_tokens": 1_234,
        "cache_read_tokens": 60_000, "cache_creation_tokens": 0, "reasoning_tokens": null
    }});
    let codex = json!({
        "attribution": {
            "hint": { "source": "codex_headers", "session": "s", "turn": null, "thread": null },
            "issues": []
        },
        "provider": "openai", "endpoint": "responses", "model": "model-b",
        "status": 529, "outcome": null
    });
    let mut opencode = open(31, 3, None, 280, true);
    opencode.attribution.hint = None;
    let _ = tui.apply(snapshot(
        300,
        vec![
            session(1, "claude_code_headers", 300),
            session(2, "codex_headers", 290),
            session(3, "opencode_headers", 300),
        ],
        vec![
            call(11, 1, Some(5), 200, 230, usage),
            open(12, 1, Some(5), 290, true),
            call(21, 2, Some(6), 280, 285, codex),
            opencode,
        ],
    ));
    let _ = tui.apply(turn(
        5,
        1,
        &["cd /w/app && cargo test"],
        vec![
            prompted(
                "Why does the parser drop the last field?",
                activity_call(
                    11,
                    Some("I'll run the tests first, then look at the failure in the parser."),
                    vec![
                        tool(
                            "a",
                            "Read",
                            Some(("read_file", "/w/app/src/lib.rs")),
                            Some("succeeded"),
                        ),
                        tool(
                            "b",
                            "Bash",
                            Some(("run_command", "cd /w/app && cargo test")),
                            Some("failed"),
                        ),
                    ],
                    &[],
                ),
            ),
            activity_call(12, None, vec![], &["not_yet_derived"]),
        ],
    ));
    let _ = tui.apply(Input::Progress(Progress {
        call: call_id(12),
        reply: "The parser drops the last field when".to_owned(),
        cut: false,
        tools: vec!["Edit".to_owned()],
        thinking: false,
    }));
    let _ = tui.apply(Input::Progress(Progress {
        call: call_id(31),
        reply: String::new(),
        cut: false,
        tools: Vec::new(),
        thinking: true,
    }));
    tui
}

/// The wall drawn into a buffer, as text: one line per row, trailing
/// spaces trimmed.
fn drawn(wall: &Wall, size: Size) -> String {
    use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
    let area = Rect::new(0, 0, size.width, size.height);
    let mut buffer = Buffer::empty(area);
    Widget::render(wall, area, &mut buffer);
    (0..size.height)
        .map(|y| {
            let row: String = (0..size.width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect();
            row.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[test]
fn the_wall_draws_the_same_at_each_size() {
    let sizes = [
        (
            "wall-80x24.txt",
            Size {
                width: 80,
                height: 24,
            },
            include_str!("../tests/fixtures/wall-80x24.txt"),
        ),
        (
            "wall-120x32.txt",
            Size {
                width: 120,
                height: 32,
            },
            include_str!("../tests/fixtures/wall-120x32.txt"),
        ),
        (
            "wall-180x42.txt",
            Size {
                width: 180,
                height: 42,
            },
            include_str!("../tests/fixtures/wall-180x42.txt"),
        ),
    ];
    for (name, size, pinned) in sizes {
        let tui = busy(size);
        assert_eq!(drawn(&tui.wall(at(300)), size), pinned, "{name}");
    }
}

#[test]
fn zoom_and_help_draw_over_the_wall() {
    let size = Size {
        width: 80,
        height: 24,
    };
    let mut tui = busy(size);
    assert_eq!(tui.press(Key::Enter), Pressed::Continue(vec![]));
    assert_eq!(
        drawn(&tui.wall(at(300)), size),
        include_str!("../tests/fixtures/wall-zoomed.txt")
    );
    assert_eq!(tui.press(Key::Char('?')), Pressed::Continue(vec![]));
    assert_eq!(
        drawn(&tui.wall(at(300)), size),
        include_str!("../tests/fixtures/wall-help.txt")
    );
}

#[test]
fn the_focused_pane_moves_with_the_keys_and_stays_in_the_grid() {
    // Two across, two down.
    let mut tui = busy(Size {
        width: 2 * Grid::PANE_WIDTH,
        height: 2 * Grid::PANE_HEIGHT + Grid::STATUS_LINES,
    });
    let focus = |tui: &Tui| match tui.wall(at(300)).view {
        View::Panes { focus } => focus,
        View::Zoomed => panic!("zoomed"),
    };
    assert_eq!(focus(&tui), 0);
    for (key, expected) in [
        (Key::Left, 0),
        (Key::Up, 0),
        (Key::Right, 1),
        (Key::Right, 1),
        (Key::Down, 3),
        (Key::Down, 3),
        (Key::Left, 2),
        (Key::Tab, 3),
        (Key::Tab, 0),
        (Key::BackTab, 3),
        (Key::Char('x'), 3),
    ] {
        let _ = tui.press(key);
        assert_eq!(focus(&tui), expected, "{key:?}");
    }
    // A smaller terminal keeps the focus inside the grid.
    let _ = tui.apply(Input::Resize(Size {
        width: Grid::PANE_WIDTH,
        height: Grid::PANE_HEIGHT + Grid::STATUS_LINES,
    }));
    assert_eq!(focus(&tui), 0);
    assert_eq!(tui.press(Key::Char('q')), Pressed::Quit);
    assert_eq!(tui.press(Key::Interrupt), Pressed::Quit);
}

#[test]
fn zoom_follows_sessions_and_leaves_with_them() {
    let mut tui = busy(Size {
        width: 3 * Grid::PANE_WIDTH,
        height: Grid::PANE_HEIGHT + Grid::STATUS_LINES,
    });
    let zoomed = |tui: &Tui| {
        let wall = tui.wall(at(300));
        assert_eq!(wall.view, View::Zoomed);
        assert_eq!(
            wall.grid,
            Grid {
                columns: 1,
                rows: 1
            }
        );
        pane(&wall, 0).session
    };
    let first = pane(&tui.wall(at(300)), 0).session;
    let second = pane(&tui.wall(at(300)), 1).session;
    let _ = tui.press(Key::Enter);
    assert_eq!(zoomed(&tui), first);
    let _ = tui.press(Key::Tab);
    assert_eq!(zoomed(&tui), second);
    let _ = tui.press(Key::BackTab);
    let _ = tui.press(Key::BackTab);
    assert_ne!(zoomed(&tui), second);
    let _ = tui.press(Key::Esc);
    assert!(matches!(tui.wall(at(300)).view, View::Panes { .. }));

    // A zoomed session that leaves the wall leaves the zoom.
    let _ = tui.press(Key::Enter);
    let _ = tui.apply(snapshot(
        1000,
        vec![session(9, "pi_extension_headers", 1000)],
        vec![],
    ));
    assert!(matches!(tui.wall(at(1000)).view, View::Panes { .. }));
}

#[test]
fn a_pinned_session_stays_when_quiet_and_the_help_takes_no_other_key() {
    let mut tui = Tui::new(Size {
        width: Grid::PANE_WIDTH,
        height: Grid::PANE_HEIGHT + Grid::STATUS_LINES,
    });
    let sessions = || {
        vec![
            session(1, "claude_code_headers", 300),
            session(2, "codex_headers", 200),
        ]
    };
    let _ = tui.apply(snapshot(300, sessions(), vec![]));
    assert_eq!(pane(&tui.wall(at(300)), 0).session, session_id(1));
    let _ = tui.press(Key::Char('p'));
    assert!(pane(&tui.wall(at(300)), 0).pinned);
    // Long after both went quiet, the pinned one is still on the wall.
    let _ = tui.apply(snapshot(2000, sessions(), vec![]));
    let wall = tui.wall(at(2000));
    assert_eq!(pane(&wall, 0).session, session_id(1));
    assert_eq!(wall.status.sessions, 1);

    let _ = tui.press(Key::Char('?'));
    assert_eq!(tui.wall(at(2000)).help, Help::Shown);
    let _ = tui.press(Key::Char('p'));
    assert!(
        pane(&tui.wall(at(2000)), 0).pinned,
        "the help took the pin key"
    );
    let _ = tui.press(Key::Esc);
    assert_eq!(tui.wall(at(2000)).help, Help::Hidden);
    // Unpinned, it leaves as any quiet session does.
    assert_eq!(tui.press(Key::Char('p')), Pressed::Continue(vec![]));
    assert_eq!(tui.wall(at(2000)).panes, [Pane::Free]);
}

#[test]
fn tiny_terminals_draw_without_panicking() {
    use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
    for (width, height) in [(0, 0), (1, 1), (2, 1), (10, 3), (20, 2), (57, 11), (40, 30)] {
        let size = Size { width, height };
        let mut tui = busy(size);
        for keys in [&[][..], &[Key::Enter][..], &[Key::Char('?')][..]] {
            for key in keys {
                let _ = tui.press(*key);
            }
            let area = Rect::new(0, 0, width, height);
            let mut buffer = Buffer::empty(area);
            Widget::render(&tui.wall(at(300)), area, &mut buffer);
        }
    }
}

#[test]
fn dark_title_bars_keep_the_original_palette() {
    use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};
    let size = Size {
        width: 180,
        height: 42,
    };
    let area = Rect::new(0, 0, size.width, size.height);
    let mut buffer = Buffer::empty(area);
    busy(size)
        .wall(at(300))
        .themed(&crate::ThemeName::Dark.theme())
        .render(area, &mut buffer);
    // Three sessions, three panes across. Responding: green. Stopped on
    // an error: red. The body: the pane's ground.
    assert_eq!(
        busy(size).wall(at(300)).grid,
        Grid {
            columns: 3,
            rows: 1
        }
    );
    assert_eq!(buffer[(5, 0)].bg, Color::Rgb(0x7e, 0xe7, 0x87));
    assert_eq!(buffer[(125, 0)].bg, Color::Rgb(0xff, 0x7b, 0x72));
    assert_eq!(buffer[(5, 10)].bg, Color::Rgb(0x0e, 0x12, 0x17));
    // A lit bar runs the pane's whole width, and its edge below it too.
    assert_eq!(buffer[(179, 0)].bg, Color::Rgb(0xff, 0x7b, 0x72));
    assert_eq!(buffer[(179, 1)].symbol(), "▀");
    assert_eq!(buffer[(179, 1)].fg, Color::Rgb(0xff, 0x7b, 0x72));
}

#[test]
fn switching_themes_resets_every_cell_in_panes_zoom_and_help() {
    use crate::{Theme, ThemeName};
    use ratatui::{
        Terminal,
        backend::TestBackend,
        buffer::Buffer,
        layout::Rect,
        style::{Color, Modifier, Style},
        widgets::Widget,
    };

    for size in [
        Size {
            width: 80,
            height: 24,
        },
        Size {
            width: 180,
            height: 42,
        },
    ] {
        let mut tui = busy(size);
        let area = Rect::new(0, 0, size.width, size.height);
        let mut terminal = Terminal::new(TestBackend::new(size.width, size.height)).unwrap();
        for keys in [&[][..], &[Key::Enter][..], &[Key::Char('?')][..]] {
            for key in keys {
                let _ = tui.press(*key);
            }
            let wall = tui.wall(at(300));
            let mut reused = Buffer::empty(area);
            // Model styles left by a previous custom theme, including modifiers.
            reused.set_style(
                area,
                Style::new()
                    .fg(Color::Rgb(1, 2, 3))
                    .bg(Color::Rgb(4, 5, 6))
                    .add_modifier(Modifier::ITALIC | Modifier::REVERSED),
            );
            let mut name = ThemeName::Terminal;
            let mut symbols = None;
            for _ in 0..7 {
                let theme = name.theme();
                let mut fresh = Buffer::empty(area);
                wall.themed(&theme).render(area, &mut fresh);
                wall.themed(&theme).render(area, &mut reused);
                assert_eq!(reused, fresh, "stale styles in {name}");
                terminal
                    .draw(|frame| frame.render_widget(wall.themed(&theme), frame.area()))
                    .unwrap();
                terminal.backend().assert_buffer(&fresh);
                let shown: Vec<_> = fresh
                    .content
                    .iter()
                    .map(|cell| cell.symbol().to_owned())
                    .collect();
                match &symbols {
                    Some(previous) => assert_eq!(&shown, previous, "theme changed the content"),
                    None => symbols = Some(shown),
                }
                if name == ThemeName::Terminal {
                    for cell in &fresh.content {
                        assert_eq!(cell.bg, Color::Reset);
                        assert!(!matches!(cell.fg, Color::Rgb(..) | Color::Indexed(_)));
                    }
                }
                name = name.next();
            }
            let mono = Theme::monochrome();
            wall.themed(&mono).render(area, &mut reused);
            assert!(
                reused
                    .content
                    .iter()
                    .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
            );
            assert!(
                reused
                    .content
                    .iter()
                    .any(|cell| cell.modifier.contains(Modifier::BOLD))
            );
        }
    }
}

#[test]
fn theme_key_leaves_the_conversation_and_help_unchanged() {
    let mut tui = busy(Size {
        width: 80,
        height: 24,
    });
    for keys in [&[][..], &[Key::Enter][..], &[Key::Char('?')][..]] {
        for key in keys {
            let _ = tui.press(*key);
        }
        let before = tui.wall(at(300));
        assert_eq!(tui.press(Key::Char('t')), Pressed::CycleTheme);
        assert_eq!(tui.wall(at(300)), before);
    }
}

#[test]
fn a_failed_read_is_shown_until_a_snapshot_succeeds() {
    let size = Size {
        width: 120,
        height: 24,
    };
    let mut tui = busy(size);
    assert_eq!(
        tui.apply(Input::ReadFailed("cannot reach the gateway".to_owned())),
        []
    );
    let wall = tui.wall(at(300));
    assert_eq!(
        wall.status.failure.as_deref(),
        Some("cannot reach the gateway")
    );
    // What was read before stays on the wall.
    assert!(matches!(wall.panes[0], Pane::Session(_)));
    let status = drawn(&wall, size).lines().last().unwrap().to_owned();
    assert!(
        status.starts_with(" ren  ! stale: cannot reach the gateway "),
        "{status}"
    );
    let _ = tui.apply(snapshot(
        302,
        vec![session(1, "claude_code_headers", 302)],
        vec![],
    ));
    assert_eq!(tui.wall(at(302)).status.failure, None);
}

#[test]
fn a_pane_shows_the_turn_before_the_newest_one() {
    let mut tui = tui();
    let sessions = vec![session(1, "claude_code_headers", 100)];
    let calls = vec![
        call(11, 1, Some(4), 10, 20, json!({})),
        call(12, 1, Some(5), 60, 70, json!({})),
    ];
    assert_eq!(
        tui.apply(snapshot(100, sessions, calls)),
        [Read::Turn(turn_id(4)), Read::Turn(turn_id(5))]
    );
    let _ = tui.apply(turn(
        4,
        1,
        &[],
        vec![prompted(
            "First ask.",
            activity_call(11, Some("Done."), vec![], &[]),
        )],
    ));
    let _ = tui.apply(turn(
        5,
        1,
        &[],
        vec![prompted(
            "Next ask.",
            activity_call(12, Some("Also done."), vec![], &[]),
        )],
    ));
    let wall = tui.wall(at(100));
    assert_eq!(
        pane(&wall, 0).lines,
        [
            Line::Call { index: Some(1) },
            Line::Prompt("First ask.".to_owned()),
            Line::Reply("Done.".to_owned()),
            Line::Call { index: Some(1) },
            Line::Prompt("Next ask.".to_owned()),
            Line::Reply("Also done.".to_owned()),
        ]
    );
}

#[test]
fn a_session_without_turns_reads_its_newest_calls() {
    let mut tui = tui();
    let sessions = vec![session(1, "opencode_headers", 100)];
    let calls: Vec<CallView> = (0..6)
        .map(|n| {
            call(
                11 + n,
                1,
                None,
                10 + 10 * u64::try_from(n).unwrap(),
                15 + 10 * u64::try_from(n).unwrap(),
                json!({}),
            )
        })
        .chain([open(20, 1, None, 90, true)])
        .collect();
    // The newest four finished calls, newest first; the open one streams.
    assert_eq!(
        tui.apply(snapshot(100, sessions.clone(), calls.clone())),
        [
            Read::Call(call_id(16)),
            Read::Call(call_id(15)),
            Read::Call(call_id(14)),
            Read::Call(call_id(13))
        ]
    );
    let activity = |call: Value| Input::Call(Box::new(serde_json::from_value(call).unwrap()));
    let _ = tui.apply(activity(prompted(
        "Fix the login form.",
        activity_call(
            15,
            Some("Looking."),
            vec![tool(
                "a",
                "read",
                Some(("read_file", "src/form.ts")),
                Some("succeeded"),
            )],
            &[],
        ),
    )));
    let _ = tui.apply(activity(activity_call(16, Some("Fixed."), vec![], &[])));
    let wall = tui.wall(at(100));
    let pane = pane(&wall, 0);
    assert_eq!(
        pane.lines[..6],
        [
            Line::Call { index: Some(5) },
            Line::Prompt("Fix the login form.".to_owned()),
            Line::Reply("Looking.".to_owned()),
            pane.lines[3].clone(),
            Line::Call { index: Some(6) },
            pane.lines[5].clone(),
        ]
    );
    assert!(matches!(pane.lines[3], Line::Tools(_)));
    assert!(matches!(pane.lines[5], Line::Result(_)));
    // Read, those two are not asked for again.
    assert_eq!(
        tui.apply(snapshot(101, sessions, calls)),
        [Read::Call(call_id(14)), Read::Call(call_id(13))]
    );
}

#[test]
fn a_plain_title_has_a_rule_under_it() {
    use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};
    let mut tui = tui();
    let _ = tui.apply(snapshot(
        100,
        vec![session(1, "claude_code_headers", 100)],
        vec![call(11, 1, None, 10, 20, json!({}))],
    ));
    let area = Rect::new(0, 0, 60, 12);
    let mut buffer = Buffer::empty(area);
    let theme = ThemeName::Dark.theme();
    tui.wall(at(100)).themed(&theme).render(area, &mut buffer);
    assert_eq!(buffer[(0, 0)].bg, Color::Rgb(0x0e, 0x12, 0x17));
    assert_eq!(buffer[(0, 1)].symbol(), "─");
    assert_eq!(buffer[(59, 1)].fg, Color::Rgb(0x2a, 0x30, 0x38));
}

const THREAD: &str = "0190f3a0-0000-7000-8000-0000000000aa";
const RUN: &str = "0190f3a0-0000-7000-8000-0000000000cc";

/// A Codex session whose native id is the run's thread.
fn run_session(n: u128, seconds: u64) -> SessionView {
    let mut view = session(n, "codex_headers", seconds);
    view.native_id = THREAD.to_owned();
    view
}

/// The daemon's runs, with one live run in `state`.
fn runs(state: Value) -> Input {
    Input::Runs(Runs::Read(vec![
        serde_json::from_value(json!({
            "run": RUN, "harness": "codex", "workdir": "/work/acme", "state": state
        }))
        .unwrap(),
    ]))
}

fn asking() -> Value {
    json!({
        "phase": "working", "thread": THREAD, "turn": null,
        "approvals": [
            { "approval": "ap-1", "ask": { "kind": "command", "command": "cargo test",
                                           "cwd": "/work/acme", "reason": null } },
            { "approval": "ap-2", "ask": { "kind": "file_change", "reason": null } }
        ]
    })
}

fn run_id() -> RunId {
    RUN.parse().unwrap()
}

/// A wall with the run's session in the only pane, and quiet otherwise.
fn with_run(state: Value) -> Tui {
    let mut tui = tui();
    // Seen long ago: only the run keeps the session on the wall.
    let _ = tui.apply(snapshot(600, vec![run_session(7, 10)], vec![]));
    assert!(
        tui.wall(at(600))
            .panes
            .iter()
            .all(|pane| *pane == Pane::Free)
    );
    let _ = tui.apply(runs(state));
    tui
}

#[test]
fn a_live_runs_session_keeps_a_pane_with_the_runs_mark() {
    let tui = with_run(asking());
    let wall = tui.wall(at(600));
    let pane = pane(&wall, 0);
    let mark = pane.run.as_ref().expect("the run's mark");
    assert_eq!(mark.run, run_id());
    assert!(matches!(&mark.phase, RunPhase::Waiting { more: 1, .. }));
    assert_eq!(
        wall.status.runs,
        Some(RunCounts {
            live: 1,
            waiting: 1
        })
    );
}

#[test]
fn run_observations_ask_for_the_runs() {
    let mut tui = tui();
    let observation: Observation = serde_json::from_value(json!({
        "seq": 1, "at": T0, "kind": "run_started",
        "channel": "00000000-0000-0000-0000-000000000000", "call": null, "attempt": null
    }))
    .unwrap();
    assert_eq!(
        tui.apply(Input::Observation(observation)),
        [Read::Runs, Read::Snapshot]
    );
}

#[test]
fn a_waiting_run_draws_as_pinned() {
    let size = Size {
        width: 80,
        height: 12,
    };
    let mut tui = Tui::new(size);
    let _ = tui.apply(snapshot(600, vec![run_session(7, 10)], vec![]));
    let _ = tui.apply(runs(asking()));
    assert_eq!(
        drawn(&tui.wall(at(600)), size),
        include_str!("../tests/fixtures/wall-run.txt")
    );
}
