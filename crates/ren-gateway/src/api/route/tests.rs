use super::*;
use ren_harness::Harness;
use ren_protocol::route::Provider;
use ren_recorder::caller::UserId;
use ren_recorder::id::{CallId, SessionId, TurnId};
use ren_recorder::time::UnixMillis;
use ren_store::query::{CallFilter, KindFilter, PageLimit, SearchText, StatusFilter, ToolFilter};
use ren_store::search::{ExposureQuery, SearchField, SearchQuery, SearchTerms};
use uuid::Uuid;

fn every_read() -> Vec<ReadRequest> {
    let call = CallId::restore(Uuid::from_u128(1));
    let session = SessionId::restore(Uuid::from_u128(2));
    let turn = TurnId::restore(Uuid::from_u128(3));
    let limit = PageLimit::new(7).unwrap();
    let time = TimeRange::new(
        Some(UnixMillis::from_millis(1_000)),
        Some(UnixMillis::from_millis(9_000)),
    )
    .unwrap();
    let mut filtered = CallQuery {
        limit,
        ..CallQuery::default()
    };
    filtered.filter = CallFilter {
        session: Some(session),
        client: Some(Harness::Codex),
        user: Some(UserId::parse("alice").unwrap()),
        kind: Some(KindFilter::Model),
        provider: Some(Provider::OpenAi),
        model: Some(SearchText::new("gpt & co").unwrap()),
        tool: Some(SearchText::new("apply_patch").unwrap()),
        text: Some(SearchText::new("a=b?c#d").unwrap()),
        status: Some(StatusFilter::Errors),
        tools: Some(ToolFilter::WithTools),
        time,
    };
    vec![
        ReadRequest::Calls(CallQuery::default()),
        ReadRequest::Calls(filtered),
        ReadRequest::Call(call),
        ReadRequest::CallActivity(call),
        ReadRequest::Sessions(limit),
        ReadRequest::SessionCalls {
            session,
            limit,
            after: None,
        },
        ReadRequest::Turns { session, limit },
        ReadRequest::Report(session),
        ReadRequest::Turn(turn),
        ReadRequest::Usage(TimeRange::default()),
        ReadRequest::Usage(time),
        ReadRequest::Search(SearchQuery {
            terms: SearchTerms::parse(r#"run "install vps" & co"#).unwrap(),
            session: Some(session),
            client: Some(Harness::ClaudeCode),
            field: Some(SearchField::ToolInput),
            time,
            limit,
        }),
        ReadRequest::Exposures(ExposureQuery {
            session: Some(session),
            time,
            limit,
        }),
    ]
}

#[test]
fn every_read_round_trips_through_its_uri() {
    for request in every_read() {
        let read = ApiRead(request);
        let uri: Uri = read.uri().parse().unwrap();
        let parsed = ApiRead::parse(&uri).unwrap_or_else(|error| panic!("{uri}: {error}"));
        assert_eq!(parsed, read, "{uri}");
        // Every read is served on one of the router's routes.
        let path = uri.path();
        assert!(
            ApiRead::ROUTES.iter().any(|route| {
                let pattern: Vec<&str> = route.split('/').collect();
                let actual: Vec<&str> = path.split('/').collect();
                pattern.len() == actual.len()
                    && pattern
                        .iter()
                        .zip(&actual)
                        .all(|(want, got)| *want == "{id}" || want == got)
            }),
            "{path}"
        );
    }
}

#[test]
fn unknown_paths_ids_and_parameters_are_refused() {
    let parse = |uri: &str| ApiRead::parse(&uri.parse().unwrap()).map(|_| ());
    assert!(matches!(parse("/api/nothing"), Err(ApiError::NotFound)));
    assert!(matches!(
        parse("/api/calls/not-an-id"),
        Err(ApiError::NotFound)
    ));
    assert!(matches!(
        parse("/api/calls?unknown=1"),
        Err(ApiError::Query(QueryError::Filter))
    ));
    assert!(matches!(
        parse("/api/sessions?limit=0"),
        Err(ApiError::Query(QueryError::Limit))
    ));
    assert!(matches!(
        parse(&format!(
            "/api/sessions/{}/turns?after=x",
            Uuid::from_u128(2)
        )),
        Err(ApiError::Query(QueryError::Cursor))
    ));
}
