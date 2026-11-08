//! The `Upstream` seam: how a forwarded request reaches a provider.
//!
//! The relay hands an [`UpstreamRequest`] whose headers already passed the
//! header policy, and receives a response head or a typed [`SendFailure`]. The
//! response body yields typed [`StreamFailure`]s. A WebSocket handshake goes
//! through [`Upstream::connect`] and yields an upgraded socket or the refusal.
//! Tests substitute a scripted upstream; the daemon uses [`ReqwestUpstream`].

use crate::attempt::{SendFailure, StreamFailure};
use crate::route::{BoundRoute, Route};
use axum::body::Body;
use bytes::Bytes;
use http_body::{Frame, SizeHint};
use http_body_util::{BodyExt, BodyStream, StreamBody, combinators::UnsyncBoxBody};
use reqwest::{Client, Url};
use std::{
    future::Future,
    net::IpAddr,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use sync_wrapper::SyncStream;
use tgv_protocol::route::{Api, Provider, Wire};
use tgv_recorder::routing::UpstreamId;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;

/// A provider endpoint. Implementations decide only how bytes travel; header
/// policy, capture, and lifecycle stay in the relay.
pub trait Upstream: Send + Sync + 'static {
    fn send(
        &self,
        request: UpstreamRequest,
    ) -> impl Future<Output = Result<http::Response<UpstreamBody>, SendFailure>> + Send;

    /// Open a WebSocket. The implementation adds the upgrade headers and
    /// checks the upstream's accept key; anything but `101` is a refusal.
    fn connect(
        &self,
        handshake: UpstreamHandshake,
    ) -> impl Future<Output = Result<Connected, SendFailure>> + Send;
}

/// A shared upstream, so a caller can keep a handle on it (a test reading what
/// a scripted upstream received).
impl<U: Upstream> Upstream for std::sync::Arc<U> {
    fn send(
        &self,
        request: UpstreamRequest,
    ) -> impl Future<Output = Result<http::Response<UpstreamBody>, SendFailure>> + Send {
        U::send(self, request)
    }

    fn connect(
        &self,
        handshake: UpstreamHandshake,
    ) -> impl Future<Output = Result<Connected, SendFailure>> + Send {
        U::connect(self, handshake)
    }
}

/// A WebSocket handshake to send. `headers` already passed the handshake
/// header policy; `key` is the client's `Sec-WebSocket-Key`, reused so the
/// upstream's accept key can be checked without minting randomness.
#[derive(Debug)]
pub struct UpstreamHandshake {
    pub upstream: UpstreamId,
    pub path: &'static str,
    pub query: Option<String>,
    pub headers: http::HeaderMap,
    pub key: http::HeaderValue,
}

/// The upstream's answer to a handshake.
pub enum Connected {
    /// `101 Switching Protocols`: the response head and the raw socket.
    Upgraded {
        status: http::StatusCode,
        headers: http::HeaderMap,
        socket: UpstreamSocket,
    },
    /// Any other response, relayed to the client as is.
    Refused(http::Response<UpstreamBody>),
}

impl std::fmt::Debug for Connected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Upgraded { status, .. } => f
                .debug_struct("Upgraded")
                .field("status", status)
                .finish_non_exhaustive(),
            Self::Refused(response) => f.debug_tuple("Refused").field(&response.status()).finish(),
        }
    }
}

/// The byte stream of an upgraded connection. WebSocket framing is the relay's.
pub struct UpstreamSocket(Pin<Box<dyn Socket>>);

/// What an upgraded connection must offer.
pub trait Socket: AsyncRead + AsyncWrite + Send + 'static {}

impl<T: AsyncRead + AsyncWrite + Send + 'static> Socket for T {}

impl UpstreamSocket {
    pub fn new(socket: impl Socket) -> Self {
        Self(Box::pin(socket))
    }
}

impl AsyncRead for UpstreamSocket {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.get_mut().0.as_mut().poll_read(cx, buf)
    }
}

impl AsyncWrite for UpstreamSocket {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.get_mut().0.as_mut().poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.get_mut().0.as_mut().poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.get_mut().0.as_mut().poll_shutdown(cx)
    }
}

