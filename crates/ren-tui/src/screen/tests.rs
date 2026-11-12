//! Runs in the app: selecting them, answering, writing, starting, and how
//! a live turn gives way to its record, with synthetic runs, events, and
//! recorded turns, run as the terminal runs them: a [`Tui`]. No daemon, no
//! terminal. Screens are pinned as text in `tests/fixtures`.

use super::main::Item;
use super::main::Mode;
use crate::{Answered, Command, Input, Key, Out, Read, Size, Theme, Tui};
use ratatui::{buffer::Buffer, layout::Rect};
use ren_harness::{
    Harness,
    drive::{ApprovalAsk, Decision, DriveEvent, Prompt},
};
use ren_protocol::native::{NativeThreadId, NativeTurnId};
use ren_recorder::id::{RunId, SessionId};
use ren_recorder::time::UnixMillis;
use ren_store::{activity::TurnActivity, view::RunView};
use ren_supervisor::{Approvals, Project, ProjectGit, RunSnapshot, RunState, RunUpdate};
use serde_json::json;

const SIZE: Size = Size {
    width: 120,
    height: 30,
};

/// The console as the terminal runs it.
struct Console {
    tui: Tui,
}

impl Console {
    fn new(size: Size) -> Self {
        Self {
            tui: Tui::new(size),
        }
    }

    fn start(&mut self) -> Vec<Out> {
        self.tui.start()
    }

    fn apply(&mut self, input: Input) -> Vec<Out> {
        self.tui.apply(input)
    }

    fn press(&mut self, key: Key) -> Vec<Out> {
        self.tui.press(key)
    }

    fn current(&self) -> Option<RunId> {
        self.tui.shown_run()
    }

    /// Writes `text` to the selected run and sends it.
    fn write(&mut self, text: &str) -> Vec<Out> {
        let _ = self.press(Key::Char('i'));
        type_text(self, text);
        self.press(Key::Enter)
    }
}

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
            Out::Read(_) | Out::CycleTheme | Out::Quit => None,
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
    let area = Rect::new(0, 0, SIZE.width, SIZE.height);
    let mut buf = Buffer::empty(area);
    console
        .tui
        .view(UnixMillis::from_millis(0))
        .render(&Theme::default(), area, &mut buf);
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
fn the_run_that_needs_you_is_selected_first() {
    let mut console = Console::new(SIZE);
    let start = console.start();
    for read in [Read::Snapshot, Read::Runs, Read::Recorded, Read::Projects] {
        assert!(start.contains(&Out::Read(read)), "{start:?}");
    }
    let out = console.apply(Input::Runs(Ok(vec![
        snapshot(1, Harness::Codex, "/dev/acme-api", working(1)),
        snapshot(2, Harness::ClaudeCode, "/srv/ren", asking(2, push())),
    ])));
    assert_eq!(console.current(), Some(run_id(2)));
    // Its session is not known yet: the recorded runs are read first.
    assert_eq!(out, [Out::Read(Read::Recorded)]);
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
fn a_key_answers_and_moves_on_to_the_next_run_that_needs_you() {
    let mut console = desk();
    assert_eq!(console.current(), Some(run_id(2)));
    let _ = console.apply(state(
        1,
        asking(
            1,
            ApprovalAsk::FileChange {
                reason: Some("src/orders/query.rs".into()),
            },
        ),
    ));
    assert_eq!(console.current(), Some(run_id(2)), "the selection stays");
    let out = console.press(Key::Char('a'));
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
        "on to the other asking run"
    );
    let shown = drawn(&console);
    assert!(shown.contains("↳ from ren"), "{shown}");
    assert!(
        shown.contains("allowed git push origin fix/upload-lease"),
        "{shown}"
    );
    assert!(shown.contains("Codex wants to change files"), "{shown}");
    // Nothing waits on a run that does not ask.
    let _ = console.press(Key::Char('j'));
    assert!(sent(&console.press(Key::Char('y'))).is_empty());
}

