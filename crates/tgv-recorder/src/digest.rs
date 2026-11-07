//! What a call's text becomes when it is stored: excerpts cut to a budget,
//! by field, after its secrets are redacted.

use serde::Serialize;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use tgv_protocol::activity::ToolInput;

/// Which part of a call a document holds.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum SearchField {
    /// The user's own words, without the context clients add.
    Prompt,
    /// The model's text output.
    Reply,
    /// Tool calls the model requested: name, targets, and input.
    ToolInput,
    /// Tool results the client sent back.
    ToolOutput,
}

/// Text cut to a budget, saying how much was left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Excerpt {
    pub text: String,
    pub omitted_chars: u64,
}

impl Excerpt {
    /// `text`, cut to `max_chars` characters.
    pub fn cut(text: &str, max_chars: usize) -> Self {
        let total = text.chars().count();
        Self {
            text: text.chars().take(max_chars).collect(),
            omitted_chars: u64::try_from(total.saturating_sub(max_chars)).unwrap_or(u64::MAX),
        }
    }

    pub fn of_input(input: &ToolInput, max_chars: usize) -> Self {
        match input {
            ToolInput::Json(value) => Self::cut(&value.to_string(), max_chars),
            ToolInput::Text(text) => Self::cut(text, max_chars),
        }
    }
}
