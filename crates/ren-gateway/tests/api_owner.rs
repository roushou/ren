//! The gateway alone declares the API's wire types: ts-rs, which writes them
//! to the UI, is a dependency of ren-gateway and of no other crate. A type
//! elsewhere that derived `TS` would make its crate part of the API without
//! anything at the gateway saying so.

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
