//! What history and control commands act on: this host's daemon, or a
//! gateway named by `--gateway`, `TGV_GATEWAY`, or `client.toml`.
//!
//! Locally, history is read from storage and controls use the daemon's
//! token. Through a gateway, both go over its APIs: history through the
//! read-only management API (whose views are the ones the local reader
//! returns, so output matches), and controls through `/control/v1`, which
//! accepts a gateway request only with a Tailscale identity the daemon lists
//! in `control.tailscale_logins`.

use crate::client::{ClientError, GatewayChoice};
use reqwest::{Method, StatusCode};
use serde_json::{Map, Value};
use std::time::Duration;
use tgv_harness::{endpoint::GatewayEndpoint, host::HostEnv};
use tgv_protocol::direction::Direction;
use tgv_recorder::id::{CallId, SessionId};
use tgv_store::query::{Cursor, PageLimit};

/// Where a history or control command acts.
pub enum Target {
    /// This host's daemon: its storage and its control token.
    Local,
    /// A gateway's management and control APIs.
    Remote(Management),
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
                "the gateway refused this Tailscale login: add it to [control] tailscale_logins in the daemon's tgv.toml and restart the daemon",
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
    /// else `TGV_GATEWAY` or `client.toml`; else this host. Explicit flags
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

    /// `GET /api/calls?limit=`: the calls, newest first.
    pub async fn calls(&self, limit: PageLimit) -> Result<Value, ManagementError> {
        let query = [("limit", limit.get().to_string())];
        Self::field(self.read("/api/calls", &query).await?, "calls")
    }

    /// `GET /api/sessions?limit=`: the sessions.
    pub async fn sessions(&self, limit: PageLimit) -> Result<Value, ManagementError> {
        let query = [("limit", limit.get().to_string())];
        Self::field(self.read("/api/sessions", &query).await?, "sessions")
    }

    /// `GET /api/sessions/{id}/calls`: one page, `{calls, next}`.
    pub async fn session_calls(
        &self,
        session: SessionId,
        limit: PageLimit,
        after: Option<&Cursor>,
    ) -> Result<Value, ManagementError> {
        let mut query = vec![("limit", limit.get().to_string())];
        query.extend(after.map(|cursor| ("after", cursor.to_string())));
        let path = format!("/api/sessions/{session}/calls");
        Ok(Value::Object(self.read(&path, &query).await?))
    }

    /// `GET /api/calls/{id}`: the call with its attempts and observations.
    pub async fn call(&self, call: CallId) -> Result<Value, ManagementError> {
        Ok(Value::Object(
            self.read(&format!("/api/calls/{call}"), &[]).await?,
        ))
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

    /// A successful read's body, without `api_version`.
    async fn read(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Map<String, Value>, ManagementError> {
        let mut url = self.endpoint.url(path);
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(name, value)| (*name, value.as_str())));
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

    fn field(mut map: Map<String, Value>, name: &str) -> Result<Value, ManagementError> {
        map.shift_remove(name).ok_or(ManagementError::Shape {
            status: StatusCode::OK,
        })
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
}
