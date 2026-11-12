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
use ren_recorder::id::RunId;
use ren_recorder::time::UnixMillis;
use ren_store::view::RunView;
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
        settings: ren_harness::settings::RunSettings::default(),
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

/// What the daemon stored of run `n`'s conversation: each turn's prompt,
/// a command, and its reply.
fn stored(n: u128, turns: &[(&str, &str)]) -> Input {
    let mut entries = Vec::new();
    for (index, (prompt, reply)) in turns.iter().enumerate() {
        let turn = NativeTurnId::new(uuid::Uuid::from_u128(
            n * 100 + u128::try_from(index).unwrap() + 1,
        ))
        .unwrap();
        for event in [
            json!({ "kind": "turn_started", "turn": turn }),
            json!({ "kind": "item", "phase": "completed", "item": {
                "id": format!("u{n}-{index}"), "kind": "user_message",
                "read": { "as": "prompt", "text": prompt }, "body": null } }),
            json!({ "kind": "item", "phase": "completed", "item": {
                "id": format!("c{n}-{index}"), "kind": "command_execution",
                "read": { "as": "command", "command": "cargo test -p upload", "status": "completed",
                          "output": "ok", "exit_code": 0 }, "body": null } }),
            json!({ "kind": "item", "phase": "completed", "item": {
                "id": format!("m{n}-{index}"), "kind": "agent_message",
                "read": { "as": "reply", "text": reply }, "body": null } }),
            json!({ "kind": "turn_ended", "turn": turn, "end": { "kind": "completed" } }),
        ] {
            entries.push(json!({ "run": run_id(n), "seq": entries.len() + 1, "at": 0,
                                 "turn": turn, "event": event }));
        }
    }
    Input::Conversation {
        run: run_id(n),
        conversation: serde_json::from_value(json!({ "entries": entries, "earlier": false }))
            .unwrap(),
    }
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
            Out::Read(_) | Out::CycleTheme | Out::Edit(_) | Out::Quit => None,
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
    // Its conversation is read at once: what the daemon stored of it.
    assert!(
        out.contains(&Out::Read(Read::Conversation(run_id(2)))),
        "{out:?}"
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
fn keys_move_through_the_list_and_the_action_menu_interrupts() {
    let mut console = desk();
    let _ = console.press(Key::Char('j'));
    assert_eq!(console.current(), Some(run_id(1)));
    // A letter alone does nothing; the menu lists what applies.
    assert!(console.press(Key::Char('x')).is_empty());
    let _ = console.press(Key::Char(' '));
    assert!(console.tui.overlaid());
    assert_eq!(
        sent(&console.press(Key::Char('x'))),
        [Command::Interrupt(run_id(1))]
    );
    assert!(!console.tui.overlaid());
    let _ = console.press(Key::Down);
    assert_eq!(console.current(), Some(run_id(3)));
    // Nothing runs there: the menu has no interrupt, and stays open.
    let _ = console.press(Key::Char(' '));
    assert!(sent(&console.press(Key::Char('x'))).is_empty());
    assert!(console.tui.overlaid());
    let _ = console.press(Key::Esc);
    assert!(!console.tui.overlaid());
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
fn n_opens_the_composer_on_where_the_run_works_then_the_prompt() {
    let mut console = desk();
    let _ = console.press(Key::Char('n'));
    assert!(console.tui.overlaid());
    // It opens on the folder: nothing is assumed, recent runs' folders first.
    let list = drawn(&console);
    assert!(list.contains("where should the run work?"), "{list}");
    assert!(list.contains("folder none yet"), "{list}");
    // Both folders had runs here: the newest run's first.
    let recent = list.find(" recent").unwrap();
    let web = list.find("acme-web  /dev/acme-web").unwrap();
    let api = list.find("acme-api  /dev/acme-api").unwrap();
    assert!(recent < web && web < api, "{list}");
    // Esc goes to the prompt with no folder; enter then asks for one.
    let _ = console.press(Key::Esc);
    type_text(&mut console, "add the upload form");
    let out = console.press(Key::Enter);
    assert!(sent(&out).is_empty());
    assert!(drawn(&console).contains("choose the folder the run works in"));
    // The list is open again: type to search, enter takes it.
    type_text(&mut console, "web");
    let _ = console.press(Key::Enter);
    // Back in the prompt, it takes typing and pastes with their line breaks.
    let _ = console.press(Key::Newline);
    assert!(
        console
            .tui
            .paste("with a progress bar\nand a retry button")
            .is_empty()
    );
    // Ctrl-L opens the client's list while writing; ↓ picks Codex.
    let _ = console.press(Key::Client);
    let _ = console.press(Key::Down);
    let _ = console.press(Key::Enter);
    let out = console.press(Key::Enter);
    assert_eq!(
        sent(&out),
        [Command::Start {
            harness: Harness::Codex,
            dir: "/dev/acme-web".into(),
            resume: None,
            prompt: Some(
                Prompt::parse(
                    "add the upload form\nwith a progress bar\nand a retry button".into()
                )
                .unwrap()
            ),
            settings: ren_harness::settings::RunSettings::default(),
        }]
    );
    assert!(!console.tui.overlaid());
    // Started: it is selected.
    let out = console.apply(Input::Answered(Ok(Answered::Started(run_id(9)))));
    assert!(out.contains(&Out::Read(Read::Runs)));
    assert_eq!(console.current(), Some(run_id(9)));
    // The next composer keeps the client, and still asks where.
    let _ = console.press(Key::Char('n'));
    let shown = drawn(&console);
    assert!(shown.contains("folder none yet"), "{shown}");
    assert!(shown.contains("client Codex"), "{shown}");
    // Ctrl-E hands the prompt to the editor; what it saves comes back.
    let _ = console.press(Key::Esc);
    type_text(&mut console, "draft");
    assert_eq!(console.press(Key::Editor), [Out::Edit("draft".into())]);
    let _ = console.tui.edited(Ok("the whole plan\n"));
    assert!(drawn(&console).contains("the whole plan"));
    let _ = console.tui.edited(Err("vi exited with an error"));
    assert!(drawn(&console).contains("not edited: vi exited with an error"));
    // Esc leaves the composer without starting anything.
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
            approvals: ren_harness::settings::ApprovalPolicy::offered(Harness::ClaudeCode).to_vec(),
        },
        Client {
            harness: Harness::Codex,
            status: ClientStatus::Ready,
            approvals: ren_harness::settings::ApprovalPolicy::offered(Harness::Codex).to_vec(),
        },
        Client {
            harness: Harness::Pi,
            status: ClientStatus::NotInstalled,
            approvals: ren_harness::settings::ApprovalPolicy::offered(Harness::Pi).to_vec(),
        },
        Client {
            harness: Harness::OpenCode,
            status: ClientStatus::NotDrivable,
            approvals: ren_harness::settings::ApprovalPolicy::offered(Harness::OpenCode).to_vec(),
        },
    ]
}

/// Models as a client lists them: one is its default.
fn models() -> Vec<ren_harness::models::Model> {
    let model = |id: &str, name: &str, description: &str, choice| ren_harness::models::Model {
        id: id.parse().unwrap(),
        name: name.into(),
        description: Some(description.into()),
        provider: None,
        choice,
        efforts: vec![Effort::Low, Effort::Medium, Effort::High],
        effort: Some(Effort::Medium),
    };
    use ren_harness::models::DefaultChoice::{Default, Other};
    use ren_harness::settings::Effort;
    vec![
        model("big", "Big 2", "Best for everyday, complex tasks", Default),
        model(
            "deep",
            "Deep 2",
            "Most capable for the hardest tasks",
            Other,
        ),
        model("small", "Small 2", "Fastest for quick answers", Other),
    ]
}

#[test]
fn the_composer_starts_on_the_model_chosen_from_the_clients_list() {
    let mut console = desk();
    let _ = console.apply(Input::Clients(Ok(clients())));
    // Opening the composer asks the client for its models.
    let out = console.press(Key::Char('n'));
    assert!(out.contains(&Out::Read(Read::Models(Harness::ClaudeCode))));
    type_text(&mut console, "ac");
    let _ = console.press(Key::Enter);
    let _ = console.apply(Input::Models {
        harness: Harness::ClaudeCode,
        models: Ok(models()),
    });
    let shown = drawn(&console);
    assert!(shown.contains("ctrl-k model default · Big 2"), "{shown}");
    assert!(shown.contains("shift-tab approvals default"), "{shown}");
    let _ = console.press(Key::Model);
    assert_eq!(
        drawn(&console),
        include_str!("../../tests/fixtures/app-new-run-model.txt")
    );
    // Typing narrows the list: the default stays first. ←→ choose the
    // highlighted model's effort.
    type_text(&mut console, "fast");
    let _ = console.press(Key::Down);
    for _ in 0..4 {
        let _ = console.press(Key::Right);
    }
    let _ = console.press(Key::Left);
    let shown = drawn(&console);
    assert!(
        shown.contains("←→ effort   default (medium)  low  medium  high"),
        "{shown}"
    );
    let _ = console.press(Key::Enter);
    assert!(drawn(&console).contains("ctrl-k model Small 2 · medium effort"));
    // Shift-Tab cycles what the client may do alone, back to its own.
    for _ in 0..2 {
        let _ = console.press(Key::BackTab);
    }
    assert!(drawn(&console).contains("shift-tab approvals edits alone"));
    type_text(&mut console, "tidy up");
    let out = console.press(Key::Enter);
    let started = sent(&out);
    let [Command::Start { settings, .. }] = started.as_slice() else {
        panic!("one start: {out:?}");
    };
    assert_eq!(
        settings,
        &ren_harness::settings::RunSettings {
            model: Some("small".parse().unwrap()),
            effort: Some(ren_harness::settings::Effort::Medium),
            approvals: Some(ren_harness::settings::ApprovalPolicy::Edits),
        }
    );
    // The next composer remembers the choice; a client that fails to list
    // says why.
    let _ = console.press(Key::Char('n'));
    assert!(drawn(&console).contains("Small 2 · medium effort"));
    let _ = console.apply(Input::Models {
        harness: Harness::Codex,
        models: Err("the client did not list its models in time".into()),
    });
    let _ = console.press(Key::Client);
    let _ = console.press(Key::Down);
    // Choosing Codex asks again, since its listing failed.
    let out = console.press(Key::Enter);
    assert_eq!(out, [Out::Read(Read::Models(Harness::Codex))]);
    let shown = drawn(&console);
    let setup = shown.lines().nth(1).unwrap_or_default();
    assert!(
        setup.contains("ctrl-k model default   shift-tab approvals default"),
        "{setup}"
    );
    // Asked already: opening the list does not ask twice.
    assert!(console.press(Key::Model).is_empty());
    assert!(drawn(&console).contains("asking Codex for its models…"));
}

#[test]
fn the_composer_offers_the_clients_installed_on_the_daemons_host() {
    let mut console = desk();
    let _ = console.apply(Input::Clients(Ok(clients())));
    let _ = console.press(Key::Char('n'));
    let _ = console.press(Key::Client);
    let list = drawn(&console);
    assert!(list.contains("Pi  not installed"), "{list}");
    assert!(!list.contains("OpenCode"));
    // A client not there is not taken, and says why.
    let _ = console.press(Key::Down);
    let _ = console.press(Key::Down);
    assert!(sent(&console.press(Key::Enter)).is_empty());
    assert!(drawn(&console).contains("Pi is not installed on the daemon's host"));
    // Should the chosen client be gone from the host, starting says why.
    let _ = console.press(Key::Folder);
    let _ = console.press(Key::Enter);
    let _ = console.apply(Input::Clients(Ok(vec![ren_supervisor::Client {
        harness: Harness::ClaudeCode,
        status: ren_supervisor::ClientStatus::NotInstalled,
        approvals: Vec::new(),
    }])));
    type_text(&mut console, "go");
    let out = console.press(Key::Enter);
    assert!(sent(&out).is_empty());
    assert!(drawn(&console).contains("Claude Code is not installed on the daemon's host"));
}

#[test]
fn a_run_shows_its_whole_conversation_and_what_streams_on_top() {
    let mut console = desk();
    let _ = console.press(Key::Char('3'));
    assert_eq!(console.current(), Some(run_id(3)));
    let long = "The lease token was compared before the retry counter. ".repeat(40);
    let _ = console.apply(stored(
        3,
        &[
            ("earlier work", "Done earlier."),
            ("look at the lease", &long),
        ],
    ));
    assert_eq!(sent(&console.write("fix the flaky upload test")).len(), 1);
    let _ = console.apply(state(3, working(3)));
    let _ = console.apply(event(3, json!({ "kind": "turn_started", "turn": turn(3) })));
    let _ = console.apply(event(
        3,
        json!({ "kind": "delta", "item": "m1", "text": "Moving the " }),
    ));
    let _ = console.apply(event(
        3,
        json!({ "kind": "delta", "item": "m1", "text": "check." }),
    ));
    let out = console.apply(event(
        3,
        json!({ "kind": "turn_ended", "turn": turn(3), "end": { "kind": "completed" } }),
    ));
    // Nothing is read again: the conversation is already whole.
    assert!(out.is_empty(), "{out:?}");
    // Every turn stays open: the first reply and the newest both show,
    // and the long reply is not cut (its last words are there).
    let _ = console.press(Key::Enter);
    for _ in 0..60 {
        let _ = console.press(Key::Up);
    }
    let top = drawn(&console);
    assert!(top.contains("Done earlier."), "{top}");
    assert!(!top.contains("▸ "), "nothing is folded: {top}");
    let _ = console.press(Key::PageDown);
    let _ = console.press(Key::PageDown);
    let _ = console.press(Key::PageDown);
    let bottom = drawn(&console);
    assert!(bottom.contains("Moving the check."), "{bottom}");
    assert_eq!(
        bottom.matches("fix the flaky upload test").count(),
        1,
        "{bottom}"
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
            settings: ren_harness::settings::RunSettings::default(),
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
    let _ = console.press(Key::Char(' '));
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
    let _ = console.apply(stored(
        2,
        &[(
            "fix the flaky upload test",
            "The lease token was compared before the retry counter was bumped, so a late worker could still finish the job. I moved the check after the bump.",
        )],
    ));
    assert_eq!(
        drawn(&console),
        include_str!("../../tests/fixtures/app-run-asking.txt")
    );
    let _ = console.apply(Input::Clients(Ok(clients())));
    // The composer opens on where the run works.
    let _ = console.press(Key::Char('n'));
    type_text(&mut console, "ac");
    assert_eq!(
        drawn(&console),
        include_str!("../../tests/fixtures/app-new-run-project.txt")
    );
    let _ = console.press(Key::Enter);
    let _ = console.tui.paste(
        "The upload test is flaky on CI: it fails about one run in ten with a lease timeout. \
         Look at how the worker renews its lease in src/upload/worker.rs, and why the retry \
         counter can be bumped after the lease check.\n\n\
         Fix it, add a test that reproduces the race every time, and push a branch named \
         fix/upload-lease. Leave the public API as it is.",
    );
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
    let _ = console.press(Key::Esc);
    let _ = console.press(Key::Char(' '));
    assert_eq!(
        drawn(&console),
        include_str!("../../tests/fixtures/app-actions.txt")
    );
}

#[test]
fn a_new_runs_first_prompt_shows_once_and_names_it() {
    let mut console = Console::new(SIZE);
    let _ = console.apply(Input::Runs(Ok(vec![snapshot(
        4,
        Harness::ClaudeCode,
        "/srv/ren",
        idle(4),
    )])));
    let _ = console.apply(Input::Recorded(vec![recorded(4, "/srv/ren", None)]));
    assert_eq!(console.current(), Some(run_id(4)));
    let _ = console.write("run the tests");
    let _ = console.apply(event(4, json!({ "kind": "turn_started", "turn": turn(4) })));
    // The client echoes the prompt under its own id: still one prompt.
    let _ = console.apply(event(
        4,
        json!({ "kind": "item", "phase": "completed", "item": {
        "id": "echo", "kind": "user_message", "read": { "as": "prompt", "text": "run the tests" },
        "body": null } }),
    ));
    let _ = console.apply(event(
        4,
        json!({ "kind": "delta", "item": "m", "text": "All pass." }),
    ));
    let _ = console.apply(event(
        4,
        json!({ "kind": "turn_ended", "turn": turn(4), "end": { "kind": "completed" } }),
    ));
    // The stored copy lands after: the same events, the streamed reply kept
    // whole as the daemon closed it. Nothing shows twice.
    let events = [
        json!({ "kind": "turn_started", "turn": turn(4) }),
        json!({ "kind": "item", "phase": "completed", "item": {
            "id": "echo", "kind": "user_message",
            "read": { "as": "prompt", "text": "run the tests" }, "body": null } }),
        json!({ "kind": "item", "phase": "completed", "item": {
            "id": "m", "kind": "agent_message", "read": { "as": "reply", "text": "All pass." },
            "body": null } }),
        json!({ "kind": "turn_ended", "turn": turn(4), "end": { "kind": "completed" } }),
    ];
    let entries: Vec<serde_json::Value> = events
        .iter()
        .enumerate()
        .map(|(seq, event)| {
            json!({ "run": run_id(4), "seq": seq + 1, "at": 0, "turn": turn(4), "event": event })
        })
        .collect();
    let _ = console.apply(Input::Conversation {
        run: run_id(4),
        conversation: serde_json::from_value(json!({ "entries": entries, "earlier": false }))
            .unwrap(),
    });
    let shown = drawn(&console);
    assert_eq!(shown.matches("All pass.").count(), 1, "{shown}");
    // The list names the run by it, and the conversation shows it once.
    assert_eq!(
        shown.matches("run the tests").count(),
        2,
        "the list, and the prompt: {shown}"
    );
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
    // A live run's menu has no archive: its `a` is not one.
    let _ = console.press(Key::Char('1'));
    let _ = console.press(Key::Char(' '));
    assert!(!drawn(&console).contains("archive"));
    let _ = console.press(Key::Esc);

    let _ = console.press(Key::Char('2'));
    assert_eq!(console.current(), Some(run_id(5)));
    let subject = HistorySubject::Run(run_id(5));
    let _ = console.press(Key::Char(' '));
    assert_eq!(
        sent(&console.press(Key::Char('a'))),
        [Command::Edit {
            subject,
            change: HistoryChange::Archive,
        }]
    );
    // Delete asks once more inside the menu; another key keeps it.
    let _ = console.press(Key::Char(' '));
    assert!(sent(&console.press(Key::Char('d'))).is_empty());
    assert!(drawn(&console).contains("for good? enter does it"));
    assert!(sent(&console.press(Key::Char('j'))).is_empty());
    assert!(console.tui.overlaid(), "kept, the menu stays");
    let _ = console.press(Key::Char('d'));
    assert_eq!(
        sent(&console.press(Key::Enter)),
        [Command::Edit {
            subject,
            change: HistoryChange::Delete,
        }]
    );
    assert!(!console.tui.overlaid());
    // Once history no longer lists it, the ended run leaves the desk.
    assert!(drawn(&console).contains("/srv/ren"));
    let _ = console.apply(Input::Recorded(vec![]));
    assert!(!drawn(&console).contains("srv/ren"));
}

#[test]
fn the_action_menu_lists_only_what_applies_to_the_selection() {
    let mut console = desk();
    let menu = |console: &mut Console| -> Vec<char> {
        let _ = console.press(Key::Char(' '));
        let keys = match console.tui.top_actions() {
            Some(actions) => actions.entries().iter().map(|entry| entry.key).collect(),
            None => Vec::new(),
        };
        let _ = console.press(Key::Esc);
        keys
    };
    // Asking: the answers first, then the run.
    let _ = console.press(Key::Char('1'));
    assert_eq!(console.current(), Some(run_id(2)));
    assert_eq!(
        menu(&mut console),
        ['y', 'a', 'd', 'D', 'i', 'x', 's', 'n', '/', 'g', 't']
    );
    // Working: interrupt and stop.
    let _ = console.press(Key::Char('2'));
    assert_eq!(menu(&mut console), ['i', 'x', 's', 'n', '/', 'g', 't']);
    // Its turn is over: nothing to interrupt.
    let _ = console.press(Key::Char('3'));
    assert_eq!(menu(&mut console), ['i', 's', 'n', '/', 'g', 't']);
    // y answers only while a run asks; elsewhere it does nothing.
    assert!(console.press(Key::Char('y')).is_empty());
}
