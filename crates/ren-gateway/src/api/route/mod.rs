//! Where each history read lives in the HTTP API: its path and query string.
//! The server parses a request's URI into a [`ReadRequest`] with
//! [`ApiRead::parse`]; a client builds the URI with [`ApiRead::uri`]. A
//! round-trip test keeps the two directions one table.

use super::ApiError;
use super::params::{RawCallQuery, RawExposureQuery, RawPageQuery, RawSearchQuery, RawUsageQuery};
use axum::extract::Query;
use http::Uri;
use ren_protocol::label::Labeled;
use ren_store::query::{CallQuery, QueryError, TimeRange};
use ren_store::read::ReadRequest;
use serde::de::DeserializeOwned;
use std::fmt::Display;

/// A history read, as the HTTP API addresses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiRead(pub ReadRequest);

impl ApiRead {
    /// The routes reads are served on, in the router's path syntax.
    pub(crate) const ROUTES: [&str; 13] = [
        "/api/calls",
        "/api/calls/{id}",
        "/api/calls/{id}/activity",
        "/api/sessions",
        "/api/sessions/{id}/calls",
        "/api/sessions/{id}/turns",
        "/api/sessions/{id}/report",
        "/api/turns/{id}",
        "/api/usage",
        "/api/search",
        "/api/exposures",
        "/api/runs",
        "/api/runs/{id}",
    ];

    /// The path and query string that ask for this read.
    pub fn uri(&self) -> String {
        let (path, query) = match &self.0 {
            ReadRequest::Calls(query) => ("/api/calls".to_owned(), Self::call_params(query)),
            ReadRequest::Call(call) => (format!("/api/calls/{call}"), QueryParams::default()),
            ReadRequest::CallActivity(call) => (
                format!("/api/calls/{call}/activity"),
                QueryParams::default(),
            ),
            ReadRequest::Sessions(limit) => (
                "/api/sessions".to_owned(),
                QueryParams::default().with("limit", limit.get()),
            ),
            ReadRequest::SessionCalls {
                session,
                limit,
                after,
            } => (
                format!("/api/sessions/{session}/calls"),
                QueryParams::default()
                    .with("limit", limit.get())
                    .opt("after", after.as_ref()),
            ),
            ReadRequest::Turns { session, limit } => (
                format!("/api/sessions/{session}/turns"),
                QueryParams::default().with("limit", limit.get()),
            ),
            ReadRequest::Report(session) => (
                format!("/api/sessions/{session}/report"),
                QueryParams::default(),
            ),
            ReadRequest::Turn(turn) => (format!("/api/turns/{turn}"), QueryParams::default()),
            ReadRequest::Usage(range) => (
                "/api/usage".to_owned(),
                QueryParams::default().range(*range),
            ),
            ReadRequest::Search(query) => (
                "/api/search".to_owned(),
                QueryParams::default()
                    .with("q", &query.terms)
                    .with("limit", query.limit.get())
                    .opt("session", query.session)
                    .opt("client", query.client.map(|client| client.label()))
                    .opt("field", query.field.map(|field| field.label()))
                    .range(query.time),
            ),
            ReadRequest::Exposures(query) => (
                "/api/exposures".to_owned(),
                QueryParams::default()
                    .with("limit", query.limit.get())
                    .opt("session", query.session)
                    .range(query.time),
            ),
            ReadRequest::Runs(limit) => (
                "/api/runs".to_owned(),
                QueryParams::default().with("limit", limit.get()),
            ),
            ReadRequest::Run(run) => (format!("/api/runs/{run}"), QueryParams::default()),
        };
        match serde_urlencoded::to_string(&query.0) {
            Ok(query) if !query.is_empty() => format!("{path}?{query}"),
            Ok(_) | Err(_) => path,
        }
    }

