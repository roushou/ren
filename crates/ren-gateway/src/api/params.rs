//! Query strings as sent, and their parsing into typed store queries.

use ren_recorder::{project::ProjectName, time::UnixMillis};
use ren_store::query::{
    CallFilter, CallQuery, Cursor, ItemBodies, PageLimit, QueryError, SearchText, Shelf, TimeRange,
    UsageScope,
};
use ren_store::search::{ExposureQuery, SearchQuery, SearchTerms};
use serde::Deserialize;
use std::str::FromStr;
use ts_rs::TS;

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
#[derive(Debug, Default, Deserialize, TS)]
#[serde(deny_unknown_fields, default)]
#[ts(export, optional_fields)]
pub(super) struct RawCallQuery {
    limit: Option<u32>,
    after: Option<String>,
    #[ts(as = "Option<String>")]
    recording: RawLabel,
    /// A project's name: its sessions' runs' calls.
    project: Option<String>,
    #[ts(as = "Option<String>")]
    client: RawLabel,
    #[ts(as = "Option<String>")]
    user: RawLabel,
    #[ts(as = "Option<String>")]
    kind: RawLabel,
    #[ts(as = "Option<String>")]
    provider: RawLabel,
    model: Option<String>,
    tool: Option<String>,
    q: Option<String>,
    #[ts(as = "Option<String>")]
    status: RawLabel,
    #[ts(as = "Option<String>")]
    has_tools: RawLabel,
    from_ms: Option<u64>,
    until_ms: Option<u64>,
}

/// `limit` and `after` alone.
#[derive(Debug, Default, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, optional_fields)]
pub(super) struct RawPageQuery {
    pub limit: Option<u32>,
    pub after: Option<String>,
}

/// `GET /api/runs/{run}/conversation` parameters: whether each item keeps
/// its body (`kept`, the default) or leaves it out (`omitted`).
#[derive(Debug, Default, Deserialize, TS)]
#[serde(deny_unknown_fields, default)]
#[ts(export, optional_fields)]
pub(super) struct RawConversationQuery {
    #[ts(as = "Option<String>")]
    bodies: RawLabel,
}

impl RawConversationQuery {
    pub fn parse(self) -> Result<ItemBodies, QueryError> {
        Ok(self.bodies.parse()?.unwrap_or_default())
    }
}

/// `GET /api/recordings`, `/api/runs`, and `/api/work` parameters: a page
/// of one shelf, of every project or of the one named.
#[derive(Debug, Default, Deserialize, TS)]
#[serde(deny_unknown_fields, default)]
#[ts(export, optional_fields)]
pub(super) struct RawShelfQuery {
    limit: Option<u32>,
    #[ts(as = "Option<String>")]
    shelf: RawLabel,
    project: Option<String>,
}

impl RawShelfQuery {
    pub fn parse(self) -> Result<(PageLimit, Shelf, Option<ProjectName>), QueryError> {
        let limit = self.limit.map_or(Ok(PageLimit::DEFAULT), PageLimit::new)?;
        let shelf = self.shelf.parse()?.unwrap_or_default();
        let project = self
            .project
            .map(|name| name.parse().map_err(|_| QueryError::Filter))
            .transpose()?;
        Ok((limit, shelf, project))
    }
}

/// `GET /api/usage` parameters: an inclusive range of call start times,
/// of every call, of the project named, or of the session named.
#[derive(Debug, Default, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, optional_fields)]
pub(super) struct RawUsageQuery {
    from_ms: Option<u64>,
    until_ms: Option<u64>,
    /// A project's name: its sessions' runs' calls.
    project: Option<String>,
    /// A session's id: its runs' calls.
    #[serde(default)]
    #[ts(as = "Option<String>")]
    session: RawLabel,
}

impl RawUsageQuery {
    pub fn parse(self) -> Result<(TimeRange, UsageScope), QueryError> {
        let range = TimeRange::new(
            self.from_ms.map(UnixMillis::from_millis),
            self.until_ms.map(UnixMillis::from_millis),
        )?;
        let project = self
            .project
            .map(|name| name.parse().map_err(|_| QueryError::Filter))
            .transpose()?;
        let scope = UsageScope::new(project, self.session.parse()?)?;
        Ok((range, scope))
    }
}

/// `GET /api/search` parameters.
#[derive(Debug, Default, Deserialize, TS)]
#[serde(deny_unknown_fields, default)]
#[ts(export, optional_fields)]
pub(super) struct RawSearchQuery {
    q: Option<String>,
    #[ts(as = "Option<String>")]
    recording: RawLabel,
    /// A project's name: its sessions' runs' calls.
    project: Option<String>,
    #[ts(as = "Option<String>")]
    client: RawLabel,
    #[ts(as = "Option<String>")]
    field: RawLabel,
    from_ms: Option<u64>,
    until_ms: Option<u64>,
    limit: Option<u32>,
}

impl TryFrom<RawSearchQuery> for SearchQuery {
    type Error = super::ApiError;