#[test]
fn writing_during_a_turn_waits_for_its_end() {
    let mut console = desk();
    // The list: needs you (2), working (1), your turn (3).
    let _ = console.press(Key::Char('j'));
    assert_eq!(console.current(), Some(run_id(1)));
    assert!(sent(&console.write("also check the dead-letter path")).is_empty());
    assert_eq!(console.tui.mode(), Mode::Normal);
    assert!(
        drawn(&console).contains("queued: also check the dead-letter path")
            || drawn(&console).contains("kept: it is sent when this turn ends")
    );
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
fn writing_while_an_approval_waits_denies_it_and_sends_the_words_next() {
    let mut console = desk();
    let out = console.write("no, push to a branch first");
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
fn keys_move_through_the_list_and_x_interrupts() {
    let mut console = desk();
    let _ = console.press(Key::Char('j'));
    assert_eq!(console.current(), Some(run_id(1)));
    assert_eq!(
        sent(&console.press(Key::Char('x'))),
        [Command::Interrupt(run_id(1))]
    );
    let _ = console.press(Key::Down);
    assert_eq!(console.current(), Some(run_id(3)));
    assert!(
        sent(&console.press(Key::Char('x'))).is_empty(),
        "nothing runs"
    );
    let _ = console.press(Key::Char('k'));
    let _ = console.press(Key::Up);
    assert_eq!(console.current(), Some(run_id(2)));
    // A digit jumps to its entry.
    let _ = console.press(Key::Char('3'));
    assert_eq!(console.current(), Some(run_id(3)));
    // A search narrows the list; enter keeps it.
    let _ = console.press(Key::Char('/'));
    type_text(&mut console, "api");
    let _ = console.press(Key::Enter);
    assert_eq!(console.current(), Some(run_id(1)));
    assert_eq!(console.tui.selected(), Some(Item::Run(run_id(1))));
}

#[test]
fn n_opens_the_new_run_form_and_starts_with_the_prompt() {
    let mut console = desk();
    let _ = console.press(Key::Char('n'));
    assert!(console.tui.overlaid());
    type_text(&mut console, "web");
    let _ = console.press(Key::Tab);
    let _ = console.press(Key::Char(' '));
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
    assert!(!console.tui.overlaid());
    // Started: it is selected.
    let out = console.apply(Input::Answered(Ok(Answered::Started(run_id(9)))));
    assert!(out.contains(&Out::Read(Read::Runs)));
    assert_eq!(console.current(), Some(run_id(9)));
    // Esc leaves the form without starting anything.
    let _ = console.press(Key::Char('n'));
    let _ = console.press(Key::Esc);
    assert!(!console.tui.overlaid());
}

/// Claude Code and Codex installed, Pi not, OpenCode not drivable.
fn clients() -> Vec<ren_supervisor::Client> {
    use ren_supervisor::{Client, ClientStatus};
    vec![
        Client {
            harness: Harness::ClaudeCode,
            status: ClientStatus::Ready,
        },
        Client {
            harness: Harness::Codex,
            status: ClientStatus::Ready,
        },
        Client {
            harness: Harness::Pi,
            status: ClientStatus::NotInstalled,
        },
        Client {
            harness: Harness::OpenCode,
            status: ClientStatus::NotDrivable,
        },
    ]
}

#[test]
fn the_form_offers_the_clients_installed_on_the_daemons_host() {
    let mut console = desk();
    let _ = console.press(Key::Char('n'));
    // Unread, every client Ren drives is offered.
    assert!(drawn(&console).contains("○ Pi   "));
    let _ = console.apply(Input::Clients(Ok(clients())));
    let form = drawn(&console);
    assert!(form.contains("○ Pi (not installed)"), "{form}");
    assert!(!form.contains("OpenCode"));
    // Choosing steps over a client that is not there, both ways.
    let _ = console.press(Key::Tab);
    let _ = console.press(Key::Right);
    assert!(drawn(&console).contains("◉ Codex"));
    let _ = console.press(Key::Right);
    assert!(drawn(&console).contains("◉ Claude Code"));
    let _ = console.press(Key::Left);
    assert!(drawn(&console).contains("◉ Codex"));
    // Should the chosen client be gone from the host, starting says why.
    let _ = console.apply(Input::Clients(Ok(vec![ren_supervisor::Client {
        harness: Harness::Codex,
        status: ren_supervisor::ClientStatus::NotInstalled,
    }])));
    let _ = console.press(Key::Tab);
    type_text(&mut console, "go");
    let out = console.press(Key::Enter);
    assert!(sent(&out).is_empty());
    assert!(drawn(&console).contains("Codex is not installed on the daemon's host"));
}

#[test]
fn a_live_turn_gives_way_to_its_record() {
    let mut console = desk();
    let _ = console.apply(Input::Recorded(vec![recorded(
        3,
        "/dev/acme-web",
        Some(SESSION),
    )]));
    let _ = console.press(Key::Char('3'));
    assert_eq!(console.current(), Some(run_id(3)));
    let _ = console.apply(Input::History {
        run: run_id(3),
        turns: vec![recorded_turn("old", "earlier work", "Done earlier.")],
    });
    assert_eq!(sent(&console.write("fix the flaky upload test")).len(), 1);
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
fn writing_to_an_ended_run_continues_its_conversation() {
    let mut console = Console::new(SIZE);
    let mut ended = recorded(5, "/srv/ren", None);
    ended.ended_at = Some(UnixMillis::from_millis(2000));
    let _ = console.apply(Input::Recorded(vec![ended]));
    let _ = console.apply(Input::Runs(Ok(vec![])));
    assert_eq!(console.current(), Some(run_id(5)));
    assert_eq!(
        sent(&console.write("pick it back up")),
        [Command::Start {
            harness: Harness::ClaudeCode,
            dir: "/srv/ren".into(),
            resume: Some(thread(5)),
            prompt: Some(Prompt::parse("pick it back up".into()).unwrap()),
        }]
    );
}

#[test]
fn refusals_read_in_words_and_esc_and_ctrl_c_leave_writing() {
    let mut console = desk();
    let _ = console.apply(Input::Answered(Err("invalid_directory".into())));
    assert!(drawn(&console).contains("that folder is not under one of the daemon's run roots"));
    let _ = console.press(Key::Char('i'));
    assert_eq!(console.tui.mode(), Mode::Writing);
    type_text(&mut console, "half typed");
    // Keys type while writing: q is a letter.
    assert!(console.press(Key::Char('q')).is_empty());
    let _ = console.press(Key::Esc);
    assert_eq!(console.tui.mode(), Mode::Normal);
    assert!(drawn(&console).contains("half typedq"), "the draft stays");
    let _ = console.press(Key::Char('i'));
    let _ = console.press(Key::Interrupt);
    let _ = console.press(Key::Interrupt);
    assert_eq!(console.tui.mode(), Mode::Normal);
    assert_eq!(console.press(Key::Char('q')), [Out::Quit]);
}

#[test]
fn earlier_turns_fold_and_open_from_the_session() {
    let mut console = desk();
    let _ = console.apply(Input::History {
        run: run_id(2),
        turns: vec![
            recorded_turn("t1", "first ask", "First answer."),
            recorded_turn("t2", "second ask", "Second answer."),
        ],
    });
    let shown = drawn(&console);
    assert!(!shown.contains("First answer."), "{shown}");
    assert!(shown.contains("Second answer."), "{shown}");
    // Into the session, up to the earlier turn, and open it.
    let _ = console.press(Key::Enter);
    let _ = console.press(Key::Char('k'));
    let _ = console.press(Key::Enter);
    assert!(drawn(&console).contains("First answer."));
}

#[test]
fn runs_are_grouped_by_folder_or_by_what_they_need() {
    let mut console = Console::new(SIZE);
    let _ = console.apply(Input::Runs(Ok(vec![
        snapshot(1, Harness::Codex, "/dev/acme-api", idle(1)),
        snapshot(2, Harness::ClaudeCode, "/srv/ren", working(2)),
        snapshot(3, Harness::ClaudeCode, "/dev/acme-api", asking(3, push())),
    ])));
    // By folder: the folder with the run that needs you first, that run
    // first in it, the other run of the folder next.
    assert_eq!(console.current(), Some(run_id(3)));
    let _ = console.press(Key::Char('j'));
    assert_eq!(console.current(), Some(run_id(1)));
    let shown = drawn(&console);
    assert!(shown.contains(" acme-api ◆1  /dev/"), "{shown}");
    assert_eq!(
        shown.matches(" acme-api ◆1").count(),
        1,
        "one heading: {shown}"
    );
    assert!(shown.contains(" 2 ○ new Codex run  your turn"), "{shown}");
    // By what each needs: needs you, working, your turn.
    let _ = console.press(Key::Char('g'));
    let _ = console.press(Key::Char('1'));
    assert_eq!(console.current(), Some(run_id(3)));
    let _ = console.press(Key::Char('j'));
    assert_eq!(console.current(), Some(run_id(2)));
    assert!(drawn(&console).contains(" WORKING"));
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
        include_str!("../../tests/fixtures/app-run-asking.txt")
    );
    let _ = console.apply(Input::Clients(Ok(clients())));
    let _ = console.press(Key::Char('n'));
    type_text(&mut console, "ac");
    assert_eq!(
        drawn(&console),
        include_str!("../../tests/fixtures/app-new-run.txt")
    );
    let _ = console.press(Key::Esc);
    let _ = console.press(Key::Tab);
    assert_eq!(
        drawn(&console),
        include_str!("../../tests/fixtures/app-activity.txt")
    );
    let _ = console.press(Key::Char('?'));
    assert_eq!(
        drawn(&console),
        include_str!("../../tests/fixtures/app-help.txt")
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
    assert_eq!(console.current(), Some(run_id(4)));
    let _ = console.write("run the tests");
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
    // Once in the conversation (the list names the run by it too).
    let shown = drawn(&console);
    assert_eq!(shown.matches("run the tests  1 s").count(), 1, "{shown}");
    assert_eq!(shown.matches("All pass.").count(), 1, "{shown}");
}

#[test]
fn an_ended_run_is_archived_or_deleted_with_a_second_press_and_leaves_the_list() {
    use ren_recorder::batch::{HistoryChange, HistorySubject};
    let mut console = Console::new(SIZE);
    let mut ended = recorded(5, "/srv/ren", None);
    ended.ended_at = Some(UnixMillis::from_millis(2000));
    let _ = console.apply(Input::Recorded(vec![ended.clone()]));
    let _ = console.apply(Input::Runs(Ok(vec![snapshot(
        1,
        Harness::Codex,
        "/dev/acme-api",
        idle(1),
    )])));
    // A live run is not archived away.
    let _ = console.press(Key::Char('1'));
    let out = console.press(Key::Char('e'));
    assert!(sent(&out).is_empty());
    assert!(drawn(&console).contains("stop the run first"));

    let _ = console.press(Key::Char('2'));
    assert_eq!(console.current(), Some(run_id(5)));
    let subject = HistorySubject::Run(run_id(5));
    assert_eq!(
        sent(&console.press(Key::Char('e'))),
        [Command::Edit {
            subject,
            change: HistoryChange::Archive,
        }]
    );
    // # asks once more; another key in between keeps it.
    assert!(sent(&console.press(Key::Char('#'))).is_empty());
    assert!(drawn(&console).contains("press # again"));
    let _ = console.press(Key::Char('j'));
    assert!(sent(&console.press(Key::Char('#'))).is_empty());
    assert_eq!(
        sent(&console.press(Key::Char('#'))),
        [Command::Edit {
            subject,
            change: HistoryChange::Delete,
        }]
    );
    // Once history no longer lists it, the ended run leaves the desk.
    assert!(drawn(&console).contains("/srv/ren"));
    let _ = console.apply(Input::Recorded(vec![]));
    assert!(!drawn(&console).contains("srv/ren"));
}
