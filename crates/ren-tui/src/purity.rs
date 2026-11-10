//! The view's core stays sans-IO: no runtime, no I/O, no terminal. It
//! depends on `ren-store` for its views, and that crate depends on Tokio
//! and SQLx, so this is checked on the sources rather than by the
//! manifest.

#[test]
fn the_core_stays_free_of_runtime_and_io_dependencies() {
    let sources = [
        ("lib.rs", include_str!("lib.rs")),
        ("call.rs", include_str!("call.rs")),
        ("grid.rs", include_str!("grid.rs")),
        ("keys.rs", include_str!("keys.rs")),
        ("session.rs", include_str!("session.rs")),
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