    fn try_from(raw: RawSearchQuery) -> Result<Self, Self::Error> {
        Ok(SearchQuery {
            terms: raw
                .q
                .as_deref()
                .unwrap_or_default()
                .parse::<SearchTerms>()?,
            recording: raw.recording.parse()?,
            project: raw
                .project
                .map(|name| name.parse().map_err(|_| QueryError::Filter))
                .transpose()?,
            client: raw.client.parse()?,
            field: raw.field.parse()?,
            time: TimeRange::new(
                raw.from_ms.map(UnixMillis::from_millis),
                raw.until_ms.map(UnixMillis::from_millis),
            )?,
            limit: raw.limit.map_or(Ok(PageLimit::DEFAULT), PageLimit::new)?,
        })
    }
}

/// `GET /api/exposures` parameters.
#[derive(Debug, Default, Deserialize, TS)]
#[serde(deny_unknown_fields, default)]
#[ts(export, optional_fields)]
pub(super) struct RawExposureQuery {
    #[ts(as = "Option<String>")]
    recording: RawLabel,
    /// A project's name: its sessions' runs' calls.
    project: Option<String>,
    from_ms: Option<u64>,
    until_ms: Option<u64>,
    limit: Option<u32>,
}

impl TryFrom<RawExposureQuery> for ExposureQuery {
    type Error = QueryError;

    fn try_from(raw: RawExposureQuery) -> Result<Self, Self::Error> {
        Ok(ExposureQuery {
            recording: raw.recording.parse()?,
            project: raw
                .project
                .map(|name| name.parse().map_err(|_| QueryError::Filter))
                .transpose()?,
            time: TimeRange::new(
                raw.from_ms.map(UnixMillis::from_millis),
                raw.until_ms.map(UnixMillis::from_millis),
            )?,
            limit: raw.limit.map_or(Ok(PageLimit::DEFAULT), PageLimit::new)?,
        })
    }
}

/// `GET /api/live` parameters.
#[derive(Debug, Default, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, optional_fields)]
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
        let text =
            |value: Option<String>| value.as_deref().map(str::parse::<SearchText>).transpose();
        Ok(Self {
            limit: raw.limit.map_or(Ok(PageLimit::DEFAULT), PageLimit::new)?,
            after: raw.after.as_deref().map(Cursor::from_str).transpose()?,
            filter: CallFilter {
                recording: raw.recording.parse()?,
                project: raw
                    .project
                    .map(|name| name.parse().map_err(|_| QueryError::Filter))
                    .transpose()?,
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
    use super::{RawCallQuery, RawExposureQuery, RawSearchQuery, RawUsageQuery};
    use ren_protocol::route::Provider;
    use ren_store::query::{CallQuery, QueryError, StatusFilter, ToolFilter};
    use ren_store::search::{ExposureQuery, SearchQuery};

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

    fn query<T: serde::de::DeserializeOwned>(path: &str) -> T {
        let uri: http::Uri = path.parse().unwrap();
        axum::extract::Query::<T>::try_from_uri(&uri).unwrap().0
    }

    #[test]
    fn usage_search_and_exposures_parse_a_project_name_once() {
        let acme = Some("acme-web".parse().unwrap());
        let (_, scope) = query::<RawUsageQuery>("/api/usage?project=acme-web")
            .parse()
            .unwrap();
        assert_eq!(scope.project(), acme.as_ref());
        let search = SearchQuery::try_from(query::<RawSearchQuery>(
            "/api/search?q=migration&project=acme-web",
        ))
        .unwrap();
        assert_eq!(search.project, acme);
        let exposures =
            ExposureQuery::try_from(query::<RawExposureQuery>("/api/exposures?project=acme-web"))
                .unwrap();
        assert_eq!(exposures.project, acme);
        for invalid in ["Acme", "acme%20web", "-acme", ""] {
            assert!(matches!(
                query::<RawUsageQuery>(&format!("/api/usage?project={invalid}")).parse(),
                Err(QueryError::Filter)
            ));
            assert!(matches!(
                SearchQuery::try_from(query::<RawSearchQuery>(&format!(
                    "/api/search?q=migration&project={invalid}"
                ))),
                Err(crate::api::ApiError::Query(QueryError::Filter))
            ));
            assert!(matches!(
                ExposureQuery::try_from(query::<RawExposureQuery>(&format!(
                    "/api/exposures?project={invalid}"
                ))),
                Err(QueryError::Filter)
            ));
        }
    }

    #[test]
    fn usage_is_read_for_everything_one_project_or_one_session() {
        use ren_store::query::UsageScope;
        let session = "0198f6c2-3a4b-7c5d-8e9f-0a1b2c3d4e5f";
        let scope = |path: &str| query::<RawUsageQuery>(path).parse().map(|(_, scope)| scope);
        assert_eq!(scope("/api/usage"), Ok(UsageScope::Everything));
        assert_eq!(scope("/api/usage?session="), Ok(UsageScope::Everything));
        assert_eq!(
            scope(&format!("/api/usage?session={session}")),
            Ok(UsageScope::Session(session.parse().unwrap()))
        );
        assert_eq!(scope("/api/usage?session=acme"), Err(QueryError::Filter));
        assert_eq!(
            scope(&format!("/api/usage?project=acme-web&session={session}")),
            Err(QueryError::Scope)
        );
    }
}
