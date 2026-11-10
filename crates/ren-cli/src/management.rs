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
use crate::tail::LiveUpdates;
use ren_gateway::ApiRead;
use ren_harness::{endpoint::GatewayEndpoint, host::HostEnv};
use ren_protocol::direction::Direction;
use ren_recorder::id::{CallId, TurnId};
use ren_store::Reader;
use ren_store::activity::{ActivityBudget, CallActivity, TurnActivity};
use ren_store::query::{CallQuery, PageLimit};
use ren_store::read::ReadRequest;
use ren_store::view::{CallPage, SessionView};
use reqwest::{Method, StatusCode};
use serde::{Deserialize, de::DeserializeOwned};
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
/// Both answer a [`ReadRequest`] with the same JSON, since the API's body is
/// the reader's response.
pub enum History {
    Local(Reader),
    Remote(Management),
}

impl History {
    pub async fn serve(&self, request: &ReadRequest) -> Result<Value, ManagementError> {
        match self {
            Self::Local(reader) => Ok(serde_json::to_value(reader.serve(request).await?)?),
            Self::Remote(gateway) => gateway.serve(request).await,
        }
    }

    /// Calls, newest first, as `query` narrows them.
    pub async fn calls(&self, query: &CallQuery) -> Result<CallPage, ManagementError> {
        match self {
            Self::Local(reader) => Ok(reader.calls(query).await?),
            Self::Remote(gateway) => gateway.calls(query).await,
        }
    }

    /// Sessions, most recently seen first.
    pub async fn sessions(&self, limit: PageLimit) -> Result<Vec<SessionView>, ManagementError> {
        match self {
            Self::Local(reader) => Ok(reader.sessions(limit).await?),
            Self::Remote(gateway) => gateway.sessions(limit).await,
        }
    }

