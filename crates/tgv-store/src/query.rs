//! Typed read queries: page limits, cursors, search text, and call filters.
//! Raw query strings are parsed into these at the API and CLI boundaries.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt, str::FromStr};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use tgv_harness::Harness;
use tgv_protocol::route::Provider;
use tgv_recorder::{
    id::{CallId, SessionId},
    time::UnixMillis,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    #[error("limit must be between 1 and {}", PageLimit::MAX)]
    Limit,
    #[error("search text must be 1 to {} bytes", SearchText::MAX_BYTES)]
    SearchText,
    #[error("invalid page cursor")]
    Cursor,
    #[error("the time range ends before it starts")]
    TimeRange,
    #[error("unknown filter value")]
    Filter,
}

/// How many rows one page returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageLimit(u32);

impl PageLimit {
    pub const MAX: u32 = 1000;
    pub const DEFAULT: Self = Self(100);

    pub fn new(limit: u32) -> Result<Self, QueryError> {
        (1..=Self::MAX)
            .contains(&limit)
            .then_some(Self(limit))
            .ok_or(QueryError::Limit)
    }

    pub fn get(self) -> u32 {
        self.0
    }

    pub(super) fn rows(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// A position in a call listing: the last call of the previous page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub(super) started_at: UnixMillis,
    pub(super) call: CallId,
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.started_at.as_millis(), self.call)
    }
}

impl FromStr for Cursor {
    type Err = QueryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (time, call) = value.split_once(':').ok_or(QueryError::Cursor)?;
        Ok(Self {
            started_at: UnixMillis::from_millis(time.parse().map_err(|_| QueryError::Cursor)?),
            call: call.parse().map_err(|_| QueryError::Cursor)?,
        })
    }
}

impl serde::Serialize for Cursor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Case-insensitive substring search over metadata, never over payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchText(Box<str>);

impl SearchText {
    pub const MAX_BYTES: usize = 128;

    pub fn new(text: &str) -> Result<Self, QueryError> {
        let text = text.trim();
        (!text.is_empty() && text.len() <= Self::MAX_BYTES)
            .then(|| Self(text.into()))
            .ok_or(QueryError::SearchText)
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A coarse lifecycle filter.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum StatusFilter {
    /// Not yet finished.
    Active,
    Finished,
    /// An error status, a failed transport, or a failed model outcome.
    Errors,
    /// Cancelled, interrupted, or recovered after a restart.
    Interrupted,
}

/// Model calls, or the bookkeeping around them.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum KindFilter {
    /// Calls that ask a model for output: messages and responses, sent over
    /// HTTP or as a WebSocket message.
    Model,
    /// Everything else: token counts, model listings, and the WebSocket
    /// handshakes that open a channel.
    Other,
}

/// Whether the model requested tools.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ToolFilter {
    #[strum(to_string = "true")]
    WithTools,
    #[strum(to_string = "false")]
    WithoutTools,
}

/// An inclusive range of call start times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimeRange {
    from: Option<UnixMillis>,
    until: Option<UnixMillis>,
}

impl TimeRange {
    pub fn new(from: Option<UnixMillis>, until: Option<UnixMillis>) -> Result<Self, QueryError> {
        match (from, until) {
            (Some(from), Some(until)) if from > until => Err(QueryError::TimeRange),
            (Some(_) | None, Some(_) | None) => Ok(Self { from, until }),
        }
    }

    pub(super) fn from(self) -> Option<UnixMillis> {
        self.from
    }

    pub(super) fn until(self) -> Option<UnixMillis> {
        self.until
    }
}

/// Filters for a call listing. Every field narrows the result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallFilter {
    /// Calls TGV grouped into this session.
    pub session: Option<SessionId>,
    /// Calls attributed to this harness by its own headers or metadata.
    pub client: Option<Harness>,
    pub kind: Option<KindFilter>,
    pub provider: Option<Provider>,
    pub model: Option<SearchText>,
    pub tool: Option<SearchText>,
    /// Matches a call ID, model, or tool name.
    pub text: Option<SearchText>,
    pub status: Option<StatusFilter>,
    pub tools: Option<ToolFilter>,
    pub time: TimeRange,
}

/// One page of a call listing, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallQuery {
    pub filter: CallFilter,
    pub limit: PageLimit,
    pub after: Option<Cursor>,
}

impl Default for CallQuery {
    fn default() -> Self {
        Self {
            filter: CallFilter::default(),
            limit: PageLimit::DEFAULT,
            after: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cursor, PageLimit, QueryError, SearchText, TimeRange};
    use tgv_recorder::time::UnixMillis;

    #[test]
    fn query_values_are_parsed_once_with_bounds() {
        assert_eq!(PageLimit::new(0), Err(QueryError::Limit));
        assert_eq!(PageLimit::new(1001), Err(QueryError::Limit));
        assert!(PageLimit::new(1000).is_ok());
        assert_eq!(SearchText::new("  "), Err(QueryError::SearchText));
        assert_eq!(
            SearchText::new(&"x".repeat(129)),
            Err(QueryError::SearchText)
        );
        let cursor: Cursor = "12:00000000-0000-0000-0000-000000000001".parse().unwrap();
        assert_eq!(cursor.to_string().parse::<Cursor>(), Ok(cursor));
        for invalid in [
            "12",
            "x:00000000-0000-0000-0000-000000000001",
            "-1:x",
            "1:nope",
        ] {
            assert_eq!(invalid.parse::<Cursor>(), Err(QueryError::Cursor));
        }
        let (early, late) = (UnixMillis::from_millis(1), UnixMillis::from_millis(2));
        assert_eq!(
            TimeRange::new(Some(late), Some(early)),
            Err(QueryError::TimeRange)
        );
        assert!(TimeRange::new(Some(early), Some(late)).is_ok());
    }
}
