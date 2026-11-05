//! Reading native JSON with an explicit difference between "absent" and
//! "malformed".
//!
//! Native payloads are open: unknown fields are ignored here and stay in raw
//! capture. A field that is present but has the wrong shape is reported as
//! [`Field::Malformed`] rather than silently treated as absent, and it never
//! hides valid sibling fields.

use super::{inspect::Observation, text::WireText};
use crate::label::{Labeled, Stored};
use serde_json::{Map, Value};

/// Why a payload could not be read as a native JSON object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("payload is not JSON")]
    NotJson,
    #[error("payload is not a JSON object")]
    NotObject,
    #[error("payload has no usable type discriminator")]
    NoKind,
}

/// One optional field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field<T> {
    /// Missing, or JSON `null`.
    Absent,
    Present(T),
    /// Present with the wrong type or an unacceptable value.
    Malformed,
}

impl<T> Field<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Field<U> {
        match self {
            Self::Absent => Field::Absent,
            Self::Present(value) => Field::Present(f(value)),
            Self::Malformed => Field::Malformed,
        }
    }

    pub fn as_ref(&self) -> Field<&T> {
        match self {
            Self::Absent => Field::Absent,
            Self::Present(value) => Field::Present(value),
            Self::Malformed => Field::Malformed,
        }
    }

    pub fn present(self) -> Option<T> {
        match self {
            Self::Present(value) => Some(value),
            Self::Absent | Self::Malformed => None,
        }
    }

    pub fn is_malformed(&self) -> bool {
        matches!(self, Self::Malformed)
    }

    /// Hand a present value to `observe`, and report a malformed one.
    pub fn observe_with<O: FnMut(Observation)>(self, out: &mut O, observe: impl FnOnce(T, &mut O)) {
        match self {
            Self::Present(value) => observe(value, out),
            Self::Malformed => out(Observation::MalformedField),
            Self::Absent => {}
        }
    }
}

/// A parsed JSON payload. Duplicate keys keep their last value.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonDocument(Value);

impl JsonDocument {
    pub fn parse(bytes: &[u8]) -> Result<Self, DecodeError> {
        serde_json::from_slice(bytes)
            .map(Self)
            .map_err(|_| DecodeError::NotJson)
    }

    pub fn object(&self) -> Result<JsonObject<'_>, DecodeError> {
        match &self.0 {
            Value::Object(map) => Ok(JsonObject(map)),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Array(_) => Err(DecodeError::NotObject),
        }
    }
}

/// A borrowed view of one JSON object.
#[derive(Debug, Clone, Copy)]
pub struct JsonObject<'a>(&'a Map<String, Value>);

impl<'a> JsonObject<'a> {
    fn value(&self, key: &str) -> Field<&'a Value> {
        match self.0.get(key) {
            None | Some(Value::Null) => Field::Absent,
            Some(value) => Field::Present(value),
        }
    }

    fn string(&self, key: &str) -> Field<&'a str> {
        match self.value(key) {
            Field::Present(Value::String(text)) => Field::Present(text),
            Field::Present(_) | Field::Malformed => Field::Malformed,
            Field::Absent => Field::Absent,
        }
    }

    /// Bounded display text, possibly empty.
    pub fn text(&self, key: &str) -> Field<WireText> {
        match self.string(key) {
            Field::Present(text) => WireText::new(text).map_or(Field::Malformed, Field::Present),
            Field::Absent => Field::Absent,
            Field::Malformed => Field::Malformed,
        }
    }

    /// A bounded, non-empty identifier.
    pub fn identifier(&self, key: &str) -> Field<WireText> {
        match self.string(key) {
            Field::Present(text) => {
                WireText::identifier(text).map_or(Field::Malformed, Field::Present)
            }
            Field::Absent => Field::Absent,
            Field::Malformed => Field::Malformed,
        }
    }

    /// A non-negative integer count, such as a token total.
    pub fn count(&self, key: &str) -> Field<u64> {
        match self.value(key) {
            Field::Present(Value::Number(number)) => {
                number.as_u64().map_or(Field::Malformed, Field::Present)
            }
            Field::Present(_) | Field::Malformed => Field::Malformed,
            Field::Absent => Field::Absent,
        }
    }

    pub fn object(&self, key: &str) -> Field<JsonObject<'a>> {
        match self.value(key) {
            Field::Present(Value::Object(map)) => Field::Present(JsonObject(map)),
            Field::Present(_) | Field::Malformed => Field::Malformed,
            Field::Absent => Field::Absent,
        }
    }

    /// An array whose elements should all be objects. Each element is checked
    /// on its own, so one malformed element does not hide the others.
    pub fn objects(&self, key: &str) -> Field<Vec<Field<JsonObject<'a>>>> {
        match self.value(key) {
            Field::Present(Value::Array(items)) => Field::Present(
                items
                    .iter()
                    .map(|item| match item {
                        Value::Object(map) => Field::Present(JsonObject(map)),
                        Value::Null
                        | Value::Bool(_)
                        | Value::Number(_)
                        | Value::String(_)
                        | Value::Array(_) => Field::Malformed,
                    })
                    .collect(),
            ),
            Field::Present(_) | Field::Malformed => Field::Malformed,
            Field::Absent => Field::Absent,
        }
    }

    /// A string discriminator from a closed vocabulary, keeping unknown text.
    /// Non-string values are malformed, so a map or array can never pose as a
    /// known kind.
    pub fn label<T: Labeled>(&self, key: &str) -> Field<Kind<T>> {
        match self.string(key) {
            Field::Present(text) => Kind::parse(text).map_or(Field::Malformed, Field::Present),
            Field::Absent => Field::Absent,
            Field::Malformed => Field::Malformed,
        }
    }
}

