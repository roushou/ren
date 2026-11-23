//! The Release view's API end to end: the pages the canary captured, served
//! as PNG by name and never by path, and the operator's promotion request,
//! recorded only for the canary's passed build. Assembled with `TestGateway`.
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
use ren_release::{
    Assessment, Attempt, Canary, Check, CheckKind, Lineage, Outcome, Policy, PromotionRequest,
    Release, ShotName, Verdict,
};
use serde_json::{Value, json};
use support::{TestGateway, client, config, mock};

const CANARY: &str = "62e9e9e0000000000000000000000000000000ff";
const PRODUCTION: &str = "07f4bb7c1e2a3b4c5d6e7f8091a2b3c4d5e6f708";

fn release(outcome: Outcome) -> Release {
    let checks = vec![
        Check::new(CheckKind::Doctor, Outcome::Passed),
        Check::new(CheckKind::Reads, Outcome::Passed),
        Check::new(CheckKind::Pages, outcome),
    ];
    Release::assess(
        Assessment {
            main: CANARY.parse().unwrap(),
            production: PRODUCTION.parse().unwrap(),
            behind: Some(1),
            canary: Some((
                Canary::new(CANARY.parse().unwrap(), Verdict::ren(checks)),
                Lineage::Ahead,
                Attempt::Untried,
            )),
        },
        Policy::Manual,
        "03:00".parse().unwrap(),
    )
}

async fn promote(gateway: &TestGateway, commit: &str) -> (StatusCode, Value) {
    let response = client()
        .post(format!("{}/control/v1/release/promote", gateway.base()))
        .bearer_auth(TestGateway::CONTROL_TOKEN)
        .header("content-type", "application/json")
        .body(json!({ "commit": commit }).to_string())
        .send()
        .await
        .unwrap();
    let status = response.status();
    (
        status,
        serde_json::from_slice(&response.bytes().await.unwrap()).unwrap(),
    )
}

/// A read's status and JSON body (null when it has none).
async fn get(base: String, path: String) -> (StatusCode, Value) {
    let response = client().get(format!("{base}{path}")).send().await.unwrap();
    let status = response.status();
    let body = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap_or(Value::Null);
    (status, body)
}

#[tokio::test]
async fn the_release_view_reads_shots_and_records_a_promotion_request() {
    let upstream = mock(Router::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;

    // Nothing to promote before the factory has run.
    assert_eq!(promote(&gateway, CANARY).await.0, StatusCode::NOT_FOUND);
    let write = |release: &Release| {
        std::fs::write(
            dir.path().join(Release::FILE),
            serde_json::to_vec(release).unwrap(),
        )
        .unwrap();
    };
    // A build that failed is not promoted, nor a commit that is not the canary's.
    write(&release(Outcome::Failed {
        detail: "the sessions page did not load".into(),
    }));
    let (status, body) = promote(&gateway, CANARY).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::CONFLICT, Some("not_promotable"))
    );
    write(&release(Outcome::Passed));
    assert_eq!(promote(&gateway, PRODUCTION).await.0, StatusCode::CONFLICT);
    let (status, body) = promote(&gateway, CANARY).await;
    assert_eq!(
        (status, body["commit"].as_str()),
        (StatusCode::ACCEPTED, Some(CANARY))
    );
    let kept: PromotionRequest =
        serde_json::from_slice(&std::fs::read(dir.path().join(PromotionRequest::FILE)).unwrap())
            .unwrap();
    assert_eq!(kept.commit.to_string(), CANARY);

    // Ren's release is read as its project's: registered, it reads.
    for (name, repository, factory) in [
        ("ren", "/home/op/ren", "ren"),
        ("acme-web", "/home/op/dev/acme-web", "contract"),
    ] {
        let response = client()
            .post(format!("{}/control/v1/projects", gateway.base()))
            .bearer_auth(TestGateway::CONTROL_TOKEN)
            .header("content-type", "application/json")
            .body(json!({ "name": name, "repository": repository, "factory": factory }).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }
    let base = gateway.base();
    let read = |path: String| get(base.clone(), path);
    for _ in 0..200 {
        if read("/api/projects".to_owned()).await.1["projects"]
            .as_array()
            .is_some_and(|projects| projects.len() == 2)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // The pages the canary kept, by name.
    let shots = dir.path().join(ShotName::FOLDER);
    std::fs::create_dir_all(&shots).unwrap();
    std::fs::write(shots.join("sessions.png"), b"\x89PNG fake").unwrap();
    std::fs::write(shots.join("notes.txt"), b"not a shot").unwrap();
    let (_, body) = read("/api/projects/ren/release".to_owned()).await;
    assert_eq!(body["shots"], json!(["sessions"]));
    assert_eq!(body["promotion_requested"], CANARY);
    assert_eq!(body["release"]["main"], CANARY);
    // A contract project's release is its own, with no pages or request.
    let (_, body) = read("/api/projects/acme-web/release".to_owned()).await;
    assert_eq!(
        (
            &body["release"],
            &body["shots"],
            &body["promotion_requested"]
        ),
        (&Value::Null, &json!([]), &Value::Null)
    );
    let (_, listed) = read("/api/projects".to_owned()).await;
    assert_eq!(listed["projects"][1]["name"], "ren");
    assert_eq!(listed["projects"][1]["release"]["main"], CANARY);
    assert_eq!(listed["projects"][0]["release"], Value::Null);
    assert_eq!(
        read("/api/projects/acme-web/release/shots/sessions".to_owned())
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        read("/api/projects/acme-none/release".to_owned()).await.0,
        StatusCode::NOT_FOUND
    );
    let shot = client()
        .get(format!(
            "{}/api/projects/ren/release/shots/sessions",
            gateway.base()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(shot.status(), StatusCode::OK);
    assert_eq!(shot.headers()["content-type"], "image/png");
    assert_eq!(&shot.bytes().await.unwrap()[..], b"\x89PNG fake");
    for name in ["missing", "..%2F..%2Fetc%2Fpasswd", "Notes"] {
        let response = client()
            .get(format!(
                "{}/api/projects/ren/release/shots/{name}",
                gateway.base()
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{name}");
    }
    gateway.stop().await;
}

#[tokio::test]
async fn previews_are_asked_for_withdrawn_and_listed() {
    let upstream = mock(Router::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let ask = |path: &'static str, commit: &'static str| {
        let base = gateway.base();
        async move {
            let response = client()
                .post(format!("{base}/control/v1/previews/{path}"))
                .bearer_auth(TestGateway::CONTROL_TOKEN)
                .header("content-type", "application/json")
                .body(json!({ "commit": commit }).to_string())
                .send()
                .await
                .unwrap();
            let status = response.status();
            let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            (status, body)
        }
    };

    let (status, body) = ask("ask", PRODUCTION).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["commits"], json!([PRODUCTION]));
    let (_, body) = ask("ask", CANARY).await;
    assert_eq!(body["commits"], json!([CANARY, PRODUCTION]));
    let (status, body) = ask("withdraw", PRODUCTION).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["commits"], json!([CANARY]));
    let (status, body) = ask("withdraw", PRODUCTION).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");
    let read: ren_gateway::preview::PreviewRequests = serde_json::from_slice(
        &std::fs::read(dir.path().join(ren_gateway::preview::PreviewRequests::FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(read.commits(), [CANARY.parse().unwrap()]);

    // Without the control token, nothing changes.
    let refused = client()
        .post(format!("{}/control/v1/previews/ask", gateway.base()))
        .header("content-type", "application/json")
        .body(json!({ "commit": PRODUCTION }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    gateway.stop().await;
}