/// What to send, and where, relative to the upstream's configured base.
#[derive(Debug)]
pub struct UpstreamRequest {
    pub upstream: UpstreamId,
    /// The native path below the provider's base URL.
    pub path: &'static str,
    /// The client's query, forwarded verbatim.
    pub query: Option<String>,
    pub method: http::Method,
    pub headers: http::HeaderMap,
    pub body: Body,
}

/// A response body whose errors say why the stream stopped.
pub struct UpstreamBody(UnsyncBoxBody<Bytes, StreamFailure>);

impl UpstreamBody {
    pub fn new<B>(body: B) -> Self
    where
        B: http_body::Body<Data = Bytes, Error = StreamFailure> + Send + 'static,
    {
        Self(UnsyncBoxBody::new(body))
    }

    pub fn empty() -> Self {
        Self::new(http_body_util::Empty::new().map_err(|never| match never {}))
    }

    pub fn full(bytes: impl Into<Bytes>) -> Self {
        Self::new(http_body_util::Full::new(bytes.into()).map_err(|never| match never {}))
    }
}

impl std::fmt::Debug for UpstreamBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UpstreamBody")
    }
}

impl http_body::Body for UpstreamBody {
    type Data = Bytes;
    type Error = StreamFailure;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, StreamFailure>>> {
        Pin::new(&mut self.get_mut().0).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.0.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.0.size_hint()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct UpstreamConfigError(&'static str);

/// A validated, credential-free base URL. Validation has no network effects.
#[derive(Clone)]
pub struct UpstreamUrl(Url);

impl std::fmt::Debug for UpstreamUrl {
    /// Validated URLs carry no credentials, queries, or fragments.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("UpstreamUrl")
            .field(&self.0.as_str())
            .finish()
    }
}

impl UpstreamUrl {
    pub fn parse(base: &str) -> Result<Self, UpstreamConfigError> {
        let base = Url::parse(base).map_err(|_| UpstreamConfigError("invalid upstream URL"))?;
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(UpstreamConfigError(
                "upstream URLs must not contain credentials, queries, or fragments",
            ));
        }
        let loopback = base.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if base.host_str().is_none()
            || !(base.scheme() == "https" || (base.scheme() == "http" && loopback))
        {
            return Err(UpstreamConfigError(
                "upstreams require HTTPS, except loopback HTTP for local testing",
            ));
        }
        Ok(Self(base))
    }

    fn request_url(&self, path: &str, query: Option<&str>) -> Url {
        let mut url = self.0.clone();
        url.set_path(&format!("{}{}", self.0.path().trim_end_matches('/'), path));
        url.set_query(query);
        url
    }
}

/// An operator's API key for an upstream. It never appears in `Debug`,
/// errors, logs, or records; it is read once from the host environment.
#[derive(Clone)]
pub struct OperatorKey(http::HeaderValue);

impl OperatorKey {
    /// `None` when the text cannot be a header value.
    pub fn new(key: &str) -> Option<Self> {
        let mut value = http::HeaderValue::from_str(key.trim()).ok()?;
        value.set_sensitive(true);
        Some(Self(value))
    }

    fn bearer(&self) -> Option<http::HeaderValue> {
        let mut value =
            http::HeaderValue::from_bytes(&[b"Bearer ".as_slice(), self.0.as_bytes()].concat())
                .ok()?;
        value.set_sensitive(true);
        Some(value)
    }
}

impl std::fmt::Debug for OperatorKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OperatorKey(<redacted>)")
    }
}

/// How an upstream entry authenticates.
#[derive(Debug, Clone)]
pub enum UpstreamAuth {
    /// The client's own credentials pass through; TGV never holds them.
    ClientOwned,
    OperatorKey(OperatorKey),
}

/// Request headers that carry client credentials or subscription-only
/// features, replaced when an operator key is used.
const CLIENT_CREDENTIALS: &[&str] = &["authorization", "x-api-key", "cookie", "chatgpt-account-id"];
/// The `anthropic-beta` token that subscription OAuth requires.
const OAUTH_BETA: &str = "oauth-2025-04-20";

