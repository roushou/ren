//! The console with synthetic runs, events, and recorded turns: no daemon,
//! no terminal. Screens are pinned as text in `tests/fixtures`.

use crate::console::{Answered, Command, Console, Input, Out, Read};
use crate::{Drawn, Key, Size, Theme};
use ratatui::{buffer::Buffer, layout::Rect};
use ren_harness::{
    Harness,
    drive::{ApprovalAsk, Decision, DriveEvent, Prompt},
};
use ren_protocol::native::{NativeThreadId, NativeTurnId};
use ren_recorder::id::{RunId, SessionId};
use ren_store::{activity::TurnActivity, view::RunView};
use ren_supervisor::{Approvals, Project, ProjectGit, RunSnapshot, RunState, RunUpdate};
use serde_json::json;

const SIZE: Size = Size {
    width: 90,
    height: 24,
};

fn run_id(n: u128) -> RunId {
    RunId::restore(uuid::Uuid::from_u128(n))
}

fn thread(n: u128) -> NativeThreadId {
    NativeThreadId::new(uuid::Uuid::from_u128(0x7700 + n)).unwrap()
}

fn turn(n: u128) -> NativeTurnId {
    NativeTurnId::new(uuid::Uuid::from_u128(0x8800 + n)).unwrap()
}

fn snapshot(n: u128, harness: Harness, workdir: &str, state: RunState) -> RunSnapshot {
    RunSnapshot {
        run: run_id(n),
        harness,
        workdir: workdir.into(),
        state,
    }
}

fn idle(n: u128) -> RunState {
    RunState::Idle { thread: thread(n) }
}

fn working(n: u128) -> RunState {
    RunState::Working {
        thread: thread(n),
        turn: Some(turn(n)),
        approvals: Approvals::default(),
    }
}

fn asking(n: u128, ask: ApprovalAsk) -> RunState {
    let mut approvals = Approvals::default();
    approvals.insert(format!("ap-{n}").parse().unwrap(), ask);
    RunState::Working {
        thread: thread(n),
        turn: Some(turn(n)),
        approvals,
    }
}

fn push() -> ApprovalAsk {
    ApprovalAsk::Command {
        command: Some("git push origin fix/upload-lease".into()),
        cwd: None,
        reason: Some("Push the fix so CI runs it".into()),
    }
}

fn projects() -> Vec<Project> {
    let project = |name: &str, path: &str, git: ProjectGit| Project {
        path: path.into(),
        name: name.into(),
        git,
    };
    vec![
        project(
            "acme-api",
            "/dev/acme-api",
            ProjectGit::Branch {
                name: "main".into(),
            },
        ),
        project(
            "acme-web",
            "/dev/acme-web",
            ProjectGit::Branch {
                name: "feat/upload".into(),
            },
        ),
        project(
            "ren",
            "/srv/ren",
            ProjectGit::Branch {
                name: "agent-runs".into(),
            },
        ),
    ]
}

fn event(run: u128, value: serde_json::Value) -> Input {
    Input::Update(RunUpdate::Event {
        run: run_id(run),
        event: serde_json::from_value::<DriveEvent>(value).unwrap(),
    })
}

fn state(run: u128, state: RunState) -> Input {
    Input::Update(RunUpdate::State {
        run: run_id(run),
        state,
    })
}

