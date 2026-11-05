//! Closed vocabularies and the labels they are stored and sent as.
//!
//! A vocabulary is an enum with these attributes, in this order (rustfmt would
//! merge adjacent derives into one long list):
//!
//! ```ignore
//! #[derive(Display, EnumString, IntoStaticStr, VariantArray)]
//! #[strum(serialize_all = "snake_case")]
//! #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
//! ```
//!
//! Its labels are written once, in the strum attributes, and `Display`,
//! `FromStr`, and serde all read them from there; an irregular label is a
//! `#[strum(to_string = "…")]` on its variant. A label used twice makes the
//! generated parser's second arm an unreachable pattern, which the `-D warnings`
//! checks reject. `tests/labels.rs` pins every label, because renaming a variant
//! would otherwise change stored data. [`Stored`] keeps labels this binary does
//! not know, as required for data written by other versions or providers.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{borrow::Cow, fmt, str::FromStr};
use strum::VariantArray;

/// A closed vocabulary whose variants each have one stable label. Implemented
/// for every enum with the derives listed in the module documentation.
pub trait Labeled: Copy + Eq + 'static + FromStr + VariantArray + Into<&'static str> {
    /// Every variant, in declaration order.
    const ALL: &'static [Self] = Self::VARIANTS;

    fn label(self) -> &'static str {
        self.into()
    }

    /// The variant with exactly this label.
    fn from_label(label: &str) -> Option<Self> {
        label.parse().ok()
    }
}

impl<T> Labeled for T where T: Copy + Eq + 'static + FromStr + VariantArray + Into<&'static str> {}

/// A label as text, kept verbatim when it is not part of a known vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RawLabel(Box<str>);

impl RawLabel {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A label read from storage or the wire: either a variant this binary knows,
/// or the original text, preserved rather than discarded.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Stored<T> {
    Known(T),
    Unknown(RawLabel),
}

impl<T: Labeled> Stored<T> {
    pub fn label(&self) -> &str {
        match self {
            Self::Known(value) => value.label(),
            Self::Unknown(raw) => raw.as_str(),
        }
    }

    /// The label `label` names, keeping unknown text rather than dropping it.
    pub fn from_label(label: &str) -> Self {
        T::from_label(label).map_or_else(|| Self::Unknown(RawLabel(label.into())), Self::Known)
    }

    pub fn known(&self) -> Option<T> {
        match self {
            Self::Known(value) => Some(*value),
            Self::Unknown(_) => None,
        }
    }
}

impl<T: Labeled> From<T> for Stored<T> {
    fn from(value: T) -> Self {
        Self::Known(value)
    }
}

impl<T: Labeled> fmt::Display for Stored<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl<T: Labeled> Serialize for Stored<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.label())
    }
}

impl<'de, T: Labeled> Deserialize<'de> for Stored<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let label = <Cow<'de, str>>::deserialize(deserializer)?;
        Ok(Self::from_label(label.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use super::{Labeled, Stored};
    use serde_with::{DeserializeFromStr, SerializeDisplay};
    use strum::{Display, EnumString, IntoStaticStr, VariantArray};

    /// A sample vocabulary.
    #[derive(Display, EnumString, IntoStaticStr, VariantArray)]
    #[strum(serialize_all = "snake_case")]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
    enum Fixture {
        First,
        SecondValue,
        #[strum(to_string = "third-label")]
        Irregular,
    }

    #[test]
    fn labels_round_trip_through_every_representation() {
        for variant in Fixture::ALL.iter().copied() {
            assert_eq!(variant.to_string(), variant.label());
            assert_eq!(variant.label().parse::<Fixture>(), Ok(variant));
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!("\"{}\"", variant.label()));
            assert_eq!(serde_json::from_str::<Fixture>(&json).unwrap(), variant);
        }
        let labels: Vec<_> = Fixture::ALL.iter().map(|variant| variant.label()).collect();
        assert_eq!(labels, ["first", "second_value", "third-label"]);
    }

    #[test]
    fn closed_parsing_rejects_unknown_labels_without_echoing_them() {
        let error = "SECRET".parse::<Fixture>().unwrap_err();
        assert_eq!(error, strum::ParseError::VariantNotFound);
        assert!(!error.to_string().contains("SECRET"));
        assert!(serde_json::from_str::<Fixture>("\"First\"").is_err());
        // An explicit label replaces the derived one; it is not an alias.
        assert!("irregular".parse::<Fixture>().is_err());
    }

    #[test]
    fn stored_labels_keep_unknown_text() {
        let known = Stored::<Fixture>::from_label("first");
        assert_eq!(known.known(), Some(Fixture::First));
        let unknown: Stored<Fixture> = serde_json::from_str("\"fourth\"").unwrap();
        assert_eq!(unknown.known(), None);
        assert_eq!(unknown.label(), "fourth");
        assert_eq!(serde_json::to_string(&unknown).unwrap(), "\"fourth\"");
    }
}
