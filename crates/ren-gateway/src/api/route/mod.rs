//! Where each history read lives in the HTTP API: its path and query string.
//! The server parses a request's URI into a [`ReadRequest`] with
//! [`ApiRead::parse`]; a client builds the URI with [`ApiRead::uri`]. A
//! round-trip test keeps the two directions one table.

use super::ApiError;
use super::params::{
    RawCallQuery, RawExposureQuery, RawShelfQuery, RawPageQuery, RawSearchQuery, RawUsageQuery,
};
use axum::extract::Query;
use http::Uri;
use ren_protocol::label::Labeled;
use ren_store::query::{CallQuery, QueryError, Shelf, TimeRange};
use ren_store::read::ReadRequest;
use ren_store::search::{ExposureQuery, SearchQuery};
use serde::de::DeserializeOwned;
use std::fmt::Display;

/// A history read, as the HTTP API addresses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiRead(pub ReadRequest);

impl ApiRead {
    /// The routes reads are served on, in the router's path syntax.
    pub(crate) const ROUTES: [&str; 16] = [
        "/api/calls",
        "/api/calls/{id}",
        "/api/calls/{id}/activity",
        "/api/recordings",
        "/api/recordings/{id}/calls",
        "/api/recordings/{id}/turns",
        "/api/recordings/{id}/report",
        "/api/turns/{id}",
        "/api/usage",
        "/api/search",
        "/api/exposures",
        "/api/runs",
        "/api/runs/{id}",
        "/api/runs/{id}/conversation",
        "/api/work",
        "/api/work/{id}",
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
            ReadRequest::Recordings {
                limit,
                shelf,
                project,
            } => (
                "/api/recordings".to_owned(),
                QueryParams::default()
                    .with("limit", limit.get())
                    .shelf(*shelf)
                    .opt("project", project.as_ref()),
            ),
            ReadRequest::RecordingCalls {
                recording,
                limit,
                after,
            } => (
                format!("/api/recordings/{recording}/calls"),
                QueryParams::default()
                    .with("limit", limit.get())
                    .opt("after", after.as_ref()),
            ),
            ReadRequest::Turns { recording, limit } => (
                format!("/api/recordings/{recording}/turns"),
                QueryParams::default().with("limit", limit.get()),
            ),
            ReadRequest::Report(recording) => (
                format!("/api/recordings/{recording}/report"),
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
                    .opt("recording", query.recording)
                    .opt("client", query.client.map(|client| client.label()))
                    .opt("field", query.field.map(|field| field.label()))
                    .range(query.time),
            ),
            ReadRequest::Exposures(query) => (
                "/api/exposures".to_owned(),
                QueryParams::default()
                    .with("limit", query.limit.get())
                    .opt("recording", query.recording)
                    .range(query.time),
            ),
            ReadRequest::Runs {
                limit,
                shelf,
                project,
            } => (
                "/api/runs".to_owned(),
                QueryParams::default()
                    .with("limit", limit.get())
                    .shelf(*shelf)
                    .opt("project", project.as_ref()),
            ),
            ReadRequest::Run(run) => (format!("/api/runs/{run}"), QueryParams::default()),
            ReadRequest::Conversation(run) => (
                format!("/api/runs/{run}/conversation"),
                QueryParams::default(),
            ),
            ReadRequest::WorkItems {
                limit,
                shelf,
                project,
            } => (
                "/api/work".to_owned(),
                QueryParams::default()
                    .with("limit", limit.get())
                    .shelf(*shelf)
                    .opt("project", project.as_ref()),
            ),
            ReadRequest::WorkItem(item) => (format!("/api/work/{item}"), QueryParams::default()),
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
            ["recordings"] => {
                let (limit, shelf, project) = Self::query::<RawShelfQuery>(uri)?.parse()?;
                ReadRequest::Recordings {
                    limit,
                    shelf,
                    project,
                }
            }
            ["recordings", recording, "calls"] => {
                let page = Self::query::<RawPageQuery>(uri)?;
                ReadRequest::RecordingCalls {
                    recording: Self::id(recording)?,
                    limit: page.limit()?,
                    after: page.after()?,
                }
            }
            ["recordings", recording, "turns"] => {
                let page = Self::query::<RawPageQuery>(uri)?;
                if page.after.is_some() {
                    return Err(QueryError::Cursor.into());
                }
                ReadRequest::Turns {
                    recording: Self::id(recording)?,
                    limit: page.limit()?,
                }
            }
            ["recordings", recording, "report"] => ReadRequest::Report(Self::id(recording)?),
            ["turns", turn] => ReadRequest::Turn(Self::id(turn)?),
            ["usage"] => {
                ReadRequest::Usage(TimeRange::try_from(Self::query::<RawUsageQuery>(uri)?)?)
            }
            ["search"] => {
                ReadRequest::Search(SearchQuery::try_from(Self::query::<RawSearchQuery>(uri)?)?)
            }
            ["exposures"] => {
                let raw = Self::query::<RawExposureQuery>(uri)?;
                ReadRequest::Exposures(ExposureQuery::try_from(raw)?)
            }
            ["runs"] => {
                let (limit, shelf, project) = Self::query::<RawShelfQuery>(uri)?.parse()?;
                ReadRequest::Runs {
                    limit,
                    shelf,
                    project,
                }
            }
            ["runs", run] => ReadRequest::Run(Self::id(run)?),
            ["runs", run, "conversation"] => ReadRequest::Conversation(Self::id(run)?),
            ["work"] => {
                let (limit, shelf, project) = Self::query::<RawShelfQuery>(uri)?.parse()?;
                ReadRequest::WorkItems {
                    limit,
                    shelf,
                    project,
                }
            }
            ["work", item] => ReadRequest::WorkItem(Self::id(item)?),
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
            .opt("recording", filter.recording.as_ref())
            .opt("project", filter.project.as_ref())
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
    /// The shelf, left out when it is the current one.
    fn shelf(self, shelf: Shelf) -> Self {
        match shelf {
            Shelf::Current => self,
            Shelf::Archived => self.with("shelf", shelf),
        }
    }

    fn range(self, range: TimeRange) -> Self {
        self.opt("from_ms", range.from().map(|time| time.as_millis()))
            .opt("until_ms", range.until().map(|time| time.as_millis()))
    }
}

#[cfg(test)]
mod tests;
