//! The codecs (each client's structured mode and the parts they share), the
//! driving vocabulary, line framing, and run settings stay free of I/O and
//! of the process and filesystem work the rest of the crate does (launching,
//! the Pi extension cache, the host environment). The crate has no I/O
//! manifest of its own to forbid that, so it is checked on the sources.

#[test]
fn codecs_stay_free_of_io() {
    let sources = [
        ("codec/mod.rs", include_str!("codec/mod.rs")),
        ("codec/approvals.rs", include_str!("codec/approvals.rs")),
        ("codec/envelope.rs", include_str!("codec/envelope.rs")),
        ("codec/ledger.rs", include_str!("codec/ledger.rs")),
        ("codec/rpc.rs", include_str!("codec/rpc.rs")),
        ("codec/turn.rs", include_str!("codec/turn.rs")),
        ("codex/app_server.rs", include_str!("codex/app_server.rs")),
        (
            "claude_code/stream_json.rs",
            include_str!("claude_code/stream_json.rs"),
        ),
        ("pi/rpc.rs", include_str!("pi/rpc.rs")),
        ("opencode/acp.rs", include_str!("opencode/acp.rs")),
        ("opencode/session.rs", include_str!("opencode/session.rs")),
        ("drive.rs", include_str!("drive.rs")),
        ("lines.rs", include_str!("lines.rs")),
        ("settings.rs", include_str!("settings.rs")),
    ];
    let forbidden = [
        "std::fs",
        "std::io",
        "std::net",
        "std::env",
        "std::process",
        "std::thread",
        "tempfile",
        "crate::host",
        "crate::launch",
        "crate::pi::cache",
        "PiCache",
        "HostEnv",
    ];
    for (file, source) in sources {
        for line in source.lines().map(str::trim_start) {
            if line.starts_with("//") {
                continue;
            }
            for dependency in forbidden {
                assert!(
                    !line.contains(&format!("{dependency}::"))
                        && !line.contains(&format!("use {dependency}"))
                        && !(dependency.chars().next().is_some_and(char::is_uppercase)
                            && line.contains(dependency)),
                    "{file} depends on {dependency}: {line}"
                );
            }
        }
    }
}