    /// One turn's activity, as much of it as the read API shows.
    pub async fn turn(&self, turn: TurnId) -> Result<TurnActivity, ManagementError> {
        match self {
            Self::Local(reader) => Ok(reader.turn_activity(turn, ActivityBudget::DEFAULT).await?),
            Self::Remote(gateway) => gateway.turn(turn).await,
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
    #[error("the export ended after {received} of {expected} bytes")]
    Truncated { received: u64, expected: u64 },
    #[error("cannot write the export")]
    Write(#[source] std::io::Error),
}

/// An API error code (`{api_version, error}`), parsed once; codes this CLI
/// does not know are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NotFound,
    Busy,
    Unavailable,
    InvalidQuery,
    /// A control request without a token or a Tailscale identity.
    Unauthorized,
    /// A Tailscale login the daemon does not list.
    Forbidden,
    Other(String),
}

impl From<&str> for Refusal {
    fn from(code: &str) -> Self {
        match code {
            "not_found" => Self::NotFound,
            "busy" => Self::Busy,
            "unavailable" => Self::Unavailable,
            "invalid_query" => Self::InvalidQuery,
            "unauthorized" => Self::Unauthorized,
            "forbidden" => Self::Forbidden,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// What a refusal means to someone running the CLI.
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("the gateway has no such record, or no complete captured body"),
            Self::Busy => f.write_str("the gateway is busy; try again"),
            Self::Unavailable => f.write_str("the gateway's storage is unavailable"),
            Self::InvalidQuery => f.write_str("the gateway refused the query"),
            Self::Unauthorized => f.write_str(
                "the gateway refused the control request without a Tailscale identity: reach it through Tailscale Serve, or run on the daemon's host without a gateway",
            ),
            Self::Forbidden => f.write_str(
                "the gateway refused this Tailscale login: add it to [control] tailscale_logins in the daemon's ren.toml and restart the daemon",
            ),
            Self::Other(code) => write!(f, "the gateway refused the request: {code}"),
        }
    }
}

/// A control API answer, printed whatever its status.
pub struct ControlAnswer {
    pub status: StatusCode,
    pub body: Value,
}

/// Whether the command line names this host's daemon (`--config` or
/// `--data-dir`).
#[derive(Clone, Copy)]
pub enum HostFlags {
    Named,
    Absent,
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
            return Err(Self::error(status, &body));
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
    pub async fn live(
        &self,
        after: Option<u64>,
        updates: LiveUpdates,
    ) -> Result<reqwest::Response, ManagementError> {
        let mut url = self.endpoint.url("/api/live");
        if let Some(after) = after {
            url.query_pairs_mut()
                .append_pair("after", &after.to_string());
        }
        match updates {
            LiveUpdates::Observations => {}
            LiveUpdates::WithProgress => {
                url.query_pairs_mut().append_pair("progress", "true");
            }
        }
        let response = self
            .send(self.http.get(url).header("accept", "text/event-stream"))
            .await?;
        let status = response.status();
        if status.is_success() {
            Ok(response)
        } else {
            let body = self.bytes(response).await?;
            Err(Self::error(status, &body))
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

    /// Sends a control request through the gateway, without a token.
    pub async fn control(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<ControlAnswer, ManagementError> {
        let mut request = self
            .http
            .request(method, self.endpoint.url(&format!("/control/v1/{path}")));
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let response = self.send(request).await?;
        let status = response.status();
        let body = self.bytes(response).await?;
        let body: Value =
            serde_json::from_slice(&body).map_err(|_| ManagementError::Shape { status })?;
        if status.is_success() {
            return Ok(ControlAnswer { status, body });
        }
        // Other refusals (a call not running, a backup in progress) are the
        // daemon's answer, printed as the local command prints them.
        match body.get("error").and_then(Value::as_str).map(Refusal::from) {
            Some(refusal @ (Refusal::Unauthorized | Refusal::Forbidden)) => {
                Err(ManagementError::Refused { status, refusal })
            }
            Some(
                Refusal::NotFound
                | Refusal::Busy
                | Refusal::Unavailable
                | Refusal::InvalidQuery
                | Refusal::Other(_),
            )
            | None => Ok(ControlAnswer { status, body }),
        }
    }

    /// One history read, through the read API: its JSON body.
    pub async fn serve(&self, request: &ReadRequest) -> Result<Value, ManagementError> {
        Ok(Value::Object(
            self.read(&ApiRead(request.clone()).uri()).await?,
        ))
    }

    /// Calls, newest first, as `query` narrows them.
    pub async fn calls(&self, query: &CallQuery) -> Result<CallPage, ManagementError> {
        self.read_as(&ReadRequest::Calls(query.clone())).await
    }

    /// Sessions, most recently seen first.
    pub async fn sessions(&self, limit: PageLimit) -> Result<Vec<SessionView>, ManagementError> {
        /// The read API's body for a session listing.
        #[derive(Deserialize)]
        struct Sessions {
            sessions: Vec<SessionView>,
        }
        self.read_as::<Sessions>(&ReadRequest::Sessions(limit))
            .await
            .map(|page| page.sessions)
    }

    /// One turn's activity, as much of it as the read API shows.
    pub async fn turn(&self, turn: TurnId) -> Result<TurnActivity, ManagementError> {
        self.read_as(&ReadRequest::Turn(turn)).await
    }

    /// One call's activity, as much of it as the read API shows.
    pub async fn call_activity(&self, call: CallId) -> Result<CallActivity, ManagementError> {
        self.read_as(&ReadRequest::CallActivity(call)).await
    }

    /// One history read, through the read API, parsed into its view.
    async fn read_as<T: DeserializeOwned>(
        &self,
        request: &ReadRequest,
    ) -> Result<T, ManagementError> {
        let body = self.read(&ApiRead(request.clone()).uri()).await?;
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
            return Err(Self::error(status, &body));
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

    /// An error answer: its `error` code, or its status when it has none.
    fn error(status: StatusCode, body: &[u8]) -> ManagementError {
        match serde_json::from_slice::<Value>(body)
            .ok()
            .as_ref()
            .and_then(|value| value.get("error"))
            .and_then(Value::as_str)
        {
            Some(code) => ManagementError::Refused {
                status,
                refusal: Refusal::from(code),
            },
            None => ManagementError::Shape { status },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_answers_keep_their_code() {
        let refused = Management::error(
            StatusCode::NOT_FOUND,
            br#"{"api_version":1,"error":"not_found"}"#,
        );
        assert!(matches!(
            refused,
            ManagementError::Refused {
                refusal: Refusal::NotFound,
                ..
            }
        ));
        let unknown = Management::error(
            StatusCode::CONFLICT,
            br#"{"api_version":1,"error":"backup_running"}"#,
        );
        assert!(matches!(
            unknown,
            ManagementError::Refused { refusal: Refusal::Other(ref code), .. } if code == "backup_running"
        ));
        assert!(matches!(
            Management::error(StatusCode::BAD_GATEWAY, b"<html>"),
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
        let session = "00000000-0000-0000-0000-000000000003";
        let router = Router::new()
            .route(
                "/api/sessions",
                get(move || async move {
                    Json(json!({
                        "api_version": 1,
                        "sessions": [{
                            "id": session,
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
            .route(
                "/api/turns/{turn}",
                get(move || async move {
                    Json(json!({
                        "api_version": 1,
                        "turn": "00000000-0000-0000-0000-000000000004",
                        "session": session,
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
        let gateway = Management::new(GatewayEndpoint::parse(&root).unwrap()).unwrap();
        tokio::select! {
            served = axum::serve(listener, router).into_future() => panic!("{served:?}"),
            answer = reads(gateway) => answer,
        }
    }

    #[tokio::test]
    async fn a_gateway_answers_reads_as_views() {
        let (sessions, calls, turn) = against_gateway(|gateway| async move {
            let history = History::Remote(gateway);
            let sessions = history.sessions(PageLimit::new(10).unwrap()).await.unwrap();
            let calls = history.calls(&CallQuery::default()).await.unwrap();
            let turn = history
                .turn("00000000-0000-0000-0000-000000000004".parse().unwrap())
                .await
                .unwrap();
            (sessions, calls, turn)
        })
        .await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].source.label(), "a_newer_source");
        assert!(calls.calls.is_empty() && calls.next.is_none());
        assert_eq!(turn.client.unwrap().label(), "a_newer_client");
    }

    #[tokio::test]
    async fn a_live_feed_asks_for_progress_only_when_told() {
        let queries = against_gateway(|gateway| async move {
            let mut queries = Vec::new();
            for (after, updates) in [
                (None, LiveUpdates::Observations),
                (Some(7), LiveUpdates::Observations),
                (Some(7), LiveUpdates::WithProgress),
            ] {
                let response = gateway.live(after, updates).await.unwrap();
                queries.push(response.text().await.unwrap());
            }
            queries
        })
        .await;
        assert_eq!(queries, ["", "after=7", "after=7&progress=true"]);
    }

    #[tokio::test]
    async fn an_answer_that_is_not_the_view_asked_for_is_refused() {
        // A session listing read as a page of calls.
        let wrong = against_gateway(|gateway| async move {
            gateway
                .read_as::<CallPage>(&ReadRequest::Sessions(PageLimit::new(10).unwrap()))
                .await
        })
        .await;
        assert!(
            matches!(wrong, Err(ManagementError::Answer(_))),
            "{wrong:?}"
        );
    }
}
