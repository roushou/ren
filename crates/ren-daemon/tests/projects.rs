//! Projects through the control API end to end: registered, refused when
//! the name or the repository is taken (archived or not) or the text is
//! not a project, archived and unarchived, each read back from the store.
//! Assembled with `TestGateway`.
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
use ren_store::{Reader, view::ProjectView};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
use support::{TestGateway, client, config, mock};

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

/// The projects once the recorder has stored one that `done` accepts.
async fn settled(dir: &Path, done: impl Fn(&[ProjectView]) -> bool) -> Vec<ProjectView> {
    for _ in 0..200 {
        let reader = Reader::open(dir, 1).await.unwrap();
        let projects = reader.projects().await.unwrap();
        reader.close().await;
        if done(&projects) {
            return projects;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the projects never settled");
}

#[tokio::test]
async fn a_project_is_registered_refused_when_taken_and_archived_through_the_api() {
    let upstream = mock(Router::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;

    let web =
        json!({ "name": "acme-web", "repository": "/home/op/dev/acme-web", "factory": "contract" });
    let (status, body) = send(&gateway, "/projects", Some(web.clone())).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["name"], "acme-web");
    let id = body["id"].as_str().unwrap().to_owned();
    let projects = settled(dir.path(), |projects| !projects.is_empty()).await;
    assert_eq!(projects[0].id.to_string(), id);
    assert_eq!(projects[0].repository.to_string(), "/home/op/dev/acme-web");

    // The name, or the repository, is taken.
    for taken in [
        json!({ "name": "acme-web", "repository": "/home/op/dev/other", "factory": "contract" }),
        json!({ "name": "other", "repository": "/home/op/dev/acme-web", "factory": "ren" }),
    ] {
        let (status, body) = send(&gateway, "/projects", Some(taken)).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::CONFLICT, Some("project_taken")),
            "{body}"
        );
    }
    // Text that is not a project.
    for invalid in [
        json!({ "name": "Acme Web", "repository": "/home/op/dev/x", "factory": "contract" }),
        json!({ "name": "x", "repository": "dev/x", "factory": "contract" }),
        json!({ "name": "x", "repository": "/home/op/dev/x", "factory": "make" }),
    ] {
        let (status, body) = send(&gateway, "/projects", Some(invalid)).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_project")),
            "{body}"
        );
    }

    // Archived, then refused again; an archived project still holds its name.
    let (status, _) = send(&gateway, "/projects/acme-web/archive", None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    settled(dir.path(), |projects| projects[0].archived_at.is_some()).await;
    let (status, body) = send(&gateway, "/projects/acme-web/archive", None).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::CONFLICT, Some("archived"))
    );
    let (status, body) = send(&gateway, "/projects", Some(web)).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::CONFLICT, Some("project_taken"))
    );
    assert!(body["message"].as_str().unwrap().contains("archived"));
    let (status, _) = send(&gateway, "/projects/acme-web/unarchive", None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    settled(dir.path(), |projects| projects[0].archived_at.is_none()).await;
    let (status, _) = send(&gateway, "/projects/acme-none/archive", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
