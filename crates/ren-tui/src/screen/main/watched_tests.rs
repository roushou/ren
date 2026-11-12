//! How a watched session is read, against reads shaped as the management
//! API sends them, run as the terminal runs it: a [`Tui`], its list
//! selecting the first session.

use super::list::{Grouping, Item, Listing};
use super::watched::{Phase, SessionPane, Tally};
use crate::conversation::{Block, ResultMark};
use crate::model::sessions::{Sessions, Spark};
use crate::model::{CallEnd, Input, Read, Snapshot};
use crate::ui::Tone;
use crate::{Out, Size, Tui};
use ren_harness::Harness;
use ren_recorder::Observation;
use ren_recorder::capture::CallState;
use ren_recorder::id::{CallId, SessionId, TurnId};
use ren_recorder::progress::Progress;
use ren_recorder::time::UnixMillis;
use ren_store::view::{CallPage, CallView, SessionView};
use serde_json::{Value, json};

/// The app, as these tests read it.
struct Wall {
    tui: Tui,
}

/// What the tests read of the watched sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    panes: Vec<SessionPane>,
    status: Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Status {
    sessions: usize,
    responding: usize,
    awaiting: usize,
    output: Vec<Spark>,
    failure: Option<String>,
}

impl Wall {
    const OUTPUT_BINS: usize = Sessions::OUTPUT_BINS;

    fn new(size: Size) -> Self {
        let mut tui = Tui::new(size);
        let _ = tui.start();
        Self { tui }
    }

    /// What `input` asks to read.
    fn apply(&mut self, input: Input) -> Vec<Read> {
        self.tui
            .apply(input)
            .into_iter()
            .filter_map(|out| match out {
                Out::Read(read) => Some(read),
                Out::Send(_) | Out::CycleTheme | Out::Quit => None,
            })
            .collect()
    }

    /// The watched sessions, in the list's order, at `now`.
    fn frame(&self, now: UnixMillis) -> Frame {
        let model = &self.tui.model;
        let panes: Vec<SessionPane> = Listing::of(model, "", Grouping::Folder)
            .items()
            .into_iter()
            .filter_map(|item| match item {
                Item::Watched(session) => model
                    .sessions
                    .sessions()
                    .iter()
                    .find(|view| view.id == session)
                    .map(|view| SessionPane::of(model, view, now)),
                Item::Run(_) => None,
            })
            .collect();
        let traffic = model.sessions.traffic(now);
        Frame {
            panes,
            status: Status {
                sessions: traffic.sessions,
                responding: traffic.responding,
                awaiting: traffic.awaiting,
                output: traffic.output,
                failure: traffic.failure,
            },
        }
    }
}

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
fn tui() -> Wall {
    Wall::new(Size {
        width: 168,
        height: 40,
    })
}