impl UpstreamAuth {
    pub fn kind(&self) -> crate::policy::AuthKind {
        match self {
            Self::ClientOwned => crate::policy::AuthKind::ClientOwned,
            Self::OperatorKey(_) => crate::policy::AuthKind::OperatorKey,
        }
    }
}

/// Whether credentials were overridden for an attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthOverride {
    None,
    /// Client credentials replaced by the entry's operator key.
    OperatorKey,
}

/// One configured upstream.
#[derive(Debug, Clone)]
pub struct UpstreamEntry {
    pub id: UpstreamId,
    pub api: Api,
    pub url: UpstreamUrl,
    pub auth: UpstreamAuth,
}

impl UpstreamEntry {
    /// The protected credential override for this entry: an operator key
    /// replaces client credentials, in the header scheme of the entry's wire
    /// format, and the Anthropic subscription OAuth beta is dropped. Returns
    /// whether anything was overridden. Header values are never recorded.
    pub fn authorize(&self, headers: &mut http::HeaderMap) -> AuthOverride {
        let key = match &self.auth {
            UpstreamAuth::ClientOwned => return AuthOverride::None,
            UpstreamAuth::OperatorKey(key) => key,
        };
        for name in CLIENT_CREDENTIALS {
            headers.remove(*name);
        }
        match self.api.wire() {
            Wire::AnthropicMessages => {
                let betas: Vec<String> = headers
                    .get_all("anthropic-beta")
                    .iter()
                    .filter_map(|value| value.to_str().ok())
                    .flat_map(|value| value.split(','))
                    .map(str::trim)
                    .filter(|beta| !beta.is_empty() && *beta != OAUTH_BETA)
                    .map(str::to_owned)
                    .collect();
                headers.remove("anthropic-beta");
                if let Ok(value) = http::HeaderValue::from_str(&betas.join(","))
                    && !betas.is_empty()
                {
                    headers.insert("anthropic-beta", value);
                }
                headers.insert("x-api-key", key.0.clone());
            }
            Wire::OpenAiResponses => {
                if let Some(bearer) = key.bearer() {
                    headers.insert(http::header::AUTHORIZATION, bearer);
                }
            }
        }
        AuthOverride::OperatorKey
    }
}

/// Every upstream entry. Each provider has a client-owned native entry named
/// after it, always present; operator-key entries are added by configuration.
#[derive(Debug, Clone)]
pub struct UpstreamTable {
    anthropic: UpstreamEntry,
    codex: UpstreamEntry,
    openai: UpstreamEntry,
    configured: Vec<UpstreamEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpstreamTableError {
    #[error("upstream id {0} is used twice (provider names are reserved)")]
    Duplicate(String),
}

impl UpstreamTable {
    /// The three native entries, then `extra`.
    pub fn new(
        anthropic: UpstreamUrl,
        codex: UpstreamUrl,
        openai: UpstreamUrl,
        extra: Vec<UpstreamEntry>,
    ) -> Result<Self, UpstreamTableError> {
        let native = |provider: Provider, url| UpstreamEntry {
            id: UpstreamId::default_for(provider),
            api: provider.native_api(),
            url,
            auth: UpstreamAuth::ClientOwned,
        };
        let mut table = Self {
            anthropic: native(Provider::Anthropic, anthropic),
            codex: native(Provider::Codex, codex),
            openai: native(Provider::OpenAi, openai),
            configured: Vec::new(),
        };
        for entry in extra {
            if table.entries().any(|known| known.id == entry.id) {
                return Err(UpstreamTableError::Duplicate(entry.id.to_string()));
            }
            table.configured.push(entry);
        }
        Ok(table)
    }

    /// The provider's own client-owned entry.
    pub fn native(&self, provider: Provider) -> &UpstreamEntry {
        match provider {
            Provider::Anthropic => &self.anthropic,
            Provider::Codex => &self.codex,
            Provider::OpenAi => &self.openai,
        }
    }

    /// The forwarding routes: [`Route::ALL`], each bound to its provider's
    /// native entry.
    pub fn routes(&self) -> Vec<BoundRoute> {
        Route::ALL
            .iter()
            .map(|route| {
                let entry = self.native(route.provider);
                BoundRoute {
                    path: route.path.into(),
                    forward: route.forward,
                    upstream: entry.id.clone(),
                    api: entry.api,
                }
            })
            .collect()
    }