fn recorded_turn(native: &str, prompt: &str, reply: &str) -> TurnActivity {
    serde_json::from_value(json!({
        "turn": "00000000-0000-0000-0000-0000000000f1",
        "session": "00000000-0000-0000-0000-0000000000f2",
        "native_id": native,
        "client": "claude_code",
        "started_at": 1000, "ended_at": 2000,
        "prompt": { "text": prompt, "omitted_chars": 0 },
        "files_read": [], "files_written": [], "commands": [],
        "tool_calls": 1, "failed_tool_calls": 0, "unanswered_tool_calls": 0,
        "findings": [],
        "calls": [{
            "call": "00000000-0000-0000-0000-0000000000f3",
            "started_at": 1000, "ended_at": 2000, "model": "claude", "status": 200,
            "outcome": null,
            "metrics": {
                "context_tokens": null, "cache_read_tokens": null, "cache_written_tokens": null,
                "output_tokens": null, "request_bytes": null, "duration_ms": null
            },
            "prompt": null,
            "tool_calls": [{
                "id": "t1", "name": "Bash",
                "effects": [{ "kind": "command", "target": "cargo test -p upload" }],
                "input": { "text": "{}", "omitted_chars": 0 },
                "result": { "status": "succeeded", "output": { "text": "ok", "omitted_chars": 0 } }
            }],
            "earlier_results": 0,
            "reply": { "text": reply, "omitted_chars": 0 },
            "limitations": []
        }],
        "omitted_calls": 0
    }))
    .unwrap()
}

const SESSION: &str = "00000000-0000-0000-0000-00000000aaaa";

fn session() -> SessionId {
    serde_json::from_value(json!(SESSION)).unwrap()
}

/// A run as recorded: started, opened with its thread, maybe with a session.
fn recorded(n: u128, workdir: &str, session: Option<&str>) -> RunView {
    serde_json::from_value(json!({
        "id": run_id(n), "harness": "claude_code", "workdir": workdir,
        "started_at": 1000 + n, "thread": thread(n).to_string(), "session": session,
        "ended_at": null, "end": null, "exit": null
    }))
    .unwrap()
}

fn type_text(console: &mut Console, text: &str) {
    for ch in text.chars() {
        let _ = console.press(Key::Char(ch));
    }
}

fn sent(out: &[Out]) -> Vec<Command> {
    out.iter()
        .filter_map(|out| match out {
            Out::Send(command) => Some(command.clone()),
            Out::Read(_) | Out::Wall | Out::Quit => None,
        })
        .collect()
}

/// A console with three live runs: one asking, one working, one idle.
fn desk() -> Console {
    let mut console = Console::new(SIZE);
    let _ = console.apply(Input::Projects(Ok(projects())));
    let _ = console.apply(Input::Runs(Ok(vec![
        snapshot(1, Harness::Codex, "/dev/acme-api", working(1)),
        snapshot(2, Harness::ClaudeCode, "/srv/ren", asking(2, push())),
        snapshot(3, Harness::ClaudeCode, "/dev/acme-web", idle(3)),
    ])));
    console
}

/// The frame as text, one line per row, trailing spaces trimmed.
fn drawn(console: &Console) -> String {
    let frame = console.frame();
    let area = Rect::new(0, 0, SIZE.width, SIZE.height);
    let mut buf = Buffer::empty(area);
    frame.render(&Theme::default(), area, &mut buf);
    let mut text = String::new();
    for y in 0..area.height {
        let mut line = String::new();
        for x in 0..area.width {
            line.push_str(buf[(x, y)].symbol());
        }
        text.push_str(line.trim_end());
        text.push('\n');
    }
    text
}

#[test]
fn the_run_that_needs_you_opens_first() {
    let mut console = Console::new(SIZE);
    assert_eq!(
        console.start(),
        [
            Out::Read(Read::Runs),
            Out::Read(Read::Recorded),
            Out::Read(Read::Projects)
        ]
    );
    let out = console.apply(Input::Runs(Ok(vec![
        snapshot(1, Harness::Codex, "/dev/acme-api", working(1)),
        snapshot(2, Harness::ClaudeCode, "/srv/ren", asking(2, push())),
    ])));
    // Its session is not known yet: the recorded runs are read first.
    assert_eq!(out, [Out::Read(Read::Recorded)]);
    assert_eq!(console.current(), Some(run_id(2)));
    let out = console.apply(Input::Recorded(vec![recorded(
        2,
        "/srv/ren",
        Some(SESSION),
    )]));
    assert_eq!(
        out,
        [Out::Read(Read::History {
            run: run_id(2),
            session: session(),
        })]
    );
}

