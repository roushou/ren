//! The invented history stays pure: no runtime, I/O, or dependency on the
//! command shell. The crate itself depends on Tokio, so this is checked on
//! the sources rather than by the manifest.

#[test]
fn the_invented_history_stays_free_of_runtime_and_io_dependencies() {
    let forbidden = [
        "tokio",
        "reqwest",
        "clap",
        "tracing",
        "anyhow",
        "std::fs",
        "std::io",
        "std::net",
        "std::env",
        "std::process",
        "crate::commands",
        "crate::context",
        "ren_store",
        "ren_daemon",
    ];
    for line in include_str!("mod.rs").lines().map(str::trim_start) {
        if line.starts_with("//") {
            continue;
        }
        for dependency in forbidden {
            assert!(
                !line.contains(&format!("{dependency}::"))
                    && !line.contains(&format!("use {dependency}")),
                "mod.rs depends on {dependency}: {line}"
            );
        }
    }
}
