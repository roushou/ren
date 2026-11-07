#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly, and mock servers are spawned tasks"
)]
mod support;

use axum::{Router, body::Body};
use bytes::Bytes;
use futures_util::StreamExt;
use http::StatusCode;
use std::{convert::Infallible, time::Duration};
use support::{client, config, mock, recorded, terminal};
use tgv_daemon::{Daemon, config::Config};
use tgv_recorder::capture::AttemptTerminal;
use tokio::{net::TcpListener, time::timeout};

#[tokio::test]
async fn same_daemon_owns_start_shutdown_cancelled_wait_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("state");
    let mut daemon = Daemon::new(
        Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            ..Config::default()
        }
        .resolve()
        .unwrap(),
        &data,
        support::build(),
    );
    assert!(!data.exists());
    assert!(daemon.address().is_err());
    assert!(daemon.wait().await.is_err());
    daemon.shutdown(); // No run exists yet; this must not cancel the next run.
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    for _ in 0..2 {
        daemon.start().await.unwrap();
        let address = daemon.address().unwrap();
        assert_ne!(address.port(), 0);
        assert!(daemon.start().await.is_err());
        assert_eq!(daemon.address().unwrap(), address);
        assert!(
            timeout(Duration::from_millis(20), daemon.wait())
                .await
                .is_err()
        );
        let response = client
            .get(format!("http://{address}/health"))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        drop(response);
        daemon.shutdown();
        daemon.shutdown();
        // A requested shutdown still owns an unjoined run.
        assert!(daemon.start().await.is_err());
        timeout(Duration::from_secs(5), daemon.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(daemon.wait().await.is_err());
        daemon.shutdown();
    }
}

