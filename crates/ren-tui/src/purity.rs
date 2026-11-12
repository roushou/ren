//! Everything but the renderer (`ui/draw.rs`), the theme, and the terminal
//! stays sans-IO: no runtime, no I/O, no terminal. The crate depends on
//! `ren-store` for its views, and that crate depends on Tokio and SQLx, so
//! this is checked on the sources rather than by the manifest.

#[test]
fn the_core_stays_free_of_runtime_and_io_dependencies() {
    let sources = [
        ("conversation/mod.rs", include_str!("conversation/mod.rs")),
        ("conversation/read.rs", include_str!("conversation/read.rs")),
        ("event.rs", include_str!("event.rs")),
        ("keys.rs", include_str!("keys.rs")),
        ("lib.rs", include_str!("lib.rs")),
        ("model/call.rs", include_str!("model/call.rs")),
        ("model/desk.rs", include_str!("model/desk.rs")),
        ("model/mod.rs", include_str!("model/mod.rs")),
        ("model/sessions.rs", include_str!("model/sessions.rs")),
        ("model/transcript.rs", include_str!("model/transcript.rs")),
        ("screen/composer.rs", include_str!("screen/composer.rs")),
        ("screen/help.rs", include_str!("screen/help.rs")),
        ("screen/message.rs", include_str!("screen/message.rs")),
        ("screen/mod.rs", include_str!("screen/mod.rs")),
        ("screen/resume.rs", include_str!("screen/resume.rs")),
        ("screen/session.rs", include_str!("screen/session.rs")),
        ("screen/frame.rs", include_str!("screen/frame.rs")),
        ("screen/slash.rs", include_str!("screen/slash.rs")),
        ("screen/wall/grid.rs", include_str!("screen/wall/grid.rs")),
        ("screen/wall/keys.rs", include_str!("screen/wall/keys.rs")),
        ("screen/wall/mod.rs", include_str!("screen/wall/mod.rs")),
        ("screen/wall/pane.rs", include_str!("screen/wall/pane.rs")),
        ("screen/wall/runs.rs", include_str!("screen/wall/runs.rs")),
        ("screen/wall/view.rs", include_str!("screen/wall/view.rs")),
        ("ui/mod.rs", include_str!("ui/mod.rs")),
        ("ui/text.rs", include_str!("ui/text.rs")),
    ];
    let forbidden = [
        "tokio",
        "sqlx",
        "reqwest",
        "tracing",
        "ratatui",
        "crossterm",
        "std::fs",
        "std::net",
        "std::env",
        "std::io",
        "std::process",
        "std::thread",
        "std::time::SystemTime",
        "std::time::Instant",
    ];
    for (file, source) in sources {
        for line in source.lines().map(str::trim_start) {
            if line.starts_with("//") {
                continue;
            }
            for dependency in forbidden {
                assert!(
                    !line.contains(&format!("{dependency}::"))
                        && !line.contains(&format!("use {dependency}")),
                    "{file} depends on {dependency}: {line}"
                );
            }
        }
    }
}
