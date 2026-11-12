//! The screens together: which one a key reaches, how they open on one
//! another, and the help over each.

use crate::{Input, Key, Opening, Out, Read, Size, Tui};
use ren_harness::Harness;
use ren_protocol::native::NativeThreadId;
use ren_recorder::id::RunId;
use ren_recorder::time::UnixMillis;
use ren_supervisor::{RunSnapshot, RunState};

const SIZE: Size = Size {
    width: 90,
    height: 24,
};

fn idle_run() -> RunSnapshot {
    RunSnapshot {
        run: RunId::restore(uuid::Uuid::from_u128(1)),
        harness: Harness::Codex,
        workdir: "/dev/acme".into(),
        state: RunState::Idle {
            thread: NativeThreadId::new(uuid::Uuid::from_u128(0x77)).unwrap(),
        },
    }
}

fn typed(tui: &mut Tui, text: &str) -> Vec<Out> {
    text.chars()
        .flat_map(|ch| tui.press(Key::Char(ch)))
        .collect()
}

#[test]
fn starting_reads_what_every_screen_shows() {
    let outs = Tui::new(SIZE, Opening::Wall).start();
    for read in [Read::Snapshot, Read::Runs, Read::Recorded, Read::Projects] {
        assert!(outs.contains(&Out::Read(read)), "{outs:?}");
    }
}

#[test]
fn keys_go_to_the_screen_on_top() {
    // On the wall, q quits; in the console, it is typed.
    let mut tui = Tui::new(SIZE, Opening::Wall);
    assert_eq!(tui.press(Key::Char('q')), vec![Out::Quit]);
    let mut tui = Tui::new(SIZE, Opening::Runs);
    assert_eq!(tui.press(Key::Char('q')), Vec::new());
    assert!(tui.wall_frame(UnixMillis::from_millis(0)).is_none());
}

#[test]
fn r_opens_the_console_over_the_wall_and_slash_wall_goes_back() {
    let mut tui = Tui::new(SIZE, Opening::Wall);
    let _ = tui.apply(Input::Runs(Ok(vec![idle_run()])));
    let _ = tui.press(Key::Char('r'));
    assert!(tui.wall_frame(UnixMillis::from_millis(0)).is_none());
    let _ = typed(&mut tui, "/wall");
    let outs = tui.press(Key::Enter);
    assert!(tui.wall_frame(UnixMillis::from_millis(0)).is_some());
    assert!(outs.contains(&Out::Read(Read::Snapshot)), "{outs:?}");
    // Back on the wall, keys are the wall's again.
    assert_eq!(tui.press(Key::Char('q')), vec![Out::Quit]);
}

#[test]
fn slash_wall_from_the_console_opens_a_wall_over_it() {
    let mut tui = Tui::new(SIZE, Opening::Runs);
    let _ = typed(&mut tui, "/wall");
    let _ = tui.press(Key::Enter);
    assert!(tui.wall_frame(UnixMillis::from_millis(0)).is_some());
}

#[test]
fn the_help_lists_the_keys_of_the_screen_under_it_and_closes() {
    let mut tui = Tui::new(SIZE, Opening::Wall);
    let _ = tui.press(Key::Char('?'));
    assert!(tui.help_shown());
    // It takes no other key.
    assert_eq!(tui.press(Key::Char('p')), Vec::new());
    assert_eq!(tui.press(Key::Char('t')), vec![Out::CycleTheme]);
    let _ = tui.press(Key::Esc);
    assert!(!tui.help_shown());

    let mut tui = Tui::new(SIZE, Opening::Runs);
    let _ = typed(&mut tui, "/help");
    let _ = tui.press(Key::Enter);
    assert!(tui.help_shown());
    let _ = tui.press(Key::Char('?'));
    assert!(!tui.help_shown());
}

#[test]
fn runs_reach_both_the_wall_and_the_console() {
    let mut tui = Tui::new(SIZE, Opening::Wall);
    let _ = tui.apply(Input::Runs(Ok(vec![idle_run()])));
    let frame = tui.wall_frame(UnixMillis::from_millis(0)).unwrap();
    assert_eq!(frame.status.runs.map(|runs| runs.live), Some(1));
    let _ = tui.press(Key::Char('r'));
    let _ = tui.press(Key::AltRight);
    assert_eq!(tui.shown_run(), Some(idle_run().run));
}
