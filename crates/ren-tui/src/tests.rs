//! The app as a whole: what it reads first, the list's selection across
//! runs and watched sessions, the help over it, and drawing at any size and
//! in any theme.

use crate::screen::main::Item;
use crate::{Input, Key, Out, Read, Size, Snapshot, Theme, ThemeName, Tui};
use ratatui::{buffer::Buffer, layout::Rect, style::Color};
use ren_harness::Harness;
use ren_protocol::native::NativeThreadId;
use ren_recorder::id::RunId;
use ren_recorder::time::UnixMillis;
use ren_store::view::{CallPage, SessionView};
use ren_supervisor::{RunSnapshot, RunState};
use serde_json::json;

const SIZE: Size = Size {
    width: 100,
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

/// A snapshot with one session seen just now, from a client Ren did not
/// start.
fn watched() -> Input {
    let session: SessionView = serde_json::from_value(json!({
        "id": "00000000-0000-0000-0000-0000000000a1", "provider": "anthropic",
        "source": "claude_code_headers", "native_id": "native-a1",
        "first_seen": 1_000, "last_seen": 1_000, "call_count": 1, "turn_count": 0,
        "error_count": 0,
        "tokens": { "input": null, "output": null, "cache_read": null }
    }))
    .unwrap();
    Input::Snapshot(Snapshot {
        read_at: UnixMillis::from_millis(1_000),
        sessions: vec![session],
        open: CallPage {
            calls: Vec::new(),
            next: None,
        },
        recent: CallPage {
            calls: Vec::new(),
            next: None,
        },
    })
}

fn drawn(tui: &Tui, size: Size, theme: &Theme) -> Buffer {
    let area = Rect::new(0, 0, size.width, size.height);
    let mut buffer = Buffer::empty(area);
    tui.view(UnixMillis::from_millis(1_000))
        .render(theme, area, &mut buffer);
    buffer
}

#[test]
fn starting_reads_what_the_app_shows() {
    let outs = Tui::new(SIZE).start();
    for read in [Read::Snapshot, Read::Runs, Read::Recorded, Read::Projects] {
        assert!(outs.contains(&Out::Read(read)), "{outs:?}");
    }
}

#[test]
fn runs_come_first_and_the_selection_follows_the_top_until_chosen() {
    let mut tui = Tui::new(SIZE);
    let _ = tui.apply(watched());
    assert!(matches!(tui.selected(), Some(Item::Watched(_))));
    // Until it is chosen, the selection follows the top: the run.
    let _ = tui.apply(Input::Runs(Ok(vec![idle_run()])));
    assert_eq!(tui.shown_run(), Some(idle_run().run));
    let _ = tui.press(Key::Char('j'));
    assert!(matches!(tui.selected(), Some(Item::Watched(_))));
    // Chosen, it stays.
    let _ = tui.apply(Input::Runs(Ok(vec![idle_run()])));
    assert!(matches!(tui.selected(), Some(Item::Watched(_))));
    // A watched session takes no writing.
    let _ = tui.press(Key::Char('i'));
    assert_eq!(tui.mode(), crate::screen::main::Mode::Normal);
}

#[test]
fn the_help_lists_the_keys_and_closes() {
    let mut tui = Tui::new(SIZE);
    let _ = tui.press(Key::Char('?'));
    assert!(tui.overlaid());
    assert_eq!(
        tui.press(Key::Char('j')),
        Vec::new(),
        "it takes no other key"
    );
    assert_eq!(tui.press(Key::Char('t')), vec![Out::CycleTheme]);
    let _ = tui.press(Key::Esc);
    assert!(!tui.overlaid());
    assert_eq!(tui.press(Key::Char('q')), vec![Out::Quit]);
}

#[test]
fn ctrl_c_quits_only_when_pressed_twice_in_a_row() {
    let mut tui = Tui::new(SIZE);
    let _ = tui.apply(watched());
    // Once: a word in the key bar, and nothing else.
    assert_eq!(tui.press(Key::Interrupt), Vec::new());
    let mut text = String::new();
    let buffer = drawn(&tui, SIZE, &Theme::default());
    for y in 0..SIZE.height {
        for x in 0..SIZE.width {
            text.push_str(buffer[(x, y)].symbol());
        }
    }
    assert!(text.contains("press ctrl-c again to quit"), "{text}");
    // Another key in between starts over.
    let _ = tui.press(Key::Char('j'));
    assert_eq!(tui.press(Key::Interrupt), Vec::new());
    assert_eq!(tui.press(Key::Interrupt), vec![Out::Quit]);
}

#[test]
fn tiny_terminals_draw_without_panicking() {
    for (width, height) in [(0, 0), (1, 1), (2, 1), (10, 3), (20, 2), (57, 11), (40, 30)] {
        let size = Size { width, height };
        let mut tui = Tui::new(size);
        let _ = tui.apply(watched());
        let _ = tui.apply(Input::Runs(Ok(vec![idle_run()])));
        for key in [Key::Tab, Key::Char('n'), Key::Esc, Key::Char('?')] {
            let _ = tui.press(key);
            let _ = drawn(&tui, size, &Theme::default());
        }
    }
}

#[test]
fn every_theme_draws_the_same_content_and_terminal_colors_stay_symbolic() {
    let mut tui = Tui::new(SIZE);
    let _ = tui.apply(watched());
    let _ = tui.apply(Input::Runs(Ok(vec![idle_run()])));
    let symbols = |buffer: &Buffer| -> Vec<String> {
        buffer
            .content
            .iter()
            .map(|cell| cell.symbol().to_owned())
            .collect()
    };
    let plain = drawn(&tui, SIZE, &ThemeName::Terminal.theme());
    for cell in &plain.content {
        assert_eq!(cell.bg, Color::Reset);
        assert!(!matches!(cell.fg, Color::Rgb(..) | Color::Indexed(_)));
    }
    for name in [ThemeName::Dark, ThemeName::Light] {
        assert_eq!(
            symbols(&drawn(&tui, SIZE, &name.theme())),
            symbols(&plain),
            "{name}"
        );
    }
    let mono = drawn(&tui, SIZE, &Theme::monochrome());
    assert!(
        mono.content
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
}