#[test]
fn a_digit_answers_and_moves_on_to_the_next_run_that_needs_you() {
    let mut console = desk();
    assert_eq!(console.current(), Some(run_id(2)));
    // Another run starts asking: the open one stays open.
    let _ = console.apply(state(
        1,
        asking(
            1,
            ApprovalAsk::FileChange {
                reason: Some("src/orders/query.rs".into()),
            },
        ),
    ));
    assert_eq!(console.current(), Some(run_id(2)));
    let out = console.press(Key::Char('2'));
    assert_eq!(
        sent(&out),
        [Command::Answer {
            run: run_id(2),
            approval: "ap-2".parse().unwrap(),
            decision: Decision::AcceptForSession,
        }]
    );
    assert_eq!(
        console.current(),
        Some(run_id(1)),
        "moved on to the other asking run"
    );
    let shown = drawn(&console);
    assert!(shown.contains("↳ from ren"), "{shown}");
    assert!(
        shown.contains("allowed git push origin fix/upload-lease"),
        "{shown}"
    );
    assert!(shown.contains("Codex wants to change files"), "{shown}");
}

#[test]
fn typing_during_a_turn_waits_for_its_end() {
    let mut console = desk();
    let _ = console.press(Key::AltRight);
    let _ = console.press(Key::AltRight);
    assert_eq!(console.current(), Some(run_id(1)), "the working run");
    type_text(&mut console, "also check the dead-letter path");
    assert!(sent(&console.press(Key::Enter)).is_empty());
    assert!(drawn(&console).contains("queued: also check the dead-letter path"));
    let out = console.apply(state(1, idle(1)));
    assert_eq!(
        sent(&out),
        [Command::Prompt {
            run: run_id(1),
            prompt: Prompt::parse("also check the dead-letter path".into()).unwrap(),
        }]
    );
}

#[test]
fn typing_while_an_approval_waits_denies_it_and_sends_the_words_next() {
    let mut console = desk();
    assert_eq!(console.current(), Some(run_id(2)));
    type_text(&mut console, "no, push to a branch first");
    let out = console.press(Key::Enter);
    assert_eq!(
        sent(&out),
        [Command::Answer {
            run: run_id(2),
            approval: "ap-2".parse().unwrap(),
            decision: Decision::Decline,
        }]
    );
    let out = console.apply(state(2, idle(2)));
    assert_eq!(
        sent(&out),
        [Command::Prompt {
            run: run_id(2),
            prompt: Prompt::parse("no, push to a branch first".into()).unwrap(),
        }]
    );
}

#[test]
fn esc_interrupts_and_alt_arrows_follow_what_needs_you() {
    let mut console = desk();
    // Order: asking (2), your turn (3), working (1).
    assert_eq!(console.current(), Some(run_id(2)));
    let _ = console.press(Key::AltRight);
    assert_eq!(console.current(), Some(run_id(3)));
    let _ = console.press(Key::AltRight);
    assert_eq!(console.current(), Some(run_id(1)));
    assert_eq!(
        sent(&console.press(Key::Esc)),
        [Command::Interrupt(run_id(1))]
    );
    let _ = console.press(Key::AltLeft);
    assert_eq!(console.current(), Some(run_id(3)));
    assert!(
        sent(&console.press(Key::Esc)).is_empty(),
        "nothing runs to interrupt"
    );
}

