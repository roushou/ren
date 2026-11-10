//! Bounded native text taken from the wire.

use crate::label::Labeled;
use std::fmt;

/// Native text kept in inspection reports: model names, stop reasons, tool
/// names, provider identifiers, and unknown kinds. At most
/// [`WireText::MAX_BYTES`] bytes, with no control characters, so it is safe to
/// store and display.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WireText(Box<str>);

/// Why wire text was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WireTextError {
    #[error("wire text is longer than the inspection limit")]
    TooLong,
    #[error("wire text contains control characters")]
    ControlCharacter,
    #[error("wire identifier is empty")]
    Empty,
}

impl WireText {
    pub const MAX_BYTES: usize = 256;

    /// Accept text that may be empty, such as a model-supplied name.
    pub fn new(value: &str) -> Result<Self, WireTextError> {
        if value.len() > Self::MAX_BYTES {
            Err(WireTextError::TooLong)
        } else if value.chars().any(char::is_control) {
            Err(WireTextError::ControlCharacter)
        } else {
            Ok(Self(value.into()))
        }
    }

    /// Accept an identifier: like [`WireText::new`], and never empty, so
    /// distinct calls can't collapse onto one blank identity.
    pub fn identifier(value: &str) -> Result<Self, WireTextError> {
        if value.is_empty() {
            Err(WireTextError::Empty)
        } else {
            Self::new(value)
        }
    }

    /// The label of a closed vocabulary. Labels are short, static, printable
    /// ASCII written in this crate, so no check is needed.
    pub fn from_label<T: Labeled>(value: T) -> Self {
        Self(value.label().into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WireText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{WireText, WireTextError};

    #[test]
    fn bounds_text_by_bytes_and_rejects_control_characters() {
        assert!(WireText::new(&"é".repeat(WireText::MAX_BYTES / 2)).is_ok());
        assert_eq!(
            WireText::new(&"é".repeat(WireText::MAX_BYTES / 2 + 1)),
            Err(WireTextError::TooLong)
        );
        assert_eq!(
            WireText::new("bad\nlabel"),
            Err(WireTextError::ControlCharacter)
        );
        assert_eq!(
            WireText::new("").map(|text| text.to_string()),
            Ok(String::new())
        );
        assert_eq!(WireText::identifier(""), Err(WireTextError::Empty));
    }
}
