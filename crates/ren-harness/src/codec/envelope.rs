//! One line from a driven client, read as JSON.

use crate::{
    drive::{DriveEvent, RawLine, Received, Unreadable},
    lines::Framed,
};
use ren_protocol::label::{Labeled, Stored};
use serde_json::Value;

/// A line that is JSON, kept as it arrived.
#[derive(Debug)]
pub(crate) struct Envelope {
    pub(crate) line: RawLine,
    pub(crate) body: Value,
}

impl Envelope {
    /// `frame` read as JSON; otherwise what it produced: nothing for an
    /// empty line, an unreadable line for one too long or not JSON.
    pub(crate) fn open(frame: Framed) -> Result<Self, Received> {
        let bytes = match frame {
            Framed::Line(bytes) => bytes,
            Framed::TooLong { bytes } => {
                return Err(Received::event(DriveEvent::Unreadable {
                    line: RawLine::new(Vec::new()),
                    reason: Unreadable::TooLong { bytes },
                }));
            }
        };
        if bytes.is_empty() {
            return Err(Received::default());
        }
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(body) => Ok(Self {
                line: RawLine::new(bytes),
                body,
            }),
            Err(_) => Err(Received::event(DriveEvent::Unreadable {
                line: RawLine::new(bytes),
                reason: Unreadable::NotJson,
            })),
        }
    }

    /// The message's `type`, as `T` names it, unknown ones kept; none when
    /// it has no type.
    pub(crate) fn typed<T: Labeled>(&self) -> Option<Stored<T>> {
        self.body
            .get("type")
            .and_then(Value::as_str)
            .map(Stored::from_label)
    }

    /// The line as one without the type every message has.
    pub(crate) fn untyped(self) -> DriveEvent {
        DriveEvent::Unreadable {
            line: self.line,
            reason: Unreadable::Untyped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Envelope;
    use crate::{
        drive::{DriveEvent, RawLine, Unreadable},
        lines::Framed,
        settings::Midway,
    };
    use ren_protocol::label::Stored;

    fn open(text: &str) -> Result<Envelope, Vec<DriveEvent>> {
        Envelope::open(Framed::Line(text.as_bytes().to_vec())).map_err(|received| received.events)
    }

    #[test]
    fn a_line_is_json_or_says_why_not() {
        assert_eq!(open("").unwrap_err(), Vec::new());
        assert_eq!(
            open("not json").unwrap_err(),
            vec![DriveEvent::Unreadable {
                line: RawLine::new(b"not json".to_vec()),
                reason: Unreadable::NotJson
            }]
        );
        assert_eq!(
            Envelope::open(Framed::TooLong { bytes: 9 })
                .unwrap_err()
                .events,
            vec![DriveEvent::Unreadable {
                line: RawLine::new(Vec::new()),
                reason: Unreadable::TooLong { bytes: 9 }
            }]
        );
        let typed = open(r#"{"type":"steer"}"#).unwrap();
        assert_eq!(typed.typed::<Midway>(), Some(Stored::Known(Midway::Steer)));
        let unknown = open(r#"{"type":"later"}"#).unwrap();
        assert_eq!(unknown.typed::<Midway>().unwrap().label(), "later");
        let untyped = open(r#"{"type":1}"#).unwrap();
        assert_eq!(untyped.typed::<Midway>(), None);
        assert!(matches!(
            untyped.untyped(),
            DriveEvent::Unreadable {
                reason: Unreadable::Untyped,
                ..
            }
        ));
    }
}
