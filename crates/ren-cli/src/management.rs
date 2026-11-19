//! What history and control commands act on: this host's daemon, or a
//! gateway named by `--gateway`, `REN_GATEWAY`, or `client.toml`.
//!
//! Locally, history is read from storage and controls use the daemon's
//! token. Through a gateway, both go over its APIs: history through the
//! read-only management API (whose views are the ones the local reader
//! returns, so output matches), and controls through `/control/v1`, which
//! accepts a gateway request only with a Tailscale identity the daemon lists
//! in `control.tailscale_logins`.

use crate::client::{ClientError, GatewayChoice};
use ren_gateway::api::ErrorCode;
use ren_gateway::api::control::request::{ControlFeed, ControlMethod, ControlRequest};
use ren_gateway::api::read::ReadBody;
use ren_gateway::api::{projects::ProjectsBody, release::ReleaseBody};
use ren_gateway::{ApiRead, ControlRefusal, ControlToken};
use ren_harness::{endpoint::GatewayEndpoint, host::HostEnv};
use ren_protocol::direction::Direction;
use ren_protocol::label::{Labeled, Stored};
use ren_recorder::id::CallId;
use ren_recorder::project::ProjectName;
use ren_store::Reader;
use ren_store::read::ReadRequest;
use reqwest::{Method, StatusCode};
use serde_json::{Map, Value};
use std::time::Duration;

/// Where a history or control command acts.
pub enum Target {
    /// This host's daemon: its storage and its control token.
    Local,
    /// A gateway's management and control APIs.
    Remote(Management),
}

/// Where history is read: this host's storage, or a gateway's read API.
/// Both answer a [`ReadRequest`] with the gateway's [`ReadBody`]: this host's
/// reader's response converted as the API converts it, or the API's body
/// decoded by the request.
pub enum History {
    Local(Reader),
    Remote(Management),
}

impl History {
    pub async fn serve(&self, request: &ReadRequest) -> Result<ReadBody, ManagementError> {
        match self {
            Self::Local(reader) => Ok(reader.serve(request).await?.into()),
            Self::Remote(gateway) => gateway.serve(request).await,
        }
    }
}

/// A client for one gateway's management and control APIs.
pub struct Management {
    http: reqwest::Client,
    endpoint: GatewayEndpoint,
}

/// Why a gateway request failed. Messages name the gateway's root URL,
/// which never carries credentials.
#[derive(Debug, thiserror::Error)]
pub enum ManagementError {
    #[error(transparent)]
    Choice(#[from] ClientError),
    #[error("cannot initialize the HTTP client")]
    Client(#[source] reqwest::Error),
    #[error("cannot reach the gateway at {root}")]
    Unreachable {
        root: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("reading this host's history failed")]
    Store(#[from] ren_store::StoreError),
    #[error("cannot encode a history answer")]
    Json(#[from] serde_json::Error),
    #[error("the gateway's history answer does not match this CLI's views")]
    Answer(#[source] serde_json::Error),
    #[error("the gateway's answer is not management API JSON ({status})")]
    Shape { status: StatusCode },
    #[error(
        "the gateway speaks management API version {found}; this CLI understands {}",
        Management::API_VERSION
    )]
    Version { found: String },
    #[error("{refusal} ({status})")]
    Refused {
        status: StatusCode,
        refusal: Refusal,
    },
    #[error("control request failed with {status}")]
    Failed { status: StatusCode },
    #[error("the export ended after {received} of {expected} bytes")]
    Truncated { received: u64, expected: u64 },
    #[error("cannot write the export")]
    Write(#[source] std::io::Error),
}

/// An API error code (`{api_version, error}`), parsed once into the read
/// API's or the control API's vocabulary; codes this CLI does not know are
/// kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Read(Stored<ErrorCode>),
    Control(Stored<ControlRefusal>),
}

/// What a refusal means to someone running the CLI.
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(Stored::Known(ErrorCode::NotFound)) => f.write_str("the gateway has no such record, or no complete captured body"),
            Self::Read(Stored::Known(ErrorCode::Busy)) => f.write_str("the gateway is busy; try again"),
            Self::Read(Stored::Known(ErrorCode::Unavailable)) => f.write_str("the gateway's storage is unavailable"),
            Self::Read(Stored::Known(ErrorCode::InvalidQuery)) => f.write_str("the gateway refused the query"),
            Self::Read(Stored::Known(ErrorCode::IdentityRefused)) => f.write_str("the gateway refused this caller's identity"),
            Self::Control(Stored::Known(ControlRefusal::Unauthorized)) => f.write_str(
                "the gateway refused the control request without a Tailscale identity: reach it through Tailscale Serve, or run on the daemon's host without a gateway",
            ),
            Self::Control(Stored::Known(ControlRefusal::Forbidden)) => f.write_str(
                "the gateway refused this Tailscale login: add it to [control] tailscale_logins in the daemon's ren.toml and restart the daemon",
            ),
            Self::Read(Stored::Unknown(code)) => write!(f, "the gateway refused the request: {}", code.as_str()),
            Self::Control(code) => write!(f, "the daemon refused the request: {code}"),
        }
    }
}

