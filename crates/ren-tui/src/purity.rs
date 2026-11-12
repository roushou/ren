//! The views' cores stay sans-IO: no runtime, no I/O, no terminal. It
//! depends on `ren-store` for its views, and that crate depends on Tokio
//! and SQLx, so this is checked on the sources rather than by the
//! manifest.

#[test]
fn the_core_stays_free_of_runtime_and_io_dependencies() {
    let sources = [
        ("lib.rs", include_str!("lib.rs")),
        ("event.rs", include_str!("event.rs")),
        ("wall/mod.rs", include_str!("wall/mod.rs")),
        ("wall/call.rs", include_str!("wall/call.rs")),
        ("wall/grid.rs", include_str!("wall/grid.rs")),
        ("wall/keys.rs", include_str!("wall/keys.rs")),
        ("wall/runs.rs", include_str!("wall/runs.rs")),
        ("wall/session.rs", include_str!("wall/session.rs")),
        ("console/mod.rs", include_str!("console/mod.rs")),
        ("console/composer.rs", include_str!("console/composer.rs")),
        ("console/desk.rs", include_str!("console/desk.rs")),
        ("console/slash.rs", include_str!("console/slash.rs")),
        (
            "console/transcript.rs",
            include_str!("console/transcript.rs"),
        ),
        ("console/view.rs", include_str!("console/view.rs")),
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
