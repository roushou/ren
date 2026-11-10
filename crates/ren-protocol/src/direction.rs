//! Which side of an exchange bytes or messages travel on.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// Request flows from the client toward the provider; response flows back.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Direction {
    Request,
    Response,
}

#[cfg(test)]
mod tests {
    use super::Direction;
    use crate::label::Labeled;

    #[test]
    fn labels_match_the_existing_storage_vocabulary() {
        assert_eq!(Direction::Request.label(), "request");
        assert_eq!(Direction::Response.label(), "response");
    }
}
