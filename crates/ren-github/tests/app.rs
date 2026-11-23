//! An App's installation token for one repository, from a loopback
//! stand-in for GitHub's REST API: the App's own token goes with each
//! request, the installation is found by the repository, and the token is
//! asked for that repository alone, to push and open pull requests. A key
//! made for the test and synthetic answers only.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly; the stand-in spawns a task"
)]

use aws_lc_rs::{encoding::AsDer as _, rsa::KeySize, signature};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use base64::Engine as _;
use ren_github::{App, AppId, AppKey, Endpoint, FetchError, Installations, MintError};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

#[derive(Default)]
struct StandIn {
    seen: Mutex<Vec<(String, Option<String>, Value)>>,
}

impl StandIn {
    fn saw(&self, what: &str, headers: &HeaderMap, body: Value) {
        let auth = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        self.seen
            .lock()
            .unwrap()
            .push((what.to_owned(), auth, body));
    }
}

async fn installation(
    State(stand_in): State<Arc<StandIn>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    stand_in.saw("installation", &headers, Value::Null);
    (
        StatusCode::OK,
        Json(json!({ "id": 77, "account": { "login": "acme" } })),
    )
}

async fn access_tokens(
    State(stand_in): State<Arc<StandIn>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    stand_in.saw("access_tokens", &headers, body);
    (
        StatusCode::CREATED,
        Json(json!({ "token": "ghs_FIXTURE", "expires_at": "2027-01-15T10:00:00Z" })),
    )
}

async fn serve() -> (Endpoint, Arc<StandIn>) {
    let stand_in = Arc::new(StandIn::default());
    let router = Router::new()
        .route("/repos/acme/web/installation", get(installation))
        .route("/app/installations/77/access_tokens", post(access_tokens))
        .with_state(stand_in.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (
        Endpoint::parse(&format!("http://{address}")).unwrap(),
        stand_in,
    )
}

fn app() -> App {
    let pair = signature::RsaKeyPair::generate(KeySize::Rsa2048).unwrap();
    let body = base64::engine::general_purpose::STANDARD.encode(pair.as_der().unwrap().as_ref());
    let pem = format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n");
    App::new(AppId::new(4242), AppKey::from_pem(&pem).unwrap())
}

#[tokio::test]
async fn a_token_is_minted_for_one_repository_to_push_and_open_pull_requests() {
    let (api, stand_in) = serve().await;
    let installations = Installations::new(api, app()).unwrap();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let minted = installations
        .repository_token(&"acme/web".parse().unwrap(), now)
        .await
        .unwrap();
    assert_eq!(minted.expires_at, "2027-01-15T10:00:00Z");
    assert!(!format!("{minted:?}").contains("FIXTURE"));
    let seen = stand_in.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for (_, auth, _) in seen.iter() {
        // The App's own token, three dotted parts, on every request.
        let auth = auth.as_deref().unwrap();
        assert!(
            auth.starts_with("Bearer ") && auth.matches('.').count() == 2,
            "{auth}"
        );
    }
    assert_eq!(
        seen[1],
        (
            "access_tokens".to_owned(),
            seen[1].1.clone(),
            json!({
                "repositories": ["web"],
                "permissions": { "contents": "write", "pull_requests": "write" },
            })
        )
    );
}

#[tokio::test]
async fn a_repository_the_app_is_not_installed_on_gets_no_token() {
    let (api, _) = serve().await;
    let installations = Installations::new(api, app()).unwrap();
    let refused = installations
        .repository_token(&"acme/other".parse().unwrap(), SystemTime::UNIX_EPOCH)
        .await
        .unwrap_err();
    assert!(
        matches!(refused, MintError::Fetch(FetchError::Status(status)) if status == StatusCode::NOT_FOUND),
        "{refused:?}"
    );
}
