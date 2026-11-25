//! The read API's JSON, pinned for the UI. Each read's answer to a fixed,
//! synthetic history is compared with `ui/apps/web/tests/fixtures/api/<read>.json`,
//! and the UI's decoder tests parse those same files, so a change on either
//! side that breaks the contract fails a test. A test binary of its own, so
//! its gateway's sequential IDs are the same on every run.
//!
//! After an intended change, regenerate with `REN_UPDATE_FIXTURES=1`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly; mock servers spawn tasks"
)]

mod support;

use axum::{Router, body::Body, http::Response, routing::post};
use std::path::PathBuf;
use support::{TestGateway, client, config, mock};

const SESSION: &str = "00000000-0000-4000-8000-0000000000aa";
const PROMPT: &str = "00000000-0000-4000-8000-0000000000bb";

/// A streamed Messages response: a reply and a tool call.
fn stream() -> String {
    let event = |name: &str, data: serde_json::Value| format!("event: {name}\ndata: {data}\n\n");
    [
        event("message_start", serde_json::json!({ "type": "message_start", "message": { "model": "claude-test", "usage": { "input_tokens": 12, "cache_read_input_tokens": 3000, "cache_creation_input_tokens": 40 } } })),
        event("content_block_start", serde_json::json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } })),
        event("content_block_delta", serde_json::json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "Running the parser tests." } })),
        event("content_block_stop", serde_json::json!({ "type": "content_block_stop", "index": 0 })),
        event("content_block_start", serde_json::json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {} } })),
        event("content_block_delta", serde_json::json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{\"command\":\"cargo test parser\"}" } })),
        event("content_block_stop", serde_json::json!({ "type": "content_block_stop", "index": 1 })),
        event("message_delta", serde_json::json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": { "output_tokens": 30 } })),
        event("message_stop", serde_json::json!({ "type": "message_stop" })),
    ]
    .concat()
}

