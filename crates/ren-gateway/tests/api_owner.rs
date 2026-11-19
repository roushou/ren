//! The gateway alone declares the API's wire types: ts-rs, which writes them
//! to the UI, is a dependency of ren-gateway and of no other crate. A type
//! elsewhere that derived `TS` would make its crate part of the API without
//! anything at the gateway saying so.
//!
//! The gateway also pins which wire types it reads from requests
//! (`fixtures/requests.txt`): `cargo xtask compat` compares a request type
//! by what older clients still send, and every other type by what they
//! still read.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use std::path::Path;

#[test]
fn only_the_gateway_depends_on_ts_rs() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut owners = Vec::new();
    for entry in std::fs::read_dir(crates).unwrap() {
        let manifest = entry.unwrap().path().join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        if text
            .lines()
            .any(|line| line.trim_start().starts_with("ts-rs"))
        {
            owners.push(manifest.parent().unwrap().file_name().unwrap().to_owned());
        }
    }
    owners.sort();
    assert_eq!(owners, ["ren-gateway"], "crates that depend on ts-rs");
}

/// The wire types the gateway reads from a request: each route's body
/// (`Json<T>`) and query string (`Query<T>`, `query::<T>`), found in the
/// gateway's sources and pinned, so a new one cannot be compared as a
/// response.
#[test]
fn every_request_type_is_pinned() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let wire = root.join("../../ui/apps/web/src/lib/wire");
    let mut sources = vec![root.join("src")];
    let mut found = std::collections::BTreeSet::new();
    while let Some(path) = sources.pop() {
        if path.is_dir() {
            sources.extend(
                std::fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for marker in ["Json<", "Query<", "query::<", "Query::<"] {
            for (start, _) in text.match_indices(marker) {
                let rest = &text[start + marker.len()..];
                let end = rest
                    .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                let (name, after) = rest.split_at(end);
                // A plain name a wire type is written for: not a path, a
                // generic parameter, or a type the UI is not given.
                if after.starts_with('>') && wire.join(format!("{name}.ts")).is_file() {
                    found.insert(name.to_owned());
                }
            }
        }
    }
    let listed: String = found.iter().map(|name| format!("{name}\n")).collect();
    assert_eq!(
        listed,
        include_str!("fixtures/requests.txt"),
        "the gateway's request types, for `cargo xtask compat`"
    );
}