/// How a control request is authorized: the daemon's token, read on its
/// host, or the Tailscale identity Serve adds on the way through a gateway.
pub enum ControlCredential {
    Token(ControlToken),
    Tailnet,
}

/// A control API answer, printed whatever its status.
pub struct ControlAnswer {
    pub status: StatusCode,
    pub body: Value,
}

impl ControlAnswer {
    /// Print the answer; a refusal is an error after it.
    pub fn report(&self) -> Result<std::process::ExitCode, ManagementError> {
        println!("{}", serde_json::to_string_pretty(&self.body)?);
        if self.status.is_success() {
            Ok(std::process::ExitCode::SUCCESS)
        } else {
            Err(ManagementError::Failed {
                status: self.status,
            })
        }
    }
}

/// Whether the command line names this host's daemon (`--config` or
/// `--data-dir`).
#[derive(Clone, Copy)]
pub enum HostFlags {
    Named,
    Absent,
}

impl ControlCredential {
    fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Self::Token(token) => request.bearer_auth(token.expose()),
            Self::Tailnet => request,
        }
    }
}

impl Target {
    /// `--gateway`; else this host when `--config` or `--data-dir` names it;
    /// else `REN_GATEWAY` or `client.toml`; else this host. Explicit flags
    /// win over saved and environment settings.
    pub fn select(
        flag: Option<&str>,
        host: HostFlags,
        env: &HostEnv,
    ) -> Result<Self, ManagementError> {
        match (flag, host) {
            (None, HostFlags::Named) => return Ok(Self::Local),
            (Some(_), HostFlags::Named | HostFlags::Absent) | (None, HostFlags::Absent) => {}
        }
        Ok(match GatewayChoice::select(flag, env)? {
            GatewayChoice::Selected { endpoint, .. } => Self::Remote(Management::new(endpoint)?),
            GatewayChoice::LocalListener => Self::Local,
        })
    }
}

impl Management {
    /// The read API version this CLI parses.
    const API_VERSION: u64 = 1;