/// The configuration revision hashes the mock upstream's port, which
/// differs per run: it is pinned as a placeholder.
fn normalized(mut value: serde_json::Value) -> serde_json::Value {
    match &mut value {
        serde_json::Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                *field = if key == "config_revision" && field.is_string() {
                    serde_json::Value::String("<config revision>".to_owned())
                } else {
                    normalized(field.take())
                };
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                *item = normalized(item.take());
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
    value
}

/// An order marked on the recordings page, with what else was on screen.
fn work_order() -> serde_json::Value {
    serde_json::json!({
        "page": {
            "path": "/recordings",
            "route": "/recordings",
            "title": "Ren - Recordings",
            "width": 1440,
            "height": 900,
            "theme": "light",
            "revision": "62e9e9e0000000000000000000000000000000ff",
        },
        "marks": [{
            "selector": "[aria-label=\"Recordings\"] table > tbody > tr",
            "text": "acme-api",
            "source": null,
            "width": 640,
            "height": 28,
            "note": "Show each recording's turn count",
        }],
        "evidence": {
            "console": ["TypeError: turns is undefined"],
            "reads": ["/api/recordings?limit=100"],
        },
    })
}

/// A release whose canary build passed, one commit ahead of production,
/// waiting for the operator.
fn release() -> ren_release::Release {
    use ren_release::{
        Assessment, Attempt, Canary, Check, CheckKind, Lineage, Outcome, Policy, Release, Verdict,
    };
    let main = "62e9e9e0000000000000000000000000000000ff";
    let checks = vec![
        Check::new(CheckKind::Doctor, Outcome::Passed),
        Check::new(CheckKind::Reads, Outcome::Passed),
        Check::new(CheckKind::Pages, Outcome::Passed),
    ];
    Release::assess(
        Assessment {
            main: main.parse().unwrap(),
            production: "07f4bb7c1e2a3b4c5d6e7f8091a2b3c4d5e6f708".parse().unwrap(),
            behind: Some(1),
            canary: Some((
                Canary::new(main.parse().unwrap(), Verdict::ren(checks)),
                Lineage::Ahead,
                Attempt::Untried,
            )),
        },
        Policy::Manual,
        "03:00".parse().unwrap(),
    )
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui/apps/web/tests/fixtures/api")
}

#[tokio::test]
async fn the_read_api_answers_as_the_ui_fixtures_pin() {
    let upstream = mock(Router::new().route(
        "/v1/messages",
        post(|| async {
            Response::builder()
                .header("content-type", "text/event-stream")
                // Synthetic limit headers, as a subscription's head carries them.
                .header("anthropic-ratelimit-unified-status", "allowed")
                .header("anthropic-ratelimit-unified-5h-utilization", "0.12")
                .body(Body::from(stream()))
                .unwrap()
        }),
    ))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let gateway = TestGateway::start(&config(&upstream), dir.path()).await;
    let key = format!("sk-ant-{}a1b2", "api03xY9z".repeat(5));
    let requests = [
        serde_json::json!({ "model": "claude-test", "stream": true, "messages": [
            { "role": "user", "content": "Fix the parser" },
        ] }),
        serde_json::json!({ "model": "claude-test", "stream": true, "messages": [
            { "role": "user", "content": "Fix the parser" },
            { "role": "assistant", "content": [{ "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {} }] },
            { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "is_error": true, "content": format!("test parser ... FAILED\nK={key}") }] },
        ] }),
    ];
    let base = gateway.base();
    let read = |path: String| {
        let url = format!("{base}{path}");
        async move {
            let response = client().get(&url).send().await.unwrap();
            assert!(response.status().is_success(), "{url}");
            serde_json::from_slice::<serde_json::Value>(&response.bytes().await.unwrap()).unwrap()
        }
    };
    // One call at a time, each recorded before the next starts: the store
    // takes recording and turn IDs as it commits, from the same sequence.
    for (sent, request) in requests.iter().enumerate() {
        let response = client()
            .post(format!("{}/anthropic/v1/messages", gateway.base()))
            .header("x-claude-code-session-id", SESSION)
            .header("x-claude-code-prompt-id", PROMPT)
            .body(request.to_string())
            .send()
            .await
            .unwrap();
        response.text().await.unwrap();
        for _ in 0..200 {
            let recorded = read("/api/calls".to_owned()).await;
            if recorded["calls"].as_array().is_some_and(|calls| {
                calls.len() == sent + 1 && calls.iter().all(|call| call["state"] == "finished")
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    // Digests are written as each call finishes; wait for both.
    let mut calls = serde_json::Value::Null;
    for _ in 0..200 {
        calls = read("/api/calls".to_owned()).await;
        let finished = calls["calls"].as_array().is_some_and(|calls| {
            calls.len() == 2 && calls.iter().all(|call| call["state"] == "finished")
        });
        let searchable = read("/api/search?q=parser".to_owned()).await["indexed_calls"] == 2;
        if finished && searchable {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let call = calls["calls"][0]["id"].as_str().unwrap().to_owned();
    let recording = calls["calls"][0]["recording"].as_str().unwrap().to_owned();
    let turn = calls["calls"][0]["turn"].as_str().unwrap().to_owned();
    let answers = [
        ("calls", calls.clone()),
        ("call", read(format!("/api/calls/{call}")).await),
        (
            "call_activity",
            read(format!("/api/calls/{call}/activity")).await,
        ),
        ("recordings", read("/api/recordings".to_owned()).await),
        (
            "recording_calls",
            read(format!("/api/recordings/{recording}/calls")).await,
        ),
        (
            "turns",
            read(format!("/api/recordings/{recording}/turns")).await,
        ),
        ("turn", read(format!("/api/turns/{turn}")).await),
        (
            "report",
            read(format!("/api/recordings/{recording}/report")).await,
        ),
        ("usage", read("/api/usage".to_owned()).await),
        ("search", read("/api/search?q=parser".to_owned()).await),
        ("exposures", read("/api/exposures".to_owned()).await),
        ("work", {
            let queued = client()
                .post(format!("{base}/control/v1/work"))
                .bearer_auth(TestGateway::CONTROL_TOKEN)
                .header("content-type", "application/json")
                .body(work_order().to_string())
                .send()
                .await
                .unwrap();
            assert_eq!(queued.status(), http::StatusCode::CREATED);
            let mut listed = serde_json::Value::Null;
            for _ in 0..200 {
                listed = read("/api/work".to_owned()).await;
                if listed["work"]
                    .as_array()
                    .is_some_and(|work| !work.is_empty())
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            listed
        }),
        ("project_release", {
            // Ren's and acme-web's, registered; none until the factory
            // writes one, then as it wrote it.
            for (name, repository, factory) in [
                ("ren", "/home/op/ren", "ren"),
                ("acme-web", "/home/op/dev/acme-web", "contract"),
            ] {
                let registered = client()
                    .post(format!("{base}/control/v1/projects"))
                    .bearer_auth(TestGateway::CONTROL_TOKEN)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({ "name": name, "repository": repository, "factory": factory })
                            .to_string(),
                    )
                    .send()
                    .await
                    .unwrap();
                assert_eq!(registered.status(), http::StatusCode::CREATED);
            }
            for _ in 0..200 {
                if read("/api/projects".to_owned()).await["projects"]
                    .as_array()
                    .is_some_and(|projects| projects.len() == 2)
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            assert_eq!(
                read("/api/projects/ren/release".to_owned()).await["release"],
                serde_json::Value::Null
            );
            std::fs::write(
                dir.path().join(ren_release::Release::FILE),
                serde_json::to_vec(&release()).unwrap(),
            )
            .unwrap();
            let shots = dir.path().join(ren_release::ShotName::FOLDER);
            std::fs::create_dir_all(&shots).unwrap();
            std::fs::write(shots.join("overview.png"), b"\x89PNG").unwrap();
            let project = dir
                .path()
                .join(ren_release::Release::PROJECTS)
                .join("acme-web");
            std::fs::create_dir_all(&project).unwrap();
            std::fs::write(
                project.join(ren_release::Release::FILE),
                serde_json::to_vec(&release()).unwrap(),
            )
            .unwrap();
            read("/api/projects/ren/release".to_owned()).await
        }),
        ("project_pulls", {
            // Registered above; its repository is watched on `acme/ren`.
            let answer = serde_json::json!({ "data": { "repository": {
                "defaultBranchRef": { "name": "main", "target": { "oid": "1111111111111111111111111111111111111111" } },
                "pullRequests": { "pageInfo": { "hasNextPage": false }, "nodes": [{
                    "number": 12,
                    "title": "feat: a landing page",
                    "body": "Work-Item: 00000000-0000-7000-8000-000000000001",
                    "url": "https://github.com/acme/ren/pull/12",
                    "isDraft": false,
                    "author": { "login": "acme-bot" },
                    "headRefName": "landing",
                    "headRefOid": "2222222222222222222222222222222222222222",
                    "baseRefName": "main",
                    "mergeStateStatus": "BEHIND",
                    "reviewDecision": null,
                    "commits": { "nodes": [{ "commit": { "statusCheckRollup": {
                        "state": "PENDING",
                        "contexts": { "pageInfo": { "hasNextPage": false }, "nodes": [
                            { "__typename": "CheckRun", "name": "CI", "status": "COMPLETED", "conclusion": "SUCCESS" },
                            { "__typename": "CheckRun", "name": "Compatibility", "status": "IN_PROGRESS", "conclusion": null }
                        ] }
                    } } }] }
                }] }
            } } });
            let snapshot = ren_github::Watch::new("acme/ren".parse().unwrap())
                .read(answer.to_string().as_bytes())
                .unwrap();
            let at = std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_millis(1_800_000_000_000);
            gateway.pulls().record(
                &"acme/ren".parse().unwrap(),
                &ren_github::Observation::Waiting.seen(ren_github::Sighting { snapshot, at }),
            );
            read("/api/projects/ren/pulls".to_owned()).await
        }),
        ("project_previews", {
            // Ren's previews, as the factory wrote them: a pull request's
            // running, and a commit asked for whose build failed.
            let wanted = [
                ren_gateway::instance::Wanted {
                    source: ren_gateway::instance::Source::Pull("12".parse().unwrap()),
                    commit: "2222222222222222222222222222222222222222".parse().unwrap(),
                },
                ren_gateway::instance::Wanted {
                    source: ren_gateway::instance::Source::Commit(
                        "3333333333333333333333333333333333333333".parse().unwrap(),
                    ),
                    commit: "3333333333333333333333333333333333333333".parse().unwrap(),
                },
            ];
            use ren_gateway::instance::{Action, Event, Instances, Reach};
            let (mut previews, actions) = Instances::default()
                .step(Event::Wanted {
                    wanted: wanted.to_vec(),
                    capacity: ren_release::PreviewCapacity::DEFAULT,
                })
                .unwrap();
            for (action, outcome) in actions.into_iter().zip([None, Some("the build failed")]) {
                let Action::Start { slot, commit, .. } = action else {
                    unreachable!("only starts from nothing")
                };
                let event = match outcome {
                    None => Event::Started {
                        slot,
                        commit,
                        reach: Reach::Tailnet,
                    },
                    Some(detail) => Event::Failed {
                        slot,
                        commit,
                        detail: detail.into(),
                    },
                };
                previews = previews.step(event).unwrap().0;
            }
            std::fs::write(
                dir.path().join(ren_gateway::instance::Instances::FILE),
                serde_json::to_vec(&previews).unwrap(),
            )
            .unwrap();
            let mut requests = ren_gateway::instance::CommitAsks::default();
            requests.ask("3333333333333333333333333333333333333333".parse().unwrap());
            std::fs::write(
                dir.path().join(ren_gateway::instance::CommitAsks::FILE),
                serde_json::to_vec(&requests).unwrap(),
            )
            .unwrap();
            read("/api/projects/ren/previews".to_owned()).await
        }),
        ("projects", read("/api/projects".to_owned()).await),
    ];
    gateway.stop().await;

    let update = std::env::var_os("REN_UPDATE_FIXTURES").is_some();
    let mut changed = Vec::new();
    for (name, answer) in answers {
        let path = fixtures().join(format!("{name}.json"));
        let answer = normalized(answer);
        if update {
            std::fs::create_dir_all(fixtures()).unwrap();
            let text = format!("{}\n", serde_json::to_string_pretty(&answer).unwrap());
            std::fs::write(&path, text).unwrap();
        } else {
            // Compared as JSON values, so the UI's formatter may lay the
            // files out as it likes.
            let pinned = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            if pinned.as_ref() != Some(&answer) {
                changed.push(name);
            }
        }
    }
    assert!(
        changed.is_empty(),
        "the read API's JSON changed for {changed:?}; if intended, run with REN_UPDATE_FIXTURES=1 and update the UI's decoders"
    );
}

/// Each pinned answer reads back, through `ReadBody::decode`, as the
/// gateway's typed body for that read, and serializes to the same JSON: the
/// CLI and MCP read those types from a gateway too.
#[test]
fn each_pinned_answer_decodes_as_its_read_and_round_trips() {
    use ren_recorder::id::{CallId, RecordingId, TurnId};
    use ren_store::query::{CallQuery, PageLimit, Shelf, TimeRange};
    use ren_store::read::ReadRequest;
    use ren_store::search::{ExposureQuery, SearchQuery};
    let id = uuid::Uuid::from_u128(1);
    let recording = RecordingId::restore(id);
    let reads = [
        ("calls", ReadRequest::Calls(CallQuery::default())),
        ("call", ReadRequest::Call(CallId::restore(id))),
        (
            "call_activity",
            ReadRequest::CallActivity(CallId::restore(id)),
        ),
        (
            "recordings",
            ReadRequest::Recordings {
                limit: PageLimit::DEFAULT,
                shelf: Shelf::Current,
                project: None,
            },
        ),
        (
            "recording_calls",
            ReadRequest::RecordingCalls {
                recording,
                limit: PageLimit::DEFAULT,
                after: None,
            },
        ),
        (
            "turns",
            ReadRequest::Turns {
                recording,
                limit: PageLimit::DEFAULT,
            },
        ),
        ("turn", ReadRequest::Turn(TurnId::restore(id))),
        ("report", ReadRequest::Report(recording)),
        (
            "usage",
            ReadRequest::Usage {
                range: TimeRange::default(),
                scope: ren_store::query::UsageScope::Everything,
            },
        ),
        (
            "search",
            ReadRequest::Search(SearchQuery {
                terms: "parser".parse::<ren_store::search::SearchTerms>().unwrap(),
                recording: None,
                project: None,
                client: None,
                field: None,
                time: TimeRange::default(),
                limit: PageLimit::DEFAULT,
            }),
        ),
        (
            "exposures",
            ReadRequest::Exposures(ExposureQuery {
                recording: None,
                project: None,
                time: TimeRange::default(),
                limit: PageLimit::DEFAULT,
            }),
        ),
    ];
    for (name, read) in reads {
        let mut pinned: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fixtures().join(format!("{name}.json"))).unwrap(),
        )
        .unwrap();
        let decoded = ren_gateway::api::read::ReadBody::decode(&read, pinned.clone())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        pinned.as_object_mut().unwrap().remove("api_version");
        assert_eq!(serde_json::to_value(&decoded).unwrap(), pinned, "{name}");
    }
}
