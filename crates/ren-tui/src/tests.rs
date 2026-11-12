//! The screens as one app: the wall under everything, the places Alt-← and
//! Alt-→ move between, slash commands everywhere, and the help over each.

use crate::{Input, Key, Opening, Out, Read, Size, Tui};
use ren_harness::Harness;
use ren_harness::drive::ApprovalAsk;
use ren_protocol::native::NativeThreadId;
use ren_recorder::id::RunId;
use ren_recorder::time::UnixMillis;
use ren_supervisor::{Approvals, RunSnapshot, RunState};

const SIZE: Size = Size {
    width: 90,
    height: 24,
};

fn run_id(n: u128) -> RunId {
    RunId::restore(uuid::Uuid::from_u128(n))
}

fn run(n: u128, state: RunState) -> RunSnapshot {
    RunSnapshot {
        run: run_id(n),
        harness: Harness::Codex,
        workdir: format!("/dev/acme-{n}").into(),
        state,
    }
}

fn idle(n: u128) -> RunState {
    RunState::Idle {
        thread: NativeThreadId::new(uuid::Uuid::from_u128(0x7700 + n)).unwrap(),
    }
}

fn asking(n: u128) -> RunState {
    let mut approvals = Approvals::default();
    approvals.insert(
        "ap-1".parse().unwrap(),
        ApprovalAsk::FileChange { reason: None },
    );
    RunState::Working {
        thread: NativeThreadId::new(uuid::Uuid::from_u128(0x7700 + n)).unwrap(),
        turn: None,
        approvals,
    }
}

fn send(tui: &mut Tui, text: &str) -> Vec<Out> {
    let mut outs: Vec<Out> = text
        .chars()
        .flat_map(|ch| tui.press(Key::Char(ch)))
        .collect();
    outs.extend(tui.press(Key::Enter));
    outs
}

fn on_wall(tui: &Tui) -> bool {
    tui.wall_frame(UnixMillis::from_millis(0)).is_some()
}

#[test]
fn starting_reads_what_every_screen_shows() {
    let outs = Tui::new(SIZE, Opening::Wall).start();
    for read in [Read::Snapshot, Read::Runs, Read::Recorded, Read::Projects] {
        assert!(outs.contains(&Out::Read(read)), "{outs:?}");
    }
}

#[test]
fn opened_on_the_runs_the_one_that_needs_you_opens_once_they_are_read() {
    let mut tui = Tui::new(SIZE, Opening::Runs);
    let _ = tui.start();
    assert!(on_wall(&tui), "the wall until runs are read");
    let _ = tui.apply(Input::Runs(Ok(vec![run(1, idle(1)), run(2, asking(2))])));
    assert_eq!(tui.shown_run(), Some(run_id(2)));
    // Only once: going back to the wall stays there.
    let _ = send(&mut tui, "/wall");
    let _ = tui.apply(Input::Runs(Ok(vec![run(1, idle(1)), run(2, asking(2))])));
    assert!(on_wall(&tui));
}

#[test]
fn alt_arrows_move_between_the_wall_and_each_live_run() {
    let mut tui = Tui::new(SIZE, Opening::Wall);
    let _ = tui.apply(Input::Runs(Ok(vec![run(1, idle(1)), run(2, asking(2))])));
    // The wall, then what needs you first.
    let _ = tui.press(Key::AltRight);
    assert_eq!(tui.shown_run(), Some(run_id(2)));
    let _ = tui.press(Key::AltRight);
    assert_eq!(tui.shown_run(), Some(run_id(1)));
    let _ = tui.press(Key::AltRight);
    assert!(on_wall(&tui));
    let _ = tui.press(Key::AltLeft);
    assert_eq!(tui.shown_run(), Some(run_id(1)));
}

#[test]
fn esc_on_an_idle_run_goes_back_to_the_wall_and_slash_commands_work_everywhere() {
    let mut tui = Tui::new(SIZE, Opening::Wall);
    let _ = tui.apply(Input::Runs(Ok(vec![run(1, idle(1))])));
    let _ = tui.press(Key::AltRight);
    assert_eq!(tui.shown_run(), Some(run_id(1)));
    let _ = tui.press(Key::Esc);
    assert!(on_wall(&tui));
    // From the wall, a command typed in its message box.
    let outs = send(&mut tui, "/theme");
    assert_eq!(outs, vec![Out::CycleTheme]);
}

#[test]
fn letters_are_typed_everywhere_and_ctrl_c_clears_then_quits() {
    for opening in [Opening::Wall, Opening::Runs] {
        let mut tui = Tui::new(SIZE, opening);
        assert_eq!(tui.press(Key::Char('q')), Vec::new());
        assert_eq!(tui.press(Key::Interrupt), Vec::new());
        assert_eq!(tui.press(Key::Interrupt), vec![Out::Quit]);
    }
}

#[test]
fn the_help_lists_the_keys_and_commands_of_the_screen_under_it() {
    let mut tui = Tui::new(SIZE, Opening::Wall);
    let _ = send(&mut tui, "/help");
    assert!(tui.help_shown());
    // It takes no other key.
    assert_eq!(tui.press(Key::Char('p')), Vec::new());
    assert_eq!(tui.press(Key::Char('t')), vec![Out::CycleTheme]);
    let _ = tui.press(Key::Esc);
    assert!(!tui.help_shown());
    assert!(on_wall(&tui));
}

#[test]
fn runs_reach_the_wall_and_the_places_line() {
    let mut tui = Tui::new(SIZE, Opening::Wall);
    let _ = tui.apply(Input::Runs(Ok(vec![run(1, idle(1))])));
    let frame = tui.wall_frame(UnixMillis::from_millis(0)).unwrap();
    assert_eq!(frame.status.runs.map(|runs| runs.live), Some(1));
}
