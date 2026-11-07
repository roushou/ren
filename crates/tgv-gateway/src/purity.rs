//! The gateway's state machines and decisions (`attempt`, `call`, `channel`,
//! `retry`, `operator`) stay pure: no runtime, I/O, or dependency on the gateway's shell
//! modules. The crate itself depends on Tokio and Axum, so this is checked on
//! the sources rather than by the manifest.

#[test]
fn pure_modules_stay_free_of_runtime_and_io_dependencies() {
    let sources = [
        ("attempt.rs", include_str!("attempt.rs")),
        ("call.rs", include_str!("call.rs")),
        ("channel.rs", include_str!("channel.rs")),
        ("operator.rs", include_str!("operator.rs")),
        ("retry.rs", include_str!("retry.rs")),
    ];
    let forbidden = [
        "tokio",
        "axum",
        "tower",
        "hyper",
        "reqwest",
        "sqlx",
        "tgv_store",
        "clap",
        "tracing",
        "std::fs",
        "std::net",
        "std::env",
        "std::process",
        "crate::api",
        "crate::buffer",
        "crate::control",
        "crate::headers",
        "crate::ingress",
        "crate::policy",
        "crate::relay",
        "crate::route",
        "crate::upstream",
        "crate::websocket",
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
