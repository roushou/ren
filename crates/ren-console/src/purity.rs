//! The console's core stays sans-IO: no runtime, no I/O, no terminal. Its
//! view types come from crates that depend on Tokio and SQLx, so this is
//! checked on the sources rather than by the manifest. `draw` alone uses
//! ratatui, to fill a buffer.

#[test]
fn the_core_stays_free_of_runtime_and_io_dependencies() {
    let sources = [
        ("lib.rs", include_str!("lib.rs")),
        ("composer.rs", include_str!("composer.rs")),
        ("desk.rs", include_str!("desk.rs")),
        ("slash.rs", include_str!("slash.rs")),
        ("transcript.rs", include_str!("transcript.rs")),
        ("view.rs", include_str!("view.rs")),
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