#[test]
fn new_completes_projects_and_clients_and_starts_with_the_prompt() {
    let mut console = desk();
    type_text(&mut console, "/new acme");
    let frame = console.frame();
    let labels: Vec<&str> = frame.popup.iter().map(|(s, _)| s.label.as_str()).collect();
    assert_eq!(labels, ["acme-api", "acme-web"]);
    let _ = console.press(Key::Down);
    let _ = console.press(Key::Tab);
    type_text(&mut console, "co");
    let _ = console.press(Key::Tab);
    type_text(&mut console, "add the upload form");
    let out = console.press(Key::Enter);
    assert_eq!(
        sent(&out),
        [Command::Start {
            harness: Harness::Codex,
            dir: "/dev/acme-web".into(),
            resume: None,
            prompt: Some(Prompt::parse("add the upload form".into()).unwrap()),
        }]
    );
    // The client named last is the default for the next.
    type_text(&mut console, "/new ren tidy the docs");
    assert!(matches!(
        sent(&console.press(Key::Enter)).as_slice(),
        [Command::Start { harness: Harness::Codex, dir, .. }] if dir == "/srv/ren"
    ));
    // Started: it opens.
    let out = console.apply(Input::Answered(Ok(Answered::Started(run_id(9)))));
    assert!(out.contains(&Out::Read(Read::Runs)));
    assert_eq!(console.current(), Some(run_id(9)));

    type_text(&mut console, "/new acme fix");
    let _ = console.press(Key::Esc);
    assert!(sent(&console.press(Key::Enter)).is_empty());
    assert!(drawn(&console).contains("which one: acme-api, acme-web"));
}

#[test]
fn a_live_turn_gives_way_to_its_record() {
    let mut console = desk();
    let _ = console.apply(Input::Recorded(vec![recorded(
        3,
        "/dev/acme-web",
        Some(SESSION),
    )]));
    let _ = console.press(Key::AltRight);
    assert_eq!(console.current(), Some(run_id(3)));
    let _ = console.apply(Input::History {
        run: run_id(3),
        turns: vec![recorded_turn("old", "earlier work", "Done earlier.")],
    });
    type_text(&mut console, "fix the flaky upload test");
    assert_eq!(sent(&console.press(Key::Enter)).len(), 1);
    let _ = console.apply(state(3, working(3)));
    let _ = console.apply(event(3, json!({ "kind": "turn_started", "turn": turn(3) })));
    let _ = console.apply(event(
        3,
        json!({ "kind": "delta", "item": "m1", "text": "The lease " }),
    ));
    let _ = console.apply(event(
        3,
        json!({ "kind": "delta", "item": "m1", "text": "token…" }),
    ));
    assert!(drawn(&console).contains("The lease token…"));
    let out = console.apply(event(
        3,
        json!({ "kind": "turn_ended", "turn": turn(3), "end": { "kind": "completed" } }),
    ));
    assert_eq!(
        out,
        [Out::Read(Read::HistoryLater {
            run: run_id(3),
            session: session(),
        })]
    );
    // Not recorded yet: asked again later.
    let out = console.apply(Input::History {
        run: run_id(3),
        turns: vec![recorded_turn("old", "earlier work", "Done earlier.")],
    });
    assert_eq!(
        out,
        [Out::Read(Read::HistoryLater {
            run: run_id(3),
            session: session(),
        })]
    );
    // Recorded under its own id (Claude Code's prompt id): matched by prompt.
    let out = console.apply(Input::History {
        run: run_id(3),
        turns: vec![
            recorded_turn("old", "earlier work", "Done earlier."),
            recorded_turn("new", "fix the flaky upload test", "The lease token…"),
        ],
    });
    assert!(out.is_empty());
    let shown = drawn(&console);
    assert_eq!(
        shown.matches("fix the flaky upload test").count(),
        1,
        "{shown}"
    );
}

#[test]
fn a_message_to_an_ended_run_continues_its_conversation() {
    let mut console = Console::new(SIZE);
    let mut ended = recorded(5, "/srv/ren", None);
    ended.ended_at = Some(ren_recorder::time::UnixMillis::from_millis(2000));
    let _ = console.apply(Input::Recorded(vec![ended]));
    let _ = console.apply(Input::Runs(Ok(vec![])));
    type_text(&mut console, "/resume");
    let _ = console.press(Key::Enter);
    let _ = console.press(Key::Enter);
    assert_eq!(console.current(), Some(run_id(5)));
    type_text(&mut console, "pick it back up");
    assert_eq!(
        sent(&console.press(Key::Enter)),
        [Command::Start {
            harness: Harness::ClaudeCode,
            dir: "/srv/ren".into(),
            resume: Some(thread(5)),
            prompt: Some(Prompt::parse("pick it back up".into()).unwrap()),
        }]
    );
}

