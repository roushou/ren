//! The gateway's state machines and decisions (`lifecycle`: `attempt`,
//! `call`, `channel`, `retry`; `operator`, `user`, and `preview`) stay pure: no runtime,
//! I/O, or dependency on the gateway's shell modules. The crate itself depends on Tokio and Axum, so this is checked on
//! the sources rather than by the manifest.

#[test]
fn pure_modules_stay_free_of_runtime_and_io_dependencies() {
    let sources = [
        ("lifecycle/attempt.rs", include_str!("lifecycle/attempt.rs")),
        ("lifecycle/call.rs", include_str!("lifecycle/call.rs")),
        ("lifecycle/channel.rs", include_str!("lifecycle/channel.rs")),
        ("operator.rs", include_str!("operator.rs")),
        ("lifecycle/retry.rs", include_str!("lifecycle/retry.rs")),
        ("user.rs", include_str!("user.rs")),
        ("preview/mod.rs", include_str!("preview/mod.rs")),
        ("preview/previews.rs", include_str!("preview/previews.rs")),
        ("preview/requests.rs", include_str!("preview/requests.rs")),
        ("preview/slot.rs", include_str!("preview/slot.rs")),
    ];
    let forbidden = [
        "tokio",
        "axum",
        "tower",
        "hyper",
        "reqwest",
        "sqlx",
        "ren_store",
        "clap",
        "tracing",
        "std::fs",
        "std::net",
        "std::env",
        "std::process",
        "std::thread",
        "crate::api",
        "crate::control",
        "crate::forward",
        "crate::policy",
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
