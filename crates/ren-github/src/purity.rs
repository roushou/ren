//! The query, what an answer says, and the observation stay free of
//! runtime and I/O, so a caller can use them with any HTTP stack; only the
//! client and the poller send and wait.

#[test]
fn sources_stay_free_of_runtime_and_io() {
    let sources = [
        ("check.rs", include_str!("check.rs")),
        ("pull.rs", include_str!("pull.rs")),
        ("repository.rs", include_str!("repository.rs")),
        ("watch.rs", include_str!("watch.rs")),
        ("observation.rs", include_str!("observation.rs")),
        ("oid.rs", include_str!("oid.rs")),
        ("reported.rs", include_str!("reported.rs")),
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