#[test]
fn refusals_read_in_words_and_quitting_leaves_runs_alone() {
    let mut console = desk();
    let _ = console.apply(Input::Answered(Err("invalid_directory".into())));
    assert!(drawn(&console).contains("that folder is not under one of the daemon's run roots"));
    type_text(&mut console, "half typed");
    assert!(
        console.press(Key::Interrupt).is_empty(),
        "ctrl-c clears first"
    );
    assert_eq!(console.press(Key::Interrupt), [Out::Quit]);
}

#[test]
fn screens_draw_as_pinned() {
    let mut console = desk();
    let _ = console.apply(Input::History {
        run: run_id(2),
        turns: vec![recorded_turn(
            "t1",
            "fix the flaky upload test",
            "The lease token was compared before the retry counter was bumped, so a late worker could still finish the job. I moved the check after the bump.",
        )],
    });
    assert_eq!(
        drawn(&console),
        include_str!("../../tests/fixtures/console-run-asking.txt")
    );

    type_text(&mut console, "/new ac");
    assert_eq!(
        drawn(&console),
        include_str!("../../tests/fixtures/console-new-completion.txt")
    );
    let _ = console.press(Key::Interrupt);

    type_text(&mut console, "/resume");
    let _ = console.press(Key::Enter);
    assert_eq!(
        drawn(&console),
        include_str!("../../tests/fixtures/console-resume.txt")
    );

    let mut quiet = Console::new(SIZE);
    let _ = quiet.apply(Input::Projects(Ok(projects())));
    let _ = quiet.apply(Input::Runs(Ok(vec![
        snapshot(1, Harness::Codex, "/dev/acme-api", working(1)),
        snapshot(3, Harness::ClaudeCode, "/dev/acme-web", idle(3)),
    ])));
    assert_eq!(
        drawn(&quiet),
        include_str!("../../tests/fixtures/console-home.txt")
    );
}

#[test]
fn a_new_runs_first_turn_gives_way_to_its_record_by_prompt() {
    let mut console = Console::new(SIZE);
    let _ = console.apply(Input::Runs(Ok(vec![snapshot(
        4,
        Harness::ClaudeCode,
        "/srv/ren",
        idle(4),
    )])));
    // Started here: recorded, with no call yet.
    let _ = console.apply(Input::Recorded(vec![recorded(4, "/srv/ren", None)]));
    let _ = console.press(Key::AltRight);
    type_text(&mut console, "run the tests");
    let _ = console.press(Key::Enter);
    let _ = console.apply(event(4, json!({ "kind": "turn_started", "turn": turn(4) })));
    let _ = console.apply(event(
        4,
        json!({ "kind": "delta", "item": "m", "text": "All pass." }),
    ));
    let out = console.apply(event(
        4,
        json!({ "kind": "turn_ended", "turn": turn(4), "end": { "kind": "completed" } }),
    ));
    // Its session is not known yet: the recorded runs are read, then its turns.
    assert_eq!(out, [Out::Read(Read::Recorded)]);
    let out = console.apply(Input::Recorded(vec![recorded(
        4,
        "/srv/ren",
        Some(SESSION),
    )]));
    assert_eq!(
        out,
        [Out::Read(Read::History {
            run: run_id(4),
            session: session()
        })]
    );
    let _ = console.apply(Input::History {
        run: run_id(4),
        turns: vec![recorded_turn("prompt-id", "run the tests", "All pass.")],
    });
    assert_eq!(drawn(&console).matches("run the tests").count(), 1);
}