fn pane(wall: &Frame, index: usize) -> &SessionPane {
    &wall.panes[index]
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
fn sessions_with_open_calls_come_first_then_the_most_recently_seen() {
    let mut tui = tui();
    let sessions = vec![
        session(1, "claude_code_headers", 100),
        session(2, "codex_headers", 200),
        session(3, "pi_extension_headers", 50),
        session(4, "opencode_headers", 300),
    ];
    let calls = vec![open(31, 3, None, 40, true)];
    let _ = tui.apply(snapshot(300, sessions.clone(), calls));
    let wall = tui.frame(at(300));
    let shown: Vec<SessionId> = (0..wall.panes.len())
        .map(|index| pane(&wall, index).session)
        .collect();
    assert_eq!(
        shown,
        [session_id(3), session_id(4), session_id(2), session_id(1)]
    );
    assert_eq!(wall.status.sessions, 4);

    // Session 3's call ends, long ago: it leaves the list.
    assert_eq!(
        Tui::snapshot_query(at(400)).recent.filter.time.from(),
        Some(at(100))
    );
    let _ = tui.apply(snapshot(
        400,
        sessions,
        vec![call(31, 3, None, 40, 41, json!({}))],
    ));
    let wall = tui.frame(at(400));
    let shown: Vec<SessionId> = (0..wall.panes.len())
        .map(|index| pane(&wall, index).session)
        .collect();
    assert_eq!(shown, [session_id(4), session_id(2), session_id(1)]);
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
    let wall = tui.frame(at(100));
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
        pane(&tui.frame(at(now)), 0).phase.clone()
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
        let wall = tui.frame(at(100));
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
    let wall = tui.frame(at(100));
    let pane = pane(&wall, 0);
    let tools = |line: &Block| match line {
        Block::Tools(tool) => Some((
            tool.name.clone(),
            tool.target.clone(),
            tool.result.clone(),
            tool.count,
        )),
        Block::Call { .. }
        | Block::Prompt(_)
        | Block::Result(_)
        | Block::Reply(_)
        | Block::Streaming(_)
        | Block::Thinking
        | Block::Step(_)
        | Block::Notice { .. }
        | Block::Waiting => None,
    };
    let results = |line: &Block| match line {
        Block::Result(tool) => Some((tool.name.clone(), tool.result.clone(), tool.count)),
        Block::Call { .. }
        | Block::Prompt(_)
        | Block::Tools(_)
        | Block::Reply(_)
        | Block::Streaming(_)
        | Block::Thinking
        | Block::Step(_)
        | Block::Notice { .. }
        | Block::Waiting => None,
    };
    assert_eq!(pane.lines[0], Block::Call { index: Some(1) });
    assert_eq!(pane.lines[1], Block::Reply("Looking.".to_owned()));
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
    assert_eq!(pane.lines[4], Block::Call { index: Some(2) });
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
    assert_eq!(pane.lines[8], Block::Call { index: Some(3) });
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
    let Block::Tools(read) = &pane.lines[2] else {
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
    let wall = tui.frame(at(100));
    let Block::Tools(read) = &pane(&wall, 0).lines[1] else {
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
    let wall = tui.frame(at(100));
    let pane = pane(&wall, 0);
    // No turn: only the open call's rule, then what it streamed.
    assert_eq!(pane.lines, [Block::Call { index: None }]);
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

fn pane_of(tui: &Wall, seconds: u64) -> SessionPane {
    pane(&tui.frame(at(seconds)), 0).clone()
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
    let wall = tui.frame(at(300));
    let output = &wall.status.output;
    // Claude Code's first call ended before the span; Pi and OpenCode had none.
    assert_eq!(output.len(), 2);
    assert_eq!(output[0].harness, Harness::ClaudeCode);
    assert_eq!(output[0].bins.len(), Wall::OUTPUT_BINS);
    assert_eq!(output[0].bins.last(), Some(&150));
    assert_eq!(output[0].bins.iter().sum::<u64>(), 150);
    assert_eq!(output[1].harness, Harness::Codex);
    assert_eq!(output[1].bins.iter().sum::<u64>(), 7);
    // Context comes from the newest call that reported usage.
    let pane = pane(&wall, 0);
    assert_eq!((pane.context, pane.output), (Some(10), Some(100)));
}

#[test]
fn a_failed_read_is_shown_until_a_snapshot_succeeds() {
    let mut tui = tui();
    let _ = tui.apply(snapshot(
        300,
        vec![session(1, "claude_code_headers", 300)],
        vec![],
    ));
    assert_eq!(
        tui.apply(Input::ReadFailed("cannot reach the gateway".to_owned())),
        []
    );
    let wall = tui.frame(at(300));
    assert_eq!(
        wall.status.failure.as_deref(),
        Some("cannot reach the gateway")
    );
    // What was read before stays on the wall.
    assert_eq!(wall.panes.len(), 1);
    let _ = tui.apply(snapshot(
        302,
        vec![session(1, "claude_code_headers", 302)],
        vec![],
    ));
    assert_eq!(tui.frame(at(302)).status.failure, None);
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
    let wall = tui.frame(at(100));
    assert_eq!(
        pane(&wall, 0).lines,
        [
            Block::Call { index: Some(1) },
            Block::Prompt("First ask.".to_owned()),
            Block::Reply("Done.".to_owned()),
            Block::Call { index: Some(1) },
            Block::Prompt("Next ask.".to_owned()),
            Block::Reply("Also done.".to_owned()),
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
    let wall = tui.frame(at(100));
    let pane = pane(&wall, 0);
    assert_eq!(
        pane.lines[..6],
        [
            Block::Call { index: Some(5) },
            Block::Prompt("Fix the login form.".to_owned()),
            Block::Reply("Looking.".to_owned()),
            pane.lines[3].clone(),
            Block::Call { index: Some(6) },
            pane.lines[5].clone(),
        ]
    );
    assert!(matches!(pane.lines[3], Block::Tools(_)));
    assert!(matches!(pane.lines[5], Block::Result(_)));
    // Read, those two are not asked for again.
    assert_eq!(
        tui.apply(snapshot(101, sessions, calls)),
        [Read::Call(call_id(14)), Read::Call(call_id(13))]
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
