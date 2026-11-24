//! A run's sandbox and listing are declared, not found: what they hold is
//! handed to them, so they stay free of the runtime and of I/O. The crate itself runs
//! processes and serves sockets, so this is checked on the source.

#[test]
fn the_sandbox_stays_free_of_runtime_and_io() {
    let sources = [
        ("sandbox.rs", include_str!("sandbox.rs")),
        ("listing.rs", include_str!("listing.rs")),
    ];
    let forbidden = [
        "tokio",
        "tracing",
        "std::fs",
        "std::net",
        "std::env",
        "std::process",
        "std::thread",
        "std::io",
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
            // A path is looked at on the host by its finder, never here.
            for call in [".exists()", ".is_file()", ".is_dir()", ".metadata("] {
                assert!(!line.contains(call), "{file} reads the host: {line}");
            }
        }
    }
}
