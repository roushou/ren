//! Every vocabulary's labels, pinned: a known value serializes as its label,
//! so renaming a variant must never change one silently.
#![allow(
    clippy::unwrap_used,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use ren_github::{CheckState, ChecksState, MergeState, Readiness, Reported, ReviewDecision};
use strum::VariantArray;

/// `T`'s labels, in declaration order, after checking that each serializes
/// as its label, alone and as a reported value.
fn labels<T>() -> Vec<&'static str>
where
    T: VariantArray + Copy + Into<&'static str> + serde::Serialize + std::fmt::Display,
{
    T::VARIANTS
        .iter()
        .map(|&variant| {
            let label: &'static str = variant.into();
            assert_eq!(variant.to_string(), label);
            assert_eq!(
                serde_json::to_string(&variant).unwrap(),
                format!("\"{label}\"")
            );
            let reported = serde_json::to_string(&Reported::Known(variant)).unwrap();
            assert_eq!(reported, format!("\"{label}\""));
            label
        })
        .collect()
}

#[test]
fn every_label_is_pinned() {
    let table: String = [
        ("github::CheckState", labels::<CheckState>()),
        ("github::ChecksState", labels::<ChecksState>()),
        ("github::Readiness", labels::<Readiness>()),
        ("github::MergeState", labels::<MergeState>()),
        ("github::ReviewDecision", labels::<ReviewDecision>()),
    ]
    .iter()
    .map(|(name, labels)| format!("{name}: {}\n", labels.join(" ")))
    .collect();
    let pinned = include_str!("fixtures/labels.txt");
    assert!(
        table == pinned,
        "labels changed; the current table is:\n{table}"
    );
}
