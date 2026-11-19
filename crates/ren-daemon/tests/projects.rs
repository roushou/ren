//! Projects through the control API end to end: registered, refused when
//! the name or the repository is taken (archived or not) or the text is
//! not a project, archived and unarchived; and what they own: the sessions
//! made in their repository or asked for by name, the work items queued
//! for them (Ren's by default), sends checked against both, and earlier
//! sessions adopted. Each read back from the store. Assembled with
//! `TestGateway`.
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
use std::path::PathBuf;
use std::{path::Path, time::Duration};
use support::{TestGateway, agent::ScriptedHost, client, config, mock};

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

/// `session`'s project once the recorder has stored one that `done` takes.
async fn owner(dir: &Path, session: &str, done: impl Fn(Option<String>) -> bool) -> Option<String> {
    let session = session.parse().unwrap();
    for _ in 0..200 {
        let reader = Reader::open(dir, 1).await.unwrap();
        let view = reader.session(session).await.unwrap();
        reader.close().await;
        let project = view.and_then(|view| view.project).map(|id| id.to_string());
        if done(project.clone()) {
            return project;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{session} never settled");
}

/// A work item's project once it is stored.
async fn item_project(dir: &Path, item: &str) -> Option<String> {
    let item = item.parse().unwrap();
    for _ in 0..200 {
        let reader = Reader::open(dir, 1).await.unwrap();
        let view = reader.work_item(item).await.unwrap();
        reader.close().await;
        if let Some(view) = view {
            return view.project.map(|id| id.to_string());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{item} was never stored");
}

fn order() -> Value {
    json!({
        "page": { "path": "/", "route": "/", "title": "Overview", "width": 1440,
                  "height": 900, "theme": "light", "revision": null },
        "marks": [{ "selector": "main", "text": "", "source": null, "width": 10,
                    "height": 10, "note": "Make it clearer" }],
    })
}

#[tokio::test]
async fn sessions_and_work_items_belong_to_the_project_they_are_for() {
    let upstream = mock(Router::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::with_runs(
        &config(&upstream),
        dir.path(),
        vec![PathBuf::from("/work")],
        ScriptedHost,
    )
    .await;
    let create = |body: Value| send(&gateway, "/sessions", Some(body));

    // A session from before acme was registered works for none.
    let (status, body) = create(json!({ "harness": "codex", "dir": "/work/acme" })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let earlier = body["session"].as_str().unwrap().to_owned();

    let (_, acme) = send(
        &gateway,
        "/projects",
        Some(json!({ "name": "acme", "repository": "/work/acme", "factory": "contract" })),
    )
    .await;
    let acme = acme["id"].as_str().unwrap().to_owned();
    let (_, ren) = send(
        &gateway,
        "/projects",
        Some(json!({ "name": "ren", "repository": "/work/web", "factory": "ren" })),
    )
    .await;
    let ren = ren["id"].as_str().unwrap().to_owned();
    settled(dir.path(), |projects| projects.len() == 2).await;

    // In its repository, or asked for by name, a session works for acme;
    // in scratch, for none.
    let (_, body) = create(json!({ "harness": "codex", "dir": "/work/acme" })).await;
    let in_repository = body["session"].as_str().unwrap().to_owned();
    let (status, body) =
        create(json!({ "harness": "codex", "project": "acme", "workspace": "folder" })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let by_name = body["session"].as_str().unwrap().to_owned();
    let (_, body) = create(json!({ "harness": "codex" })).await;
    let scratch = body["session"].as_str().unwrap().to_owned();
    for session in [&in_repository, &by_name] {
        assert_eq!(
            owner(dir.path(), session, |project| project.is_some()).await,
            Some(acme.clone())
        );
    }
    assert_eq!(owner(dir.path(), &scratch, |_| true).await, None);
    assert_eq!(owner(dir.path(), &earlier, |_| true).await, None);
    // A project and a folder together, or a project that is not one.
    let (status, _) =
        create(json!({ "harness": "codex", "project": "acme", "dir": "/work/acme" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = create(json!({ "harness": "codex", "project": "acme-none" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Queued with no project, an item is Ren's; named, it is that one's.
    let (status, body) = send(&gateway, "/work", Some(order())).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let for_ren = body["id"].as_str().unwrap().to_owned();
    let (_, body) = send(&gateway, "/work?project=acme", Some(order())).await;
    let for_acme = body["id"].as_str().unwrap().to_owned();
    assert_eq!(item_project(dir.path(), &for_ren).await, Some(ren));
    assert_eq!(
        item_project(dir.path(), &for_acme).await,
        Some(acme.clone())
    );

    // Ren's item does not go to a session working for acme; acme's does.
    let (status, body) = send(
        &gateway,
        &format!("/work/{for_ren}/send"),
        Some(json!({ "session": in_repository })),
    )
    .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::CONFLICT, Some("project_mismatch"))
    );
    let (status, body) = send(
        &gateway,
        &format!("/work/{for_acme}/send"),
        Some(json!({ "session": in_repository })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");

    // Adopting takes the earlier session in acme's repository, not scratch.
    let (status, body) = send(&gateway, "/projects/acme/adopt", None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["sessions"], json!([earlier]));
    assert_eq!(
        owner(dir.path(), &earlier, |project| project.is_some()).await,
        Some(acme)
    );
    assert_eq!(owner(dir.path(), &scratch, |_| true).await, None);
}

#[tokio::test]
async fn a_projects_workspaces_are_its_sessions_worktrees() {
    let upstream = mock(Router::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::with_runs(
        &config(&upstream),
        dir.path(),
        vec![PathBuf::from("/work")],
        ScriptedHost,
    )
    .await;
    let (status, _) = send(
        &gateway,
        "/projects",
        Some(json!({ "name": "acme", "repository": "/work/acme", "factory": "contract" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    settled(dir.path(), |projects| !projects.is_empty()).await;
    // One session in a workspace of its own, one in the folder itself.
    let (status, body) = send(
        &gateway,
        "/sessions",
        Some(json!({ "harness": "codex", "project": "acme" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let in_workspace = body["session"].as_str().unwrap().to_owned();
    send(
        &gateway,
        "/sessions",
        Some(json!({ "harness": "codex", "project": "acme", "workspace": "folder" })),
    )
    .await;
    let read = || async {
        let response = client()
            .get(format!("{}/api/projects/acme/workspaces", gateway.base()))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        serde_json::from_slice::<Value>(&response.bytes().await.unwrap()).unwrap()
    };
    let mut listed = Value::Null;
    for _ in 0..200 {
        listed = read().await;
        if listed["workspaces"]
            .as_array()
            .is_some_and(|all| !all.is_empty())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let workspaces = listed["workspaces"].as_array().unwrap();
    assert_eq!(workspaces.len(), 1, "{listed}");
    assert_eq!(workspaces[0]["session"], in_workspace.as_str());
    assert_eq!(workspaces[0]["name"], "acme-1");
    assert_eq!(workspaces[0]["ports"], json!([20000, 20099]));
    assert!(
        workspaces[0]["data"]
            .as_str()
            .unwrap()
            .ends_with(".data/acme-1")
    );
    let missing = client()
        .get(format!(
            "{}/api/projects/acme-none/workspaces",
            gateway.base()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}
