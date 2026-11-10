//! The recorder's pure parts (its core, the digest and its replay, capture
//! options, and progress updates) stay free of the runtime, I/O, and the shell modules
//! around them. The crate itself depends on Tokio for its queue and task, so
//! this is checked on the sources rather than by the manifest.

#[test]
fn pure_modules_stay_free_of_runtime_and_io_dependencies() {
    let sources = [
        ("recording/mod.rs", include_str!("recording/mod.rs")),
        ("digest/mod.rs", include_str!("digest/mod.rs")),
        ("digest/replay.rs", include_str!("digest/replay.rs")),
        ("options.rs", include_str!("options.rs")),
        ("progress.rs", include_str!("progress.rs")),
    ];
    let forbidden = [
        "tokio",
        "tracing",
        "std::fs",
        "std::net",
        "std::env",
        "std::process",
        "std::thread",
        "crate::item",
        "crate::sink",
        "crate::tap",
        "crate::task",
        "crate::metrics",
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
