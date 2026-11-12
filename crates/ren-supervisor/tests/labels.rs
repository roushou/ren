//! A client's status reads the same everywhere: one label table.

use ren_protocol::label::Labeled;
use ren_supervisor::ClientStatus;

#[test]
fn client_status_labels_are_pinned_and_round_trip() {
    let mut table = String::from("ClientStatus:");
    for status in ClientStatus::ALL {
        let label = status.label();
        assert_eq!(status.to_string(), label);
        assert_eq!(label.parse::<ClientStatus>().unwrap(), *status);
        let json = serde_json::to_string(status).unwrap();
        assert_eq!(json, format!("\"{label}\""));
        assert_eq!(
            serde_json::from_str::<ClientStatus>(&json).unwrap(),
            *status
        );
        table.push(' ');
        table.push_str(label);
    }
    table.push('\n');
    assert_eq!(table, include_str!("fixtures/labels.txt"));
}
