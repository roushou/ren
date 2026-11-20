//! A value GitHub reported: one this crate knows, or GitHub's own text.

use serde::{Serialize, Serializer};
use std::fmt;

/// A value of a vocabulary GitHub reported: a known one, or the text GitHub
/// sent for one this crate does not know, kept rather than dropped or
/// rounded to a known one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Reported<T> {
    Known(T),
    Unknown(String),
}

impl<T: Copy> Reported<T> {
    /// The known value, if it is one.
    pub fn known(&self) -> Option<T> {
        match self {
            Self::Known(value) => Some(*value),
            Self::Unknown(_) => None,
        }
    }
}

impl<T: Copy + Into<&'static str>> Reported<T> {
    /// Its label: a known value's own (`snake_case`), or GitHub's text.
    pub fn label(&self) -> &str {
        match self {
            Self::Known(value) => (*value).into(),
            Self::Unknown(text) => text,
        }
    }
}

impl<T: Copy + Into<&'static str>> fmt::Display for Reported<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl<T: Copy + Into<&'static str>> Serialize for Reported<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.label())
    }
}

impl<T> From<T> for Reported<T> {
    fn from(value: T) -> Self {
        Self::Known(value)
    }
}
