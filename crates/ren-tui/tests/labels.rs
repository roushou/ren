//! Saved theme names are stable and share one label table.

use ren_protocol::label::Labeled;
use ren_tui::ThemeName;

#[test]
fn theme_labels_are_pinned_and_round_trip() {
    let mut table = String::from("ThemeName:");
    for name in ThemeName::ALL {
        let label = name.label();
        assert_eq!(name.to_string(), label);
        assert_eq!(label.parse::<ThemeName>().unwrap(), *name);
        let json = serde_json::to_string(name).unwrap();
        assert_eq!(json, format!("\"{label}\""));
        assert_eq!(serde_json::from_str::<ThemeName>(&json).unwrap(), *name);
        table.push(' ');
        table.push_str(label);
    }
    table.push('\n');
    assert_eq!(table, include_str!("fixtures/labels.txt"));
}
