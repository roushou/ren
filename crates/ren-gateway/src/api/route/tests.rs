use super::*;
use ren_harness::Harness;
use ren_protocol::route::Provider;
use ren_recorder::caller::UserId;
use ren_recorder::id::{CallId, RecordingId, RunId, TurnId};
use ren_recorder::time::UnixMillis;
use ren_store::query::{
    CallFilter, KindFilter, PageLimit, SearchText, Shelf, StatusFilter, ToolFilter, UsageScope,
};
use ren_store::search::{ExposureQuery, SearchField, SearchQuery, SearchTerms};
use uuid::Uuid;

fn every_read() -> Vec<ReadRequest> {
    let call = CallId::restore(Uuid::from_u128(1));
    let session = RecordingId::restore(Uuid::from_u128(2));
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
        recording: Some(session),
        project: Some("acme-web".parse().unwrap()),
        client: Some(Harness::Codex),
        user: Some("alice".parse::<UserId>().unwrap()),
        kind: Some(KindFilter::Model),
        provider: Some(Provider::OpenAi),
        model: Some("gpt & co".parse::<SearchText>().unwrap()),
        tool: Some("apply_patch".parse::<SearchText>().unwrap()),
        text: Some("a=b?c#d".parse::<SearchText>().unwrap()),
        status: Some(StatusFilter::Errors),
        tools: Some(ToolFilter::WithTools),
        time,
    };
    vec![
        ReadRequest::Calls(CallQuery::default()),
        ReadRequest::Calls(filtered),
        ReadRequest::Call(call),
        ReadRequest::CallActivity(call),
        ReadRequest::Recordings {
            limit,
            shelf: Shelf::Current,
            project: None,
        },
        ReadRequest::Recordings {
            limit,
            shelf: Shelf::Archived,
            project: Some("acme-web".parse().unwrap()),
        },
        ReadRequest::RecordingCalls {
            recording: session,
            limit,
            after: None,
        },
        ReadRequest::Turns {
            recording: session,
            limit,
        },
        ReadRequest::Report(session),
        ReadRequest::Turn(turn),
        ReadRequest::Runs {
            limit,
            shelf: Shelf::Archived,
            project: None,
        },
        ReadRequest::Runs {
            limit,
            shelf: Shelf::Current,
            project: Some("acme-web".parse().unwrap()),
        },
        ReadRequest::Run(RunId::restore(Uuid::from_u128(4))),
        ReadRequest::Conversation {
            run: RunId::restore(Uuid::from_u128(4)),
            bodies: ren_store::query::ItemBodies::Kept,
        },
        ReadRequest::Conversation {
            run: RunId::restore(Uuid::from_u128(4)),
            bodies: ren_store::query::ItemBodies::Omitted,
        },
        ReadRequest::Passages(RunId::restore(Uuid::from_u128(4))),
        ReadRequest::WorkItems {
            limit,
            shelf: Shelf::Current,
            project: None,
        },
        ReadRequest::WorkItems {
            limit,
            shelf: Shelf::Archived,
            project: Some("acme-web".parse().unwrap()),
        },
        ReadRequest::WorkItem(ren_recorder::id::WorkItemId::restore(Uuid::from_u128(5))),
        ReadRequest::Usage {
            range: TimeRange::default(),
            scope: UsageScope::Everything,
        },
        ReadRequest::Usage {
            range: time,
            scope: UsageScope::Project("acme-web".parse().unwrap()),
        },
        ReadRequest::Usage {
            range: time,
            scope: UsageScope::Session(ren_recorder::id::SessionId::restore(Uuid::from_u128(6))),
        },
        ReadRequest::Search(SearchQuery {
            terms: r#"run "install vps" & co"#.parse::<SearchTerms>().unwrap(),
            recording: Some(session),
            project: Some("acme-web".parse().unwrap()),
            client: Some(Harness::ClaudeCode),
            field: Some(SearchField::ToolInput),
            time,
            limit,
        }),
        ReadRequest::Exposures(ExposureQuery {
            recording: Some(session),
            project: None,
            time,
            limit,
        }),
        ReadRequest::Exposures(ExposureQuery {
            recording: None,
            project: Some("acme-web".parse().unwrap()),
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
        parse("/api/recordings?limit=0"),
        Err(ApiError::Query(QueryError::Limit))
    ));
    assert!(matches!(
        parse(&format!(
            "/api/recordings/{}/turns?after=x",
            Uuid::from_u128(2)
        )),
        Err(ApiError::Query(QueryError::Cursor))
    ));
}
