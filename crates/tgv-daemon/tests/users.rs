//! Callers end to end: each forwarded call records who made it, identity
//! headers never reach the provider, and a refused identity is answered and
//! recorded without forwarding. Assembled with `TestGateway` against a
//! loopback mock; tokens and logins are fixtures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly, and mock servers are spawned tasks"
)]

use axum::{Router, extract::Request, response::IntoResponse};
use http::StatusCode;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tgv_daemon::config::Config;
use tgv_gateway::user::{UserConfig, UserToken, Users};
use tgv_protocol::label::Stored;
use tgv_recorder::{
    caller::{CallerKind, TailscaleLogin, UserId},
    capture::CallRejection,
    id::SequentialIds,
};
use tgv_store::{
    Reader,
    query::{CallFilter, CallQuery, TimeRange},
    view::CallView,
};

mod support;

use support::{Mock, TestGateway, client, mock, recorded};

struct Tokens {
    bob: UserToken,
    unknown: UserToken,
}

fn tokens() -> Tokens {
    let ids = SequentialIds::default();
    Tokens {
        bob: UserToken::generate(&ids),
        unknown: UserToken::generate(&ids),
    }
}

fn config(mock: &Mock, tokens: &Tokens) -> Config {
    Config {
        users: vec![
            UserConfig {
                id: UserId::parse("alice").unwrap(),
                tailscale_logins: vec![TailscaleLogin::parse("alice@example.com").unwrap()],
                token_blake3: None,
            },
            UserConfig {
                id: UserId::parse("bob").unwrap(),
                tailscale_logins: vec![],
                token_blake3: Some(tokens.bob.digest()),
            },
        ],
        ..support::config(mock)
    }
}

/// Answers every request, saying whether identity headers reached it.
fn upstream(reached: Arc<AtomicUsize>) -> Router {
    Router::new().fallback(move |request: Request| {
        let reached = reached.clone();
        async move {
            reached.fetch_add(1, Ordering::SeqCst);
            let leaked = request.headers().keys().any(|name| {
                name.as_str().starts_with("x-tgv-") || name.as_str().starts_with("tailscale-")
            });
            (StatusCode::OK, if leaked { "leaked" } else { "clean" }).into_response()
        }
    })
}

async fn post(gateway: &TestGateway, headers: &[(&str, &str)]) -> reqwest::Response {
    let mut request = client()
        .post(format!("{}/anthropic/v1/messages", gateway.base()))
        .header("authorization", "Bearer CLIENT-OAUTH")
        .body(r#"{"model":"fixture","messages":[]}"#);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request.send().await.unwrap()
}

fn caller(call: &CallView) -> (CallerKind, Option<&str>, Option<&str>) {
    let caller = call.caller.as_ref().unwrap();
    (
        caller.kind.known().unwrap(),
        caller.user.as_deref(),
        caller.login.as_deref(),
    )
}

#[tokio::test]
async fn every_call_records_its_caller_and_identity_never_reaches_the_provider() {
    let reached = Arc::new(AtomicUsize::new(0));
    let mock = mock(upstream(reached.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let tokens = tokens();
    let gateway = TestGateway::start(&config(&mock, &tokens), dir.path()).await;
    let requests: [&[(&str, &str)]; 4] = [
        &[("tailscale-user-login", "alice@example.com")],
        &[(Users::TOKEN_HEADER, tokens.bob.expose())],
        &[("tailscale-user-login", "carol@example.com")],
        &[],
    ];
    for headers in requests {
        let response = post(&gateway, headers).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "clean");
    }
    gateway.stop().await;
    assert_eq!(reached.load(Ordering::SeqCst), 4);

    let mut calls = recorded(dir.path()).await;
    calls.reverse();
    let callers: Vec<_> = calls.iter().map(caller).collect();
    assert_eq!(
        callers,
        [
            (
                CallerKind::UserTailscale,
                Some("alice"),
                Some("alice@example.com")
            ),
            (CallerKind::UserToken, Some("bob"), None),
            (CallerKind::Unlisted, None, Some("carol@example.com")),
            (CallerKind::Unidentified, None, None),
        ]
    );

    // History filters by user, and usage counts each user's calls.
    let reader = Reader::open(dir.path(), 1).await.unwrap();
    let query = CallQuery {
        filter: CallFilter {
            user: Some(UserId::parse("bob").unwrap()),
            ..CallFilter::default()
        },
        ..CallQuery::default()
    };
    let bobs = reader.calls(&query).await.unwrap().calls;
    assert_eq!(bobs.len(), 1);
    assert_eq!(bobs[0].id, calls[1].id);
    let mut usage: Vec<(Option<String>, u64)> = reader
        .usage(TimeRange::default())
        .await
        .unwrap()
        .users
        .into_iter()
        .map(|user| (user.user, user.usage.calls))
        .collect();
    usage.sort();
    assert_eq!(
        usage,
        [
            (None, 2),
            (Some("alice".to_owned()), 1),
            (Some("bob".to_owned()), 1)
        ]
    );
    reader.close().await;
}

#[tokio::test]
async fn a_refused_identity_is_answered_and_recorded_without_forwarding() {
    let reached = Arc::new(AtomicUsize::new(0));
    let mock = mock(upstream(reached.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let tokens = tokens();
    let gateway = TestGateway::start(&config(&mock, &tokens), dir.path()).await;
    let refused: [&[(&str, &str)]; 2] = [
        &[(Users::TOKEN_HEADER, tokens.unknown.expose())],
        &[
            ("tailscale-user-login", "alice@example.com"),
            (Users::TOKEN_HEADER, tokens.bob.expose()),
        ],
    ];
    for headers in refused {
        let response = post(&gateway, headers).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()["x-should-retry"], "false");
        let body: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(body["error"]["type"], "tgv_identity_refused");
        let message = body["error"]["message"].as_str().unwrap();
        assert!(!message.contains("tgvu_"), "{message}");
    }
    gateway.stop().await;
    assert_eq!(reached.load(Ordering::SeqCst), 0);

    let calls = recorded(dir.path()).await;
    assert_eq!(calls.len(), 2);
    for call in &calls {
        assert_eq!(
            call.rejection.as_ref().and_then(Stored::known),
            Some(CallRejection::IdentityRefused)
        );
        assert_eq!(caller(call), (CallerKind::Unidentified, None, None));
    }
}