#[tokio::test]
async fn failed_start_can_retry_after_listener_and_storage_conflicts_clear() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("state");
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    let mut daemon = Daemon::new(
        Config {
            listen: address,
            ..Config::default()
        }
        .resolve()
        .unwrap(),
        &data,
        support::build(),
    );
    assert!(daemon.start().await.is_err());
    assert!(daemon.address().is_err());
    assert!(!data.exists());
    drop(occupied);
    daemon.start().await.unwrap();

    let mut contender = Daemon::new(
        Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            ..Config::default()
        }
        .resolve()
        .unwrap(),
        &data,
        support::build(),
    );
    assert!(contender.start().await.is_err());
    assert!(contender.address().is_err());
    daemon.shutdown();
    timeout(Duration::from_secs(5), daemon.wait())
        .await
        .unwrap()
        .unwrap();
    contender.start().await.unwrap();
    contender.shutdown();
    timeout(Duration::from_secs(5), contender.wait())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn dropping_daemon_requests_shutdown_and_releases_storage() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        ..Config::default()
    }
    .resolve()
    .unwrap();
    let mut daemon = Daemon::new(config.clone(), dir.path(), support::build());
    daemon.start().await.unwrap();
    drop(daemon);
    let mut replacement = Daemon::new(config, dir.path(), support::build());
    timeout(Duration::from_secs(5), async {
        while replacement.start().await.is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    replacement.shutdown();
    timeout(Duration::from_secs(5), replacement.wait())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn headless_daemon_exposes_only_gateway_and_management_routes() {
    let upstream = mock(Router::new().fallback(|| async { StatusCode::NO_CONTENT })).await;
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::new(
        config(&upstream).resolve().unwrap(),
        dir.path(),
        support::build(),
    );
    daemon.start().await.unwrap();
    let base = format!("http://{}", daemon.address().unwrap());
    for path in [
        "/",
        "/traffic",
        "/sessions",
        "/sessions/example",
        "/recorder",
        "/_app/version.json",
    ] {
        let response = client().get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let response = client()
        .get(format!("{base}/api/storage"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-origin")
    );
    let value: serde_json::Value =
        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(value["api_version"], 2);
    assert_eq!(value["usage"]["calls"], 0);
    assert_eq!(value["policy"]["enabled"], true);
    assert_eq!(value["policy"]["max_storage_bytes"], 2147483648u64);
    // The daemon reports the build it was given.
    let health = client()
        .get(format!("{base}/health"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let health: serde_json::Value = serde_json::from_slice(&health).unwrap();
    assert_eq!(
        health["build"],
        serde_json::json!({ "version": "0.0.0-test", "revision": "unknown" })
    );
    assert_eq!(
        client()
            .post(format!("{base}/api/storage"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        client()
            .get(format!("{base}/not-an-asset"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    daemon.shutdown();
    timeout(Duration::from_secs(5), daemon.wait())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn shutdown_lets_a_running_stream_finish_before_the_deadline() {
    let upstream = mock(Router::new().fallback(|| async {
        let chunks = futures_util::stream::iter(["data: first\n\n", "data: last\n\n"]).then(
            |chunk| async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                Ok::<_, Infallible>(Bytes::from_static(chunk.as_bytes()))
            },
        );
        Body::from_stream(chunks)
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::new(
        config(&upstream).resolve().unwrap(),
        dir.path(),
        support::build(),
    );
    daemon.start().await.unwrap();
    let base = format!("http://{}", daemon.address().unwrap());
    let response = client()
        .post(format!("{base}/codex/responses"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    daemon.shutdown();
    // Draining refuses new calls but leaves the running one alone.
    let refused = client()
        .post(format!("{base}/codex/responses"))
        .body("{}")
        .send()
        .await;
    assert!(refused.is_err() || refused.unwrap().status() == StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.bytes().await.unwrap(),
        "data: first\n\ndata: last\n\n"
    );
    timeout(Duration::from_secs(5), daemon.wait())
        .await
        .unwrap()
        .unwrap();
    let records = recorded(dir.path()).await;
    assert_eq!(records.len(), 1);
    assert_eq!(terminal(&records[0]), AttemptTerminal::BodyComplete);
}

#[tokio::test]
async fn operator_state_survives_a_restart() {
    const SESSION: &str = "00000000-0000-4000-8000-000000000007";
    // A loopback upstream: no call may reach a provider.
    let reached = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = reached.clone();
    let upstream = mock(Router::new().fallback(move || {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { "{}" }
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("state");
    let mut daemon = Daemon::new(
        config(&upstream).resolve().unwrap(),
        &data,
        support::build(),
    );
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    // Each run writes a fresh control token.
    let control = |daemon: &Daemon, path: &str, body: &str| {
        let token = std::fs::read_to_string(data.join("control.token")).unwrap();
        client
            .post(format!(
                "http://{}/control/v1/{path}",
                daemon.address().unwrap()
            ))
            .bearer_auth(token)
            .header("content-type", "application/json")
            .body(body.to_owned())
            .send()
    };
    let message = |daemon: &Daemon, session: Option<&str>| {
        let mut request = client
            .post(format!(
                "http://{}/anthropic/v1/messages",
                daemon.address().unwrap()
            ))
            .header("content-type", "application/json")
            .body("{}");
        if let Some(session) = session {
            request = request.header("x-claude-code-session-id", session);
        }
        request.send()
    };

    daemon.start().await.unwrap();
    let paused = control(&daemon, "pause", r#"{"retry_after_seconds":30}"#)
        .await
        .unwrap();
    assert_eq!(paused.status(), StatusCode::OK);
    let blocked = control(&daemon, &format!("sessions/{SESSION}/block"), "{}")
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::OK);
    daemon.shutdown();
    timeout(Duration::from_secs(5), daemon.wait())
        .await
        .unwrap()
        .unwrap();

    // A restart keeps the pause and the block, and revisions continue.
    daemon.start().await.unwrap();
    let response = message(&daemon, None).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["retry-after"], "30");
    let resumed = control(&daemon, "resume", "").await.unwrap();
    let state: serde_json::Value = serde_json::from_slice(&resumed.bytes().await.unwrap()).unwrap();
    assert_eq!(state["state"]["revision"], 3);
    assert_eq!(state["state"]["pause"]["state"], "running");
    let response = message(&daemon, Some(SESSION)).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        message(&daemon, None).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(reached.load(std::sync::atomic::Ordering::SeqCst), 1);
    daemon.shutdown();
    timeout(Duration::from_secs(5), daemon.wait())
        .await
        .unwrap()
        .unwrap();
}