    pub fn get(&self, id: &UpstreamId) -> Option<&UpstreamEntry> {
        self.entries().find(|entry| &entry.id == id)
    }

    pub fn summaries(&self) -> Vec<crate::policy::UpstreamSummary> {
        self.entries()
            .map(|entry| crate::policy::UpstreamSummary {
                id: entry.id.clone(),
                provider: entry.api.provider(),
                auth: entry.auth.kind(),
            })
            .collect()
    }

    /// The native entries, then the configured ones.
    fn entries(&self) -> impl Iterator<Item = &UpstreamEntry> {
        [&self.anthropic, &self.codex, &self.openai]
            .into_iter()
            .chain(&self.configured)
    }
}

/// Transport deadlines owned by the upstream connection.
#[derive(Debug, Clone, Copy)]
pub struct TransportTimeouts {
    pub connect: Duration,
    /// Longest wait for any single read, which bounds a quiet stream.
    pub idle: Duration,
}

/// HTTP forwarding through Reqwest: no redirects, retries, decompression, or
/// environment proxies. WebSocket handshakes use a separate HTTP/1.1-only
/// client, because the upgrade needs HTTP/1.1.
pub struct ReqwestUpstream {
    client: Client,
    handshakes: Client,
    urls: UpstreamTable,
}

impl ReqwestUpstream {
    pub fn new(
        urls: UpstreamTable,
        timeouts: TransportTimeouts,
    ) -> Result<Self, UpstreamConfigError> {
        let builder = || {
            Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .no_gzip()
                .no_brotli()
                .no_deflate()
                .no_zstd()
                .no_proxy()
                .connect_timeout(timeouts.connect)
        };
        let client = builder()
            .read_timeout(timeouts.idle)
            .build()
            .map_err(|_| UpstreamConfigError("failed to initialize HTTP transport"))?;
        // No read timeout: a quiet socket between calls is normal.
        let handshakes = builder()
            .http1_only()
            .build()
            .map_err(|_| UpstreamConfigError("failed to initialize HTTP transport"))?;
        Ok(Self {
            client,
            handshakes,
            urls,
        })
    }

    /// Classify a failure before the response head. Only an error raised while
    /// connecting, or an HTTP/2 stream the upstream refused before processing,
    /// proves the request was not accepted.
    fn send_failure(error: &reqwest::Error) -> SendFailure {
        if error.is_connect() {
            SendFailure::NotSent
        } else if Self::refused_stream(error) {
            SendFailure::Refused
        } else if error.is_timeout() {
            SendFailure::HeaderTimeout
        } else {
            SendFailure::SendFailed
        }
    }

    /// Whether the error is the upstream's own `RST_STREAM` with
    /// `REFUSED_STREAM`. A reset TGV's transport raised locally is not
    /// evidence about the upstream, so only a remote reset counts.
    fn refused_stream(error: &reqwest::Error) -> bool {
        let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
        while let Some(current) = source {
            if let Some(h2) = current.downcast_ref::<h2::Error>() {
                return h2.is_remote() && h2.reason() == Some(h2::Reason::REFUSED_STREAM);
            }
            source = current.source();
        }
        false
    }

    fn stream_failure(error: &reqwest::Error) -> StreamFailure {
        if error.is_timeout() {
            StreamFailure::IdleTimeout
        } else {
            StreamFailure::BodyError
        }
    }
}

impl Upstream for ReqwestUpstream {
    async fn send(
        &self,
        request: UpstreamRequest,
    ) -> Result<http::Response<UpstreamBody>, SendFailure> {
        let url = self
            .urls
            .get(&request.upstream)
            .ok_or(SendFailure::NotSent)?
            .url
            .request_url(request.path, request.query.as_deref());
        // Reqwest requires a Sync body. Adapt the Send-only body through the
        // standard frame stream and SyncStream without locking or losing frames.
        let body = reqwest::Body::wrap(StreamBody::new(SyncStream::new(BodyStream::new(
            request.body,
        ))));
        let response = self
            .client
            .request(request.method, url)
            .headers(request.headers)
            .body(body)
            .send()
            .await
            .map_err(|error| Self::send_failure(&error))?;
        let response: http::Response<reqwest::Body> = response.into();
        Ok(response
            .map(|body| UpstreamBody::new(body.map_err(|error| Self::stream_failure(&error)))))
    }

