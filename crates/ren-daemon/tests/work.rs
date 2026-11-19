//! Work items through the API end to end: an order queued through the
//! control API, read back from the read API with what was on screen, sent
//! to a session, landed, and shipped; a change its state does not take is
//! refused, and a dismissed item is archived. Assembled with `TestGateway`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly; mock servers spawn tasks"
)]

mod support;

use axum::Router;
use http::StatusCode;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use support::{TestGateway, agent::ScriptedHost, client, config, mock};

const COMMIT: &str = "62e9e9e0000000000000000000000000000000ff";

async fn send(gateway: &TestGateway, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut request = client()
        .post(format!("{}/control/v1{path}", gateway.base()))
        .bearer_auth(TestGateway::CONTROL_TOKEN);
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(body.to_string());
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    (
        status,
        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap(),
    )
}

async fn read(gateway: &TestGateway, path: &str) -> Value {
    let response = client()
        .get(format!("{}{path}", gateway.base()))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{path}");
    serde_json::from_slice(&response.bytes().await.unwrap()).unwrap()
}

/// The item's state once the recorder has stored `state`.
async fn settled(gateway: &TestGateway, item: &str, state: &str) -> Value {
    for _ in 0..200 {
        let response = client()
            .get(format!("{}/api/work/{item}", gateway.base()))
            .send()
            .await
            .unwrap();
        if response.status().is_success() {
            let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            if body["state"] == state {
                return body;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{item} never became {state}");
}

fn order(note: &str) -> Value {
    json!({
        "page": {
            "path": "/recordings?shelf=current",
            "route": "/recordings",
            "title": "Recordings",
            "width": 1440,
            "height": 900,
            "theme": "dark",
            "revision": format!("{COMMIT}-dirty"),
        },
        "marks": [
            { "selector": "[aria-label=\"Recordings\"] tr", "text": "acme-api", "source": null,
              "width": 600, "height": 24, "note": note },
            { "selector": "h1", "text": "Recordings", "source": null,
              "width": 200, "height": 30, "note": "  " },
        ],
        "evidence": { "console": ["TypeError: turns is undefined"], "reads": ["/api/recordings?limit=100"] },
    })
}

#[tokio::test]
async fn a_work_item_goes_from_queued_to_shipped_through_the_api() {
    let upstream = mock(Router::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::with_runs(
        &config(&upstream),
        dir.path(),
        vec![PathBuf::from("/work")],
        ScriptedHost,
    )
    .await;

    let (status, body) = send(
        &gateway,
        "/work",
        Some(order("Show each turn count\nand trend")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["state"], "queued");
    let item = body["id"].as_str().unwrap().to_owned();
    let queued = settled(&gateway, &item, "queued").await;
    assert_eq!(queued["title"], "Show each turn count");
    // The mark without a note is left out; what was on screen is kept.
    assert_eq!(queued["order"]["marks"].as_array().unwrap().len(), 1);
    assert_eq!(
        queued["order"]["page"]["revision"],
        format!("{COMMIT}-dirty")
    );
    assert_eq!(
        queued["order"]["evidence"]["console"][0],
        "TypeError: turns is undefined"
    );
    let listed = read(&gateway, "/api/work").await;
    assert_eq!(listed["work"][0]["id"], item.as_str());

    // A queued item cannot land, and a session it is sent to must exist.
    let (status, body) = send(
        &gateway,
        &format!("/work/{item}/land"),
        Some(json!({ "commit": COMMIT })),
    )
    .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::CONFLICT, Some("work_state"))
    );
    let unknown = "00000000-0000-4000-8000-00000000ffff";
    let (status, _) = send(
        &gateway,
        &format!("/work/{item}/send"),
        Some(json!({ "session": unknown })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = send(&gateway, "/sessions", Some(json!({ "harness": "codex" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let session = body["session"].as_str().unwrap().to_owned();
    let mut sent = Value::Null;
    for _ in 0..200 {
        let (status, body) = send(
            &gateway,
            &format!("/work/{item}/send"),
            Some(json!({ "session": session })),
        )
        .await;
        if status == StatusCode::ACCEPTED {
            sent = body;
            break;
        }
        // The session is recorded after it is made.
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(sent["state"], "sent", "{sent}");
    settled(&gateway, &item, "sent").await;
    let (status, _) = send(
        &gateway,
        &format!("/work/{item}/land"),
        Some(json!({ "commit": COMMIT })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    settled(&gateway, &item, "landed").await;
    let (status, _) = send(
        &gateway,
        &format!("/work/{item}/ship"),
        Some(json!({ "revision": COMMIT })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let shipped = settled(&gateway, &item, "shipped").await;
    assert_eq!(shipped["session"], session.as_str());
    assert_eq!(shipped["landed"], COMMIT);
    assert_eq!(shipped["shipped"], COMMIT);
    // Shipped is done: archived, and dismissing it is refused.
    let (status, _) = send(&gateway, &format!("/work/{item}/dismiss"), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let archived = read(&gateway, "/api/work?shelf=archived").await;
    assert_eq!(archived["work"][0]["id"], item.as_str());
    assert!(
        read(&gateway, "/api/work").await["work"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // An order with no note is not queued.
    let (status, body) = send(&gateway, "/work", Some(order(" "))).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_order"))
    );
    gateway.stop().await;
}
