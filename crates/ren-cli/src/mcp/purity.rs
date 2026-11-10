//! The MCP core stays pure: no runtime, I/O, or dependency on the command
//! shell. The crate itself depends on Tokio and Reqwest, so this is checked
//! on the sources rather than by the manifest.

#[test]
fn the_mcp_core_stays_free_of_runtime_and_io_dependencies() {
    let sources = [
        ("mod.rs", include_str!("mod.rs")),
        ("tools.rs", include_str!("tools.rs")),
    ];
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
        "crate::management",
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