    /// The read a request's URI asks for. A path no read lives at, or an
    /// identifier that cannot be one, is not found.
    pub(super) fn parse(uri: &Uri) -> Result<Self, ApiError> {
        let segments: Vec<&str> = uri
            .path()
            .strip_prefix("/api/")
            .unwrap_or_default()
            .split('/')
            .collect();
        let request = match segments.as_slice() {
            ["calls"] => {
                ReadRequest::Calls(CallQuery::try_from(Self::query::<RawCallQuery>(uri)?)?)
            }
            ["calls", call] => ReadRequest::Call(Self::id(call)?),
            ["calls", call, "activity"] => ReadRequest::CallActivity(Self::id(call)?),
            ["sessions"] => {
                let page = Self::query::<RawPageQuery>(uri)?;
                if page.after.is_some() {
                    return Err(QueryError::Cursor.into());
                }
                ReadRequest::Sessions(page.limit()?)
            }
            ["sessions", session, "calls"] => {
                let page = Self::query::<RawPageQuery>(uri)?;
                ReadRequest::SessionCalls {
                    session: Self::id(session)?,
                    limit: page.limit()?,
                    after: page.after()?,
                }
            }
            ["sessions", session, "turns"] => {
                let page = Self::query::<RawPageQuery>(uri)?;
                if page.after.is_some() {
                    return Err(QueryError::Cursor.into());
                }
                ReadRequest::Turns {
                    session: Self::id(session)?,
                    limit: page.limit()?,
                }
            }
            ["sessions", session, "report"] => ReadRequest::Report(Self::id(session)?),
            ["turns", turn] => ReadRequest::Turn(Self::id(turn)?),
            ["usage"] => {
                ReadRequest::Usage(TimeRange::try_from(Self::query::<RawUsageQuery>(uri)?)?)
            }
            ["search"] => ReadRequest::Search(Self::query::<RawSearchQuery>(uri)?.parse()?),
            ["exposures"] => ReadRequest::Exposures(Self::query::<RawExposureQuery>(uri)?.parse()?),
            ["runs"] => {
                let page = Self::query::<RawPageQuery>(uri)?;
                if page.after.is_some() {
                    return Err(QueryError::Cursor.into());
                }
                ReadRequest::Runs(page.limit()?)
            }
            ["runs", run] => ReadRequest::Run(Self::id(run)?),
            _ => return Err(ApiError::NotFound),
        };
        Ok(Self(request))
    }

    fn id<T: std::str::FromStr>(text: &str) -> Result<T, ApiError> {
        text.parse().map_err(|_| ApiError::NotFound)
    }

    fn query<T: DeserializeOwned>(uri: &Uri) -> Result<T, ApiError> {
        Query::<T>::try_from_uri(uri)
            .map(|query| query.0)
            .map_err(|_| ApiError::Query(QueryError::Filter))
    }

    /// The parameters `/api/calls` parses back into `query`.
    fn call_params(query: &CallQuery) -> QueryParams {
        let filter = &query.filter;
        QueryParams::default()
            .with("limit", query.limit.get())
            .opt("after", query.after.as_ref())
            .opt("session", filter.session.as_ref())
            .opt("client", filter.client.map(|client| client.label()))
            .opt("user", filter.user.as_ref())
            .opt("kind", filter.kind.map(|kind| kind.label()))
            .opt("provider", filter.provider.map(|provider| provider.label()))
            .opt("model", filter.model.as_ref().map(|value| value.as_str()))
            .opt("tool", filter.tool.as_ref().map(|value| value.as_str()))
            .opt("q", filter.text.as_ref().map(|value| value.as_str()))
            .opt("status", filter.status.map(|status| status.label()))
            .opt("has_tools", filter.tools.map(|tools| tools.label()))
            .range(filter.time)
    }
}

/// Query parameters in order; absent values are left out.
#[derive(Default)]
struct QueryParams(Vec<(&'static str, String)>);

impl QueryParams {
    fn with(mut self, name: &'static str, value: impl Display) -> Self {
        self.0.push((name, value.to_string()));
        self
    }

    fn opt(self, name: &'static str, value: Option<impl Display>) -> Self {
        match value {
            Some(value) => self.with(name, value),
            None => self,
        }
    }

    /// `from_ms` and `until_ms`, for the ends `range` has.
    fn range(self, range: TimeRange) -> Self {
        self.opt("from_ms", range.from().map(|time| time.as_millis()))
            .opt("until_ms", range.until().map(|time| time.as_millis()))
    }
}

#[cfg(test)]
mod tests;
