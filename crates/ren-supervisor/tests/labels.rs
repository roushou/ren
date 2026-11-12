//! The supervisor's vocabularies read the same everywhere: one label table.
#![allow(
    clippy::unwrap_used,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly"
)]

use ren_protocol::label::Labeled;
use ren_supervisor::{ClientStatus, Delivery, PathKind, PlaceKind};
use serde::{Serialize, de::DeserializeOwned};
use std::fmt::{Debug, Display};
use std::str::FromStr;

/// `name:` and its labels, after checking every representation agrees.
fn row<T>(name: &str) -> String
where
    T: Labeled + Debug + Display + FromStr + Serialize + DeserializeOwned,
    <T as FromStr>::Err: Debug,
{
    let mut row = format!("{name}:");
    for variant in T::ALL {
        let label = variant.label();
        assert_eq!(variant.to_string(), label);
        assert_eq!(label.parse::<T>().unwrap(), *variant);
        let json = serde_json::to_string(variant).unwrap();
        assert_eq!(json, format!("\"{label}\""));
        assert_eq!(serde_json::from_str::<T>(&json).unwrap(), *variant);
        row.push(' ');
        row.push_str(label);
    }
    row.push('\n');
    row
}

#[test]
fn labels_are_pinned_and_round_trip() {
    let table = row::<ClientStatus>("ClientStatus")
        + &row::<PlaceKind>("PlaceKind")
        + &row::<Delivery>("Delivery")
        + &row::<PathKind>("PathKind");
    assert_eq!(table, include_str!("fixtures/labels.txt"));
}
