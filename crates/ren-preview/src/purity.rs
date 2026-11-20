//! The crate's sources stay free of runtime and I/O; the manifest keeps its
//! dependencies pure, and this keeps its own code so.

#[test]
fn sources_stay_free_of_runtime_and_io() {
    let sources = [
        ("lib.rs", include_str!("lib.rs")),
        ("previews.rs", include_str!("previews.rs")),
        ("slot.rs", include_str!("slot.rs")),
    ];
    let forbidden = [
        "tokio",
        "reqwest",
        "sqlx",
        "clap",
        "tracing",
        "std::fs",
        "std::net",
        "std::env",
        "std::process",
        "std::thread",
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