/// A discriminator read from the wire: a known variant, or bounded unknown
/// text kept for the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind<T> {
    Known(T),
    Unknown(WireText),
}

impl<T: Labeled> Kind<T> {
    /// `None` when unknown text is not acceptable wire text.
    pub fn parse(text: &str) -> Option<Self> {
        match Stored::<T>::from_label(text) {
            Stored::Known(value) => Some(Self::Known(value)),
            Stored::Unknown(raw) => WireText::identifier(raw.as_str()).ok().map(Self::Unknown),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DecodeError, Field, JsonDocument, Kind};
    use crate::direction::Direction;

    #[test]
    fn fields_distinguish_absent_null_and_malformed() {
        let document = JsonDocument::parse(
            br#"{"name":"ok","null":null,"number":3,"negative":-1,"text":[],"id":"","kind":"request","odd":"x\ny","list":[{},1]}"#,
        )
        .unwrap();
        let object = document.object().unwrap();
        assert_eq!(
            object.text("name").map(|t| t.to_string()),
            Field::Present("ok".into())
        );
        assert_eq!(object.text("null"), Field::Absent);
        assert_eq!(object.text("missing"), Field::Absent);
        assert_eq!(object.text("text"), Field::Malformed);
        assert_eq!(object.text("odd"), Field::Malformed);
        assert_eq!(object.identifier("id"), Field::Malformed);
        assert_eq!(object.count("number"), Field::Present(3));
        assert_eq!(object.count("negative"), Field::Malformed);
        assert_eq!(
            object.label::<Direction>("kind"),
            Field::Present(Kind::Known(Direction::Request))
        );
        assert!(matches!(
            object.label::<Direction>("name"),
            Field::Present(Kind::Unknown(text)) if text.as_str() == "ok"
        ));
        assert_eq!(object.label::<Direction>("number"), Field::Malformed);
        let list = object.objects("list").present().unwrap();
        assert!(matches!(list[0], Field::Present(_)));
        assert!(list[1].is_malformed());
    }

    #[test]
    fn documents_must_be_json_objects() {
        assert_eq!(
            JsonDocument::parse(b"not json").unwrap_err(),
            DecodeError::NotJson
        );
        for scalar in [&b"42"[..], b"[]", b"\"text\"", b"null"] {
            assert_eq!(
                JsonDocument::parse(scalar).unwrap().object().unwrap_err(),
                DecodeError::NotObject
            );
        }
        let duplicate = JsonDocument::parse(br#"{"a":"first","a":"last"}"#).unwrap();
        assert_eq!(
            duplicate.object().unwrap().text("a").map(|t| t.to_string()),
            Field::Present("last".into())
        );
    }
}
