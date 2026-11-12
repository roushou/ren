//! The routes together: what each input reaches, and which route keys go to.

use crate::{Frame, Input, Key, Out, Route, Size, Tui, console, wall};
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

#[test]
fn starting_asks_for_both_routes_reads() {
    let outs = Tui::new(SIZE, Route::Wall).start();
    assert!(outs.contains(&Out::Wall(wall::Read::Snapshot)));
    assert!(outs.contains(&Out::Console(console::Read::Runs)));
    assert!(outs.contains(&Out::Console(console::Read::Projects)));
}

#[test]
fn keys_go_to_the_route_shown() {
    // On the wall, q quits; in the console, it is typed.
    let mut tui = Tui::new(SIZE, Route::Wall);
    assert_eq!(tui.press(Key::Char('q')), vec![Out::Quit]);
    let mut tui = Tui::new(SIZE, Route::Console);
    assert_eq!(tui.press(Key::Char('q')), Vec::new());
    assert_eq!(tui.route(), Route::Console);
}

#[test]
fn r_on_the_wall_opens_the_console_and_slash_wall_comes_back() {
    let mut tui = Tui::new(SIZE, Route::Wall);
    let _ = tui.apply(Input::Runs(Ok(vec![idle_run()])));
    let _ = tui.press(Key::Char('r'));
    assert_eq!(tui.route(), Route::Console);
    assert!(matches!(
        tui.frame(UnixMillis::from_millis(0)),
        Frame::Console(_)
    ));
    for ch in "/wall".chars() {
        let _ = tui.press(Key::Char(ch));
    }
    let outs = tui.press(Key::Enter);
    assert_eq!(tui.route(), Route::Wall);
    assert_eq!(outs, vec![Out::Wall(wall::Read::Snapshot)]);
    // Back on the wall, keys are the wall's again.
    assert_eq!(tui.press(Key::Char('q')), vec![Out::Quit]);
}

#[test]
fn runs_reach_both_routes() {
    let mut tui = Tui::new(SIZE, Route::Console);
    let _ = tui.apply(Input::Runs(Ok(vec![idle_run()])));
    let Frame::Console(console) = tui.frame(UnixMillis::from_millis(0)) else {
        panic!("the console is shown");
    };
    assert!(format!("{console:?}").contains("acme"));
    let mut tui = Tui::new(SIZE, Route::Wall);
    let _ = tui.apply(Input::Runs(Ok(vec![idle_run()])));
    let Frame::Wall(wall) = tui.frame(UnixMillis::from_millis(0)) else {
        panic!("the wall is shown");
    };
    assert_eq!(wall.status.runs.map(|runs| runs.live), Some(1));
}