    async fn connect(&self, handshake: UpstreamHandshake) -> Result<Connected, SendFailure> {
        let url = self
            .urls
            .get(&handshake.upstream)
            .ok_or(SendFailure::NotSent)?
            .url
            .request_url(handshake.path, handshake.query.as_deref());
        let expected = derive_accept_key(handshake.key.as_bytes());
        let response = self
            .handshakes
            .get(url)
            .headers(handshake.headers)
            .header(http::header::CONNECTION, "upgrade")
            .header(http::header::UPGRADE, "websocket")
            .header(http::header::SEC_WEBSOCKET_VERSION, "13")
            .header(http::header::SEC_WEBSOCKET_KEY, handshake.key)
            .send()
            .await
            .map_err(|error| Self::send_failure(&error))?;
        if response.status() != http::StatusCode::SWITCHING_PROTOCOLS {
            let response: http::Response<reqwest::Body> = response.into();
            return Ok(Connected::Refused(response.map(|body| {
                UpstreamBody::new(body.map_err(|error| Self::stream_failure(&error)))
            })));
        }
        // A 101 that does not answer this key, or upgrades to something
        // else, is not this WebSocket: the request may have been accepted.
        let accepted = response
            .headers()
            .get(http::header::SEC_WEBSOCKET_ACCEPT)
            .is_some_and(|accept| accept.as_bytes() == expected.as_bytes())
            && response
                .headers()
                .get(http::header::UPGRADE)
                .is_some_and(|upgrade| upgrade.as_bytes().eq_ignore_ascii_case(b"websocket"));
        if !accepted {
            return Err(SendFailure::SendFailed);
        }
        let status = response.status();
        let headers = response.headers().clone();
        let socket = response
            .upgrade()
            .await
            .map_err(|_| SendFailure::SendFailed)?;
        Ok(Connected::Upgraded {
            status,
            headers,
            socket: UpstreamSocket::new(socket),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AuthOverride, OperatorKey, ReqwestUpstream, UpstreamAuth, UpstreamEntry, UpstreamTable,
        UpstreamUrl,
    };
    use crate::attempt::SendFailure;
    use tgv_protocol::route::Provider;
    use tgv_recorder::routing::UpstreamId;

    /// Send one request to an HTTP/2 server that resets every stream with
    /// `reason`, and classify the failure.
    async fn reset_with(reason: h2::Reason) -> SendFailure {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = async {
            let (socket, _) = listener.accept().await.unwrap();
            let mut connection = h2::server::handshake(socket).await.unwrap();
            while let Some(Ok((_request, mut respond))) = connection.accept().await {
                respond.send_reset(reason);
            }
        };
        let client = reqwest::Client::builder()
            .http2_prior_knowledge()
            .no_proxy()
            .build()
            .unwrap();
        let send = client
            .post(format!("http://{address}/v1/messages"))
            .body("{}")
            .send();
        tokio::select! {
            () = server => unreachable!("the server serves until the client is done"),
            result = send => ReqwestUpstream::send_failure(&result.unwrap_err()),
        }
    }

    #[tokio::test]
    async fn only_a_refused_stream_proves_the_request_was_not_processed() {
        // RFC 9113 section 8.7: a stream reset with REFUSED_STREAM was closed
        // before any processing, so it is safe to send elsewhere.
        assert_eq!(
            reset_with(h2::Reason::REFUSED_STREAM).await,
            SendFailure::Refused
        );
        // Any other reset may come after processing began.
        assert_eq!(
            reset_with(h2::Reason::INTERNAL_ERROR).await,
            SendFailure::SendFailed
        );
    }

    #[test]
    fn rejects_credential_urls_without_echoing_them() {
        for base in [
            "https://user:SECRET@example.com",
            "https://example.com?token=SECRET",
            "https://example.com/#SECRET",
        ] {
            let error = UpstreamUrl::parse(base).err().unwrap();
            assert!(!format!("{error:?}").contains("SECRET"));
        }
    }

    #[test]
    fn cleartext_is_only_allowed_for_loopback() {
        assert!(UpstreamUrl::parse("http://example.com").is_err());
        for base in [
            "http://127.0.0.1:1234",
            "http://[::1]:1234",
            "http://localhost:1234",
            "https://example.com/prefix/",
        ] {
            assert!(UpstreamUrl::parse(base).is_ok());
        }
    }

    #[test]
    fn targets_preserve_base_prefixes_and_native_queries() {
        let base = UpstreamUrl::parse("https://example.com/backend-api/codex/").unwrap();
        let url = base.request_url("/responses", Some("beta=true&value=a%2Fb"));
        assert_eq!(
            url.as_str(),
            "https://example.com/backend-api/codex/responses?beta=true&value=a%2Fb"
        );
        assert!(base.request_url("/models", None).query().is_none());
    }

    #[test]
    fn each_provider_resolves_to_its_own_base() {
        let table = UpstreamTable::new(
            UpstreamUrl::parse("https://a.example").unwrap(),
            UpstreamUrl::parse("https://c.example").unwrap(),
            UpstreamUrl::parse("https://o.example").unwrap(),
            Vec::new(),
        )
        .unwrap();
        for (provider, host) in [
            (Provider::Anthropic, "a.example"),
            (Provider::Codex, "c.example"),
            (Provider::OpenAi, "o.example"),
        ] {
            let entry = table.native(provider);
            assert_eq!(table.get(&entry.id).unwrap().url.0.host_str(), Some(host));
            assert_eq!(entry.api, provider.native_api());
        }
    }

    #[test]
    fn an_operator_key_replaces_client_credentials_and_the_oauth_beta_only() {
        let key = OperatorKey::new("sk-FIXTURE").unwrap();
        assert!(!format!("{key:?}").contains("FIXTURE"));
        let entry = |provider: Provider, auth: UpstreamAuth| UpstreamEntry {
            id: UpstreamId::parse("fixture").unwrap(),
            api: provider.native_api(),
            url: UpstreamUrl::parse("https://example.com").unwrap(),
            auth,
        };
        let anthropic = entry(Provider::Anthropic, UpstreamAuth::OperatorKey(key.clone()));
        let openai = entry(Provider::OpenAi, UpstreamAuth::OperatorKey(key));
        let mut headers = http::HeaderMap::new();
        headers.insert("authorization", "Bearer CLIENT".parse().unwrap());
        headers.insert("cookie", "CLIENT".parse().unwrap());
        headers.insert(
            "anthropic-beta",
            "oauth-2025-04-20, fine-grained-tool-streaming-2025-05-14"
                .parse()
                .unwrap(),
        );
        headers.insert("x-claude-code-session-id", "kept".parse().unwrap());
        assert_eq!(anthropic.authorize(&mut headers), AuthOverride::OperatorKey);
        assert!(headers.get("authorization").is_none() && headers.get("cookie").is_none());
        assert_eq!(headers["x-api-key"], "sk-FIXTURE");
        assert!(headers["x-api-key"].is_sensitive());
        assert_eq!(
            headers["anthropic-beta"],
            "fine-grained-tool-streaming-2025-05-14"
        );
        assert_eq!(headers["x-claude-code-session-id"], "kept");

        let mut headers = http::HeaderMap::new();
        headers.insert("chatgpt-account-id", "CLIENT".parse().unwrap());
        assert_eq!(openai.authorize(&mut headers), AuthOverride::OperatorKey);
        assert_eq!(headers["authorization"], "Bearer sk-FIXTURE");
        assert!(headers.get("chatgpt-account-id").is_none());

        let mut untouched = http::HeaderMap::new();
        untouched.insert("authorization", "Bearer CLIENT".parse().unwrap());
        assert_eq!(
            entry(Provider::OpenAi, UpstreamAuth::ClientOwned).authorize(&mut untouched),
            AuthOverride::None
        );
        assert_eq!(untouched["authorization"], "Bearer CLIENT");
    }
}