    pub fn new(endpoint: GatewayEndpoint) -> Result<Self, ManagementError> {
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30))
            .build()
            .map_err(ManagementError::Client)?;
        Ok(Self { http, endpoint })
    }

    /// `GET /api/calls/{id}/body/{direction}`: a bounded preview of a body.
    pub async fn preview(
        &self,
        call: CallId,
        direction: Direction,
    ) -> Result<Value, ManagementError> {
        let path = format!("/api/calls/{call}/body/{direction}");
        Ok(Value::Object(self.read(&path).await?))
    }

    /// Writes a complete captured body through `write`, and returns its
    /// length. A body the gateway did not store whole is refused.
    pub async fn export(
        &self,
        call: CallId,
        direction: Direction,
        mut write: impl FnMut(&[u8]) -> std::io::Result<()>,
    ) -> Result<u64, ManagementError> {
        let path = format!("/api/calls/{call}/body/{direction}/export");
        let mut response = self.send(self.http.get(self.endpoint.url(&path))).await?;
        let status = response.status();
        if !status.is_success() {
            let body = self.bytes(response).await?;
            return Err(Self::error(status, &body, Refusal::Read));
        }
        let expected = response.content_length();
        let mut received = 0_u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|source| self.unreachable(source))?
        {
            write(&chunk).map_err(ManagementError::Write)?;
            received = received.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        }
        match expected {
            Some(expected) if expected != received => {
                Err(ManagementError::Truncated { received, expected })
            }
            Some(_) | None => Ok(received),
        }
    }

    /// `GET /api/live`: the response that streams the live feed, from after
    /// `after` when given, else from now. Read it with [`Self::chunk`].
    pub async fn live(&self, after: Option<u64>) -> Result<reqwest::Response, ManagementError> {
        let mut url = self.endpoint.url("/api/live");
        if let Some(after) = after {
            url.query_pairs_mut()
                .append_pair("after", &after.to_string());
        }
        let response = self
            .send(self.http.get(url).header("accept", "text/event-stream"))
            .await?;
        let status = response.status();
        if status.is_success() {
            Ok(response)
        } else {
            let body = self.bytes(response).await?;
            Err(Self::error(status, &body, Refusal::Read))
        }
    }

    /// The next bytes of a streamed response; `None` when it ended cleanly.
    pub async fn chunk(
        &self,
        response: &mut reqwest::Response,
    ) -> Result<Option<Vec<u8>>, ManagementError> {
        response
            .chunk()
            .await
            .map(|chunk| chunk.map(Vec::from))
            .map_err(|source| self.unreachable(source))
    }

    /// The root URL requests go to.
    pub fn root(&self) -> &str {
        self.endpoint.root()
    }

    /// Send a control request, authorized by `credential`. Through a
    /// gateway, a refused identity is an error; every other answer is
    /// returned to be printed, as the daemon gave it.
    pub async fn control(
        &self,
        request: &ControlRequest,
        credential: &ControlCredential,
    ) -> Result<ControlAnswer, ManagementError> {
        let method = match request.method() {
            ControlMethod::Get => Method::GET,
            ControlMethod::Post => Method::POST,
        };
        let url = self
            .endpoint
            .url(&format!("/control/v1/{}", request.path()));
        let mut sending = credential.authorize(self.http.request(method, url));
        if let Some(body) = request.body()? {
            sending = sending
                .header("content-type", "application/json")
                .body(body);
        }
        let response = self.send(sending).await?;
        let status = response.status();
        let body = self.bytes(response).await?;
        let body: Value =
            serde_json::from_slice(&body).map_err(|_| ManagementError::Shape { status })?;
        let refusal = body
            .get("error")
            .and_then(Value::as_str)
            .map(Stored::<ControlRefusal>::from_label);
        match (credential, refusal) {
            (
                ControlCredential::Tailnet,
                Some(
                    refusal @ Stored::Known(
                        ControlRefusal::Unauthorized | ControlRefusal::Forbidden,
                    ),
                ),
            ) => Err(ManagementError::Refused {
                status,
                refusal: Refusal::Control(refusal),
            }),
            (ControlCredential::Tailnet | ControlCredential::Token(_), _) => {
                Ok(ControlAnswer { status, body })
            }
        }
    }

    /// A control event stream (server-sent events), authorized by
    /// `credential`. Read it with [`Self::chunk`].
    pub async fn control_stream(
        &self,
        feed: ControlFeed,
        credential: &ControlCredential,
    ) -> Result<reqwest::Response, ManagementError> {
        let url = self.endpoint.url(&format!("/control/v1/{}", feed.path()));
        let request =
            credential.authorize(self.http.get(url).header("accept", "text/event-stream"));
        let response = self.send(request).await?;
        let status = response.status();
        if status.is_success() {
            Ok(response)
        } else {
            let body = self.bytes(response).await?;
            Err(Self::error(status, &body, Refusal::Control))
        }
    }

    /// One history read, through the read API, decoded as the read's
    /// response.
    pub async fn serve(&self, request: &ReadRequest) -> Result<ReadBody, ManagementError> {
        let body = self.read(&ApiRead(request.clone()).uri()).await?;
        ReadBody::decode(request, Value::Object(body)).map_err(ManagementError::Answer)
    }

    /// Every registered project, `/api/projects`.
    pub async fn projects(&self) -> Result<ProjectsBody, ManagementError> {
        let body = self.read("/api/projects").await?;
        serde_json::from_value(Value::Object(body)).map_err(ManagementError::Answer)
    }

    /// Where `project`'s release stands, `/api/projects/{name}/release`.
    pub async fn release(&self, project: &ProjectName) -> Result<ReleaseBody, ManagementError> {
        let body = self
            .read(&format!("/api/projects/{project}/release"))
            .await?;
        serde_json::from_value(Value::Object(body)).map_err(ManagementError::Answer)
    }

    /// A successful read's body, without `api_version`. `uri` is a path and
    /// query string.
    async fn read(&self, uri: &str) -> Result<Map<String, Value>, ManagementError> {
        let (path, query) = uri.split_once('?').unwrap_or((uri, ""));
        let mut url = self.endpoint.url(path);
        if !query.is_empty() {
            url.set_query(Some(query));
        }
        let response = self.send(self.http.get(url)).await?;
        let status = response.status();
        let body = self.bytes(response).await?;
        if !status.is_success() {
            return Err(Self::error(status, &body, Refusal::Read));
        }
        let Ok(Value::Object(mut map)) = serde_json::from_slice::<Value>(&body) else {
            return Err(ManagementError::Shape { status });
        };
        match map.shift_remove("api_version") {
            Some(version) if version.as_u64() == Some(Self::API_VERSION) => Ok(map),
            Some(version) => Err(ManagementError::Version {
                found: version.to_string(),
            }),
            None => Err(ManagementError::Shape { status }),
        }
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, ManagementError> {
        request
            .send()
            .await
            .map_err(|source| self.unreachable(source))
    }

    async fn bytes(&self, response: reqwest::Response) -> Result<Vec<u8>, ManagementError> {
        response
            .bytes()
            .await
            .map(Vec::from)
            .map_err(|source| self.unreachable(source))
    }

    fn unreachable(&self, source: reqwest::Error) -> ManagementError {
        ManagementError::Unreachable {
            root: self.endpoint.root().to_owned(),
            source: source.without_url(),
        }
    }

    /// An error answer: its `error` code, read in the API's vocabulary, or
    /// its status when it has none.
    fn error<T: Labeled>(
        status: StatusCode,
        body: &[u8],
        vocabulary: impl FnOnce(Stored<T>) -> Refusal,
    ) -> ManagementError {
        match serde_json::from_slice::<Value>(body)
            .ok()
            .as_ref()
            .and_then(|value| value.get("error"))
            .and_then(Value::as_str)
        {
            Some(code) => ManagementError::Refused {
                status,
                refusal: vocabulary(Stored::from_label(code)),
            },
            None => ManagementError::Shape { status },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ren_gateway::api::read::Recordings;
    use ren_store::query::{CallQuery, PageLimit, Shelf};

    #[test]
    fn error_answers_keep_their_code_in_their_apis_vocabulary() {
        let refused = Management::error(
            StatusCode::NOT_FOUND,
            br#"{"api_version":1,"error":"not_found"}"#,
            Refusal::Read,
        );
        assert!(matches!(
            refused,
            ManagementError::Refused {
                refusal: Refusal::Read(Stored::Known(ErrorCode::NotFound)),
                ..
            }
        ));
        let control = Management::error(
            StatusCode::CONFLICT,
            br#"{"api_version":1,"error":"backup_running"}"#,
            Refusal::Control,
        );
        assert!(matches!(
            control,
            ManagementError::Refused {
                refusal: Refusal::Control(Stored::Known(ControlRefusal::BackupRunning)),
                ..
            }
        ));
        let unknown = Management::error(
            StatusCode::CONFLICT,
            br#"{"api_version":1,"error":"a_newer_code"}"#,
            Refusal::Read,
        );
        assert!(matches!(
            unknown,
            ManagementError::Refused { refusal: Refusal::Read(Stored::Unknown(ref code)), .. } if code.as_str() == "a_newer_code"
        ));
        assert!(matches!(
            Management::error(StatusCode::BAD_GATEWAY, b"<html>", Refusal::Read),
            ManagementError::Shape { .. }
        ));
    }

    /// A gateway's read API answering with fixed bodies, until `reads`
    /// finishes.
    async fn against_gateway<F, T>(reads: impl FnOnce(Management) -> F) -> T
    where
        F: Future<Output = T>,
    {
        use axum::{Json, Router, routing::get};
        use serde_json::json;
        let recording = "00000000-0000-0000-0000-000000000003";
        let router = Router::new()
            .route(
                "/api/recordings",
                get(move || async move {
                    Json(json!({
                        "api_version": 1,
                        "recordings": [{
                            "id": recording,
                            "provider": "openai",
                            "source": "a_newer_source",
                            "native_id": "native",
                            "first_seen": 1000,
                            "last_seen": 2000,
                            "call_count": 4,
                            "turn_count": 2,
                            "error_count": 1,
                            "tokens": { "input": 10, "output": null, "cache_read": 5 }
                        }]
                    }))
                }),
            )
            .route(
                "/api/live",
                get(|uri: axum::http::Uri| async move { uri.query().unwrap_or("").to_owned() }),
            )
            .route(
                "/api/calls",
                get(|| async { Json(json!({ "api_version": 1, "calls": [], "next": null })) }),
            )
            // Not a usage report: a page of calls.
            .route(
                "/api/usage",
                get(|| async { Json(json!({ "api_version": 1, "calls": [], "next": null })) }),
            )
            .route(
                "/api/turns/{turn}",
                get(move || async move {
                    Json(json!({
                        "api_version": 1,
                        "turn": "00000000-0000-0000-0000-000000000004",
                        "recording": recording,
                        "native_id": "prompt-1",
                        "client": "a_newer_client",
                        "files_read": [],
                        "files_written": [],
                        "commands": [],
                        "tool_calls": 0,
                        "failed_tool_calls": 0,
                        "unanswered_tool_calls": 0,
                        "findings": [],
                        "calls": [],
                        "omitted_calls": 0
                    }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let gateway = Management::new(root.parse::<GatewayEndpoint>().unwrap()).unwrap();
        tokio::select! {
            served = axum::serve(listener, router).into_future() => panic!("{served:?}"),
            answer = reads(gateway) => answer,
        }
    }

    #[tokio::test]
    async fn a_gateway_answers_reads_as_views() {
        let (recordings, calls, turn) = against_gateway(|gateway| async move {
            let history = History::Remote(gateway);
            let ReadBody::Recordings(Recordings { recordings }) = history
                .serve(&ReadRequest::Recordings {
                    limit: PageLimit::new(10).unwrap(),
                    shelf: Shelf::Current,
                    project: None,
                })
                .await
                .unwrap()
            else {
                panic!("a recording listing")
            };
            let ReadBody::Calls(calls) = history
                .serve(&ReadRequest::Calls(CallQuery::default()))
                .await
                .unwrap()
            else {
                panic!("a call page")
            };
            let ReadBody::Turn(turn) = history
                .serve(&ReadRequest::Turn(
                    "00000000-0000-0000-0000-000000000004".parse().unwrap(),
                ))
                .await
                .unwrap()
            else {
                panic!("a turn")
            };
            (recordings, calls, turn)
        })
        .await;
        assert_eq!(recordings.len(), 1);
        assert_eq!(recordings[0].source.label(), "a_newer_source");
        assert!(calls.calls.is_empty() && calls.next.is_none());
        assert_eq!(turn.client.unwrap().label(), "a_newer_client");
    }

    #[tokio::test]
    async fn a_live_feed_starts_after_the_position_given() {
        let queries = against_gateway(|gateway| async move {
            let mut queries = Vec::new();
            for after in [None, Some(7)] {
                let response = gateway.live(after).await.unwrap();
                queries.push(response.text().await.unwrap());
            }
            queries
        })
        .await;
        assert_eq!(queries, ["", "after=7"]);
    }

    #[tokio::test]
    async fn an_answer_that_is_not_the_reads_response_is_refused() {
        let wrong = against_gateway(|gateway| async move {
            gateway
                .serve(&ReadRequest::Usage {
                    range: ren_store::query::TimeRange::default(),
                    project: None,
                })
                .await
        })
        .await;
        assert!(
            matches!(wrong, Err(ManagementError::Answer(_))),
            "{wrong:?}"
        );
    }
}
