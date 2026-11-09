//! Query strings as sent, and their parsing into typed store queries.

use serde::Deserialize;
use std::str::FromStr;
use tgv_recorder::time::UnixMillis;
use tgv_store::query::{
    CallFilter, CallQuery, Cursor, PageLimit, QueryError, SearchText, TimeRange,
};
use tgv_store::search::{ExposureQuery, SearchQuery, SearchTerms};

/// A label filter as sent: blank means absent, and anything else must be
/// one of the filter's labels.
#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
pub(super) struct RawLabel(Option<String>);

impl RawLabel {
    fn parse<T: FromStr>(self) -> Result<Option<T>, QueryError> {
        self.0
            .filter(|value| !value.trim().is_empty())
            .map(|value| T::from_str(&value).map_err(|_| QueryError::Filter))
            .transpose()
    }
}

/// `GET /api/calls` parameters.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct RawCallQuery {
    limit: Option<u32>,
    after: Option<String>,
    session: RawLabel,
    client: RawLabel,
    user: RawLabel,
    kind: RawLabel,
    provider: RawLabel,
    model: Option<String>,
    tool: Option<String>,
    q: Option<String>,
    status: RawLabel,
    has_tools: RawLabel,
    from_ms: Option<u64>,
    until_ms: Option<u64>,
}

/// `limit` and `after` alone.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawPageQuery {
    pub limit: Option<u32>,
    pub after: Option<String>,
}

/// `GET /api/usage` parameters: an inclusive range of call start times.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawUsageQuery {
    from_ms: Option<u64>,
    until_ms: Option<u64>,
}

impl TryFrom<RawUsageQuery> for TimeRange {
    type Error = QueryError;

    fn try_from(raw: RawUsageQuery) -> Result<Self, Self::Error> {
        TimeRange::new(
            raw.from_ms.map(UnixMillis::from_millis),
            raw.until_ms.map(UnixMillis::from_millis),
        )
    }
}

/// `GET /api/search` parameters.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct RawSearchQuery {
    q: Option<String>,
    session: RawLabel,
    client: RawLabel,
    field: RawLabel,
    from_ms: Option<u64>,
    until_ms: Option<u64>,
    limit: Option<u32>,
}

impl RawSearchQuery {
    pub fn parse(self) -> Result<SearchQuery, super::ApiError> {
        Ok(SearchQuery {
            terms: SearchTerms::parse(self.q.as_deref().unwrap_or_default())?,
            session: self.session.parse()?,
            client: self.client.parse()?,
            field: self.field.parse()?,
            time: TimeRange::new(
                self.from_ms.map(UnixMillis::from_millis),
                self.until_ms.map(UnixMillis::from_millis),
            )?,
            limit: self.limit.map_or(Ok(PageLimit::DEFAULT), PageLimit::new)?,
        })
    }
}

/// `GET /api/exposures` parameters.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct RawExposureQuery {
    session: RawLabel,
    from_ms: Option<u64>,
    until_ms: Option<u64>,
    limit: Option<u32>,
}

impl RawExposureQuery {
    pub fn parse(self) -> Result<ExposureQuery, QueryError> {
        Ok(ExposureQuery {
            session: self.session.parse()?,
            time: TimeRange::new(
                self.from_ms.map(UnixMillis::from_millis),
                self.until_ms.map(UnixMillis::from_millis),
            )?,
            limit: self.limit.map_or(Ok(PageLimit::DEFAULT), PageLimit::new)?,
        })
    }
}

/// `GET /api/live` parameters.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawLiveQuery {
    pub after: Option<u64>,
    /// Also send progress events on streaming calls.
    pub progress: Option<bool>,
}

impl RawPageQuery {
    pub fn limit(&self) -> Result<PageLimit, QueryError> {
        self.limit.map_or(Ok(PageLimit::DEFAULT), PageLimit::new)
    }

    pub fn after(&self) -> Result<Option<Cursor>, QueryError> {
        self.after.as_deref().map(Cursor::from_str).transpose()
    }
}

impl TryFrom<RawCallQuery> for CallQuery {
    type Error = QueryError;

    fn try_from(raw: RawCallQuery) -> Result<Self, Self::Error> {
        let text = |value: Option<String>| value.as_deref().map(SearchText::new).transpose();
        Ok(Self {
            limit: raw.limit.map_or(Ok(PageLimit::DEFAULT), PageLimit::new)?,
            after: raw.after.as_deref().map(Cursor::from_str).transpose()?,
            filter: CallFilter {
                session: raw.session.parse()?,
                client: raw.client.parse()?,
                user: raw.user.parse()?,
                kind: raw.kind.parse()?,
                provider: raw.provider.parse()?,
                model: text(raw.model)?,
                tool: text(raw.tool)?,
                text: text(raw.q)?,
                status: raw.status.parse()?,
                tools: raw.has_tools.parse()?,
                time: TimeRange::new(
                    raw.from_ms.map(UnixMillis::from_millis),
                    raw.until_ms.map(UnixMillis::from_millis),
                )?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RawCallQuery;
    use tgv_protocol::route::Provider;
    use tgv_store::query::{CallQuery, QueryError, StatusFilter, ToolFilter};

    fn raw(query: &str) -> Option<RawCallQuery> {
        let uri: http::Uri = format!("/api/calls?{query}").parse().unwrap();
        axum::extract::Query::<RawCallQuery>::try_from_uri(&uri)
            .ok()
            .map(|query| query.0)
    }

    fn parse(query: &str) -> Result<CallQuery, QueryError> {
        CallQuery::try_from(raw(query).unwrap())
    }

    #[test]
    fn query_strings_parse_into_typed_filters_once() {
        let query = parse("provider=codex&status=errors&has_tools=true&limit=5&model=gpt").unwrap();
        assert_eq!(query.filter.provider, Some(Provider::Codex));
        assert_eq!(query.filter.status, Some(StatusFilter::Errors));
        assert_eq!(query.filter.tools, Some(ToolFilter::WithTools));
        assert_eq!(query.limit.get(), 5);
        assert_eq!(parse("provider=").unwrap().filter.provider, None);
        assert_eq!(
            parse("user=alice")
                .unwrap()
                .filter
                .user
                .map(|user| user.to_string()),
            Some("alice".to_owned())
        );
        for invalid in [
            "provider=nope",
            "user=Alice",
            "status=done",
            "has_tools=yes",
            "limit=0",
            "after=x",
            "from_ms=5&until_ms=1",
        ] {
            assert!(parse(invalid).is_err(), "{invalid}");
        }
        assert!(raw("extra=1").is_none());
    }
}
