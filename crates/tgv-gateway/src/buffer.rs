//! Request buffering. Policy, transforms, and failover replay need the
//! whole request, so a request body is read completely before dispatch.
//!
//! Memory is bounded globally: a byte semaphore is acquired from
//! `Content-Length` before reading, or incrementally while reading a body
//! without one. A request over its provider's limit is refused with `413`;
//! when the budget is exhausted the client gets `503` with `retry-after` at
//! once, never a wait. There is no pass-through mode, because it would bypass
//! policy. The permit is held for as long as the call holds its bytes.

use axum::body::Body;
use bytes::{Bytes, BytesMut};
use http::HeaderMap;
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tgv_protocol::{
    encoding::{Decompressor, ResponseEncoding},
    route::Provider,
};
use tgv_recorder::capture::CallRejection;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// The largest request body each provider's routes accept; larger ones get
/// `413`. Defaults are the providers' documented limits: Anthropic's 32 MB
/// Messages limit, and OpenAI's 512 MB request payload limit, also used for
/// the Codex backend, which documents none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RequestLimits {
    pub anthropic: usize,
    pub codex: usize,
    pub openai: usize,
}

impl Default for RequestLimits {
    fn default() -> Self {
        Self {
            anthropic: 32 * 1024 * 1024,
            codex: 512 * 1024 * 1024,
            openai: 512 * 1024 * 1024,
        }
    }
}

impl RequestLimits {
    /// The same limit for every provider.
    pub fn all(bytes: usize) -> Self {
        Self {
            anthropic: bytes,
            codex: bytes,
            openai: bytes,
        }
    }

    pub fn get(&self, provider: Provider) -> usize {
        match provider {
            Provider::Anthropic => self.anthropic,
            Provider::Codex => self.codex,
            Provider::OpenAi => self.openai,
        }
    }

    fn each(&self) -> [usize; 3] {
        [self.anthropic, self.codex, self.openai]
    }
}

/// The `[buffer]` configuration section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BufferOptions {
    pub request_limits: RequestLimits,
    /// Request bytes held at once across every call.
    pub budget_bytes: usize,
    /// Longest gap between two pieces of an upload.
    pub idle_timeout_seconds: u64,
    /// Longest time to receive a whole upload.
    pub total_timeout_seconds: u64,
}

impl Default for BufferOptions {
    fn default() -> Self {
        Self {
            request_limits: RequestLimits::default(),
            budget_bytes: 1024 * 1024 * 1024,
            idle_timeout_seconds: 30,
            total_timeout_seconds: 300,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BufferOptionsError {
    #[error("buffer.request_limits values must be between 1 KiB and buffer.budget_bytes")]
    RequestLimit,
    #[error("buffer.budget_bytes must be at most 4 GiB")]
    Budget,
    #[error("buffer timeouts must be between 1 and 3600 seconds")]
    Timeout,
}

impl BufferOptions {
    const MAX_BUDGET: usize = 4 * 1024 * 1024 * 1024;

    pub fn validate(&self) -> Result<(), BufferOptionsError> {
        if self.budget_bytes > Self::MAX_BUDGET {
            return Err(BufferOptionsError::Budget);
        }
        if !self
            .request_limits
            .each()
            .iter()
            .all(|limit| (1024..=self.budget_bytes).contains(limit))
        {
            return Err(BufferOptionsError::RequestLimit);
        }
        if [self.idle_timeout_seconds, self.total_timeout_seconds]
            .iter()
            .any(|seconds| !(1..=3600).contains(seconds))
        {
            return Err(BufferOptionsError::Timeout);
        }
        Ok(())
    }
}

/// Why a request was not buffered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BufferRefusal {
    #[error("request body exceeds the TGV request limit")]
    TooLarge,
    #[error("TGV request buffer is full")]
    BufferFull,
    #[error("request body did not arrive in time")]
    UploadTimeout,
    #[error("request body failed")]
    UploadFailed,
}

impl BufferRefusal {
    pub fn rejection(self) -> CallRejection {
        match self {
            Self::TooLarge => CallRejection::TooLarge,
            Self::BufferFull => CallRejection::BufferFull,
            Self::UploadTimeout => CallRejection::UploadTimeout,
            Self::UploadFailed => CallRejection::UploadFailed,
        }
    }
}

/// A whole request body and the budget it holds.
#[derive(Debug)]
pub struct Buffered {
    bytes: Bytes,
    _permit: Option<OwnedSemaphorePermit>,
}

impl Buffered {
    pub fn bytes(&self) -> &Bytes {
        &self.bytes
    }
}

/// The global request buffer.
#[derive(Debug, Clone)]
pub struct RequestBuffer {
    budget: Arc<Semaphore>,
    limits: RequestLimits,
    idle: Duration,
    total: Duration,
}

impl RequestBuffer {
    pub fn new(options: BufferOptions) -> Self {
        Self {
            budget: Arc::new(Semaphore::new(options.budget_bytes)),
            limits: options.request_limits,
            idle: Duration::from_secs(options.idle_timeout_seconds),
            total: Duration::from_secs(options.total_timeout_seconds),
        }
    }

    /// Bytes of budget not held by any request.
    pub fn available(&self) -> usize {
        self.budget.available_permits()
    }

    /// Read the whole body, within `provider`'s limit. Trailers on a request
    /// are not kept.
    pub async fn read(
        &self,
        provider: Provider,
        headers: &HeaderMap,
        body: Body,
    ) -> Result<Buffered, BufferRefusal> {
        let declared = headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());
        let mut held = Held::new(self, self.limits.get(provider));
        if let Some(declared) = declared {
            held.reserve(declared)?;
        }
        let mut buffer = BytesMut::with_capacity(declared.unwrap_or(0));
        let read = async {
            let mut body = body;
            loop {
                let frame = tokio::time::timeout(self.idle, body.frame())
                    .await
                    .map_err(|_| BufferRefusal::UploadTimeout)?;
                match frame {
                    None => return Ok(()),
                    Some(Err(_)) => return Err(BufferRefusal::UploadFailed),
                    Some(Ok(frame)) => {
                        if let Some(data) = frame.data_ref() {
                            held.reserve(buffer.len().saturating_add(data.len()))?;
                            buffer.extend_from_slice(data);
                        }
                    }
                }
            }
        };
        tokio::time::timeout(self.total, read)
            .await
            .map_err(|_| BufferRefusal::UploadTimeout)??;
        Ok(Buffered {
            bytes: buffer.freeze(),
            _permit: held.permit,
        })
    }
}

/// Budget held by one read, grown as bytes arrive.
struct Held<'a> {
    buffer: &'a RequestBuffer,
    limit: usize,
    permit: Option<OwnedSemaphorePermit>,
    reserved: usize,
}

impl<'a> Held<'a> {
    fn new(buffer: &'a RequestBuffer, limit: usize) -> Self {
        Self {
            buffer,
            limit,
            permit: None,
            reserved: 0,
        }
    }

    /// Hold at least `total` bytes, or refuse.
    fn reserve(&mut self, total: usize) -> Result<(), BufferRefusal> {
        if total > self.limit {
            return Err(BufferRefusal::TooLarge);
        }
        let more = total.saturating_sub(self.reserved);
        if more == 0 {
            return Ok(());
        }
        let more_permits = u32::try_from(more).map_err(|_| BufferRefusal::TooLarge)?;
        let permit = self
            .buffer
            .budget
            .clone()
            .try_acquire_many_owned(more_permits)
            .map_err(|_| BufferRefusal::BufferFull)?;
        match &mut self.permit {
            Some(held) => held.merge(permit),
            None => self.permit = Some(permit),
        }
        self.reserved = total;
        Ok(())
    }
}

/// A request body decoded for reading, if its encoding allows: what policy
/// reads (the model) and body transforms change.
#[derive(Debug)]
pub enum Decoded<'a> {
    /// The body as sent.
    Identity(&'a [u8]),
    /// Decoded from a compressed body, at most the request limit.
    Decompressed(Vec<u8>),
    /// Not decodable: unsupported, corrupt, or larger than the limit.
    Opaque,
}

impl<'a> Decoded<'a> {
    pub fn new(headers: &HeaderMap, body: &'a [u8], limit: usize) -> Self {
        let encoding = ResponseEncoding::from_headers(headers);
        match encoding {
            ResponseEncoding::Identity => return Self::Identity(body),
            ResponseEncoding::Gzip
            | ResponseEncoding::Zstd
            | ResponseEncoding::Brotli
            | ResponseEncoding::Unsupported => {}
        }
        let Ok(mut decoder) = Decompressor::new(encoding) else {
            return Self::Opaque;
        };
        let mut out = Vec::new();
        let mut over = false;
        let mut push = |bytes: &[u8]| {
            if out.len().saturating_add(bytes.len()) > limit {
                over = true;
            } else {
                out.extend_from_slice(bytes);
            }
        };
        let decoded = decoder
            .decode(body, &mut push)
            .and_then(|()| decoder.finish(&mut push));
        match (decoded, over) {
            (Ok(()), false) => Self::Decompressed(out),
            (Ok(()), true) | (Err(_), _) => Self::Opaque,
        }
    }

    pub fn json(&self) -> Option<&[u8]> {
        match self {
            Self::Identity(bytes) => Some(bytes),
            Self::Decompressed(bytes) => Some(bytes),
            Self::Opaque => None,
        }
    }

    /// The top-level `model`, if the body is a JSON object with one.
    pub fn model(&self) -> Option<String> {
        #[derive(serde::Deserialize)]
        struct Model {
            model: Option<String>,
        }
        serde_json::from_slice::<Model>(self.json()?)
            .ok()
            .and_then(|body| body.model)
    }
}

#[cfg(test)]
mod tests {
    use super::{BufferOptions, BufferRefusal, Decoded, RequestBuffer, RequestLimits};
    use axum::body::Body;
    use bytes::Bytes;
    use http::HeaderMap;
    use tgv_protocol::route::Provider;

    fn buffer(limit: usize, budget: usize) -> RequestBuffer {
        RequestBuffer::new(BufferOptions {
            request_limits: RequestLimits::all(limit),
            budget_bytes: budget,
            idle_timeout_seconds: 1,
            total_timeout_seconds: 2,
        })
    }

    fn sized(len: usize) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(http::header::CONTENT_LENGTH, len.into());
        headers
    }

    fn chunked(pieces: Vec<&'static [u8]>) -> Body {
        Body::from_stream(futures_util::stream::iter(
            pieces
                .into_iter()
                .map(|piece| Ok::<_, std::io::Error>(Bytes::from_static(piece))),
        ))
    }

    #[tokio::test]
    async fn memory_is_bounded_by_the_budget_and_released_with_the_body() {
        let buffer = buffer(1024, 2048);
        let first = buffer
            .read(
                Provider::Anthropic,
                &sized(1024),
                Body::from(vec![1_u8; 1024]),
            )
            .await
            .unwrap();
        let second = buffer
            .read(
                Provider::Anthropic,
                &HeaderMap::new(),
                chunked(vec![&[2; 600], &[3; 400]]),
            )
            .await
            .unwrap();
        assert_eq!(second.bytes().len(), 1000);
        assert_eq!(buffer.available(), 24);
        // The budget is exhausted: refused at once, from the declared length
        // or while reading.
        let declared = buffer
            .read(
                Provider::Anthropic,
                &sized(100),
                Body::from(vec![0_u8; 100]),
            )
            .await;
        assert_eq!(declared.unwrap_err(), BufferRefusal::BufferFull);
        let streamed = buffer
            .read(
                Provider::Anthropic,
                &HeaderMap::new(),
                chunked(vec![&[0; 20], &[0; 20]]),
            )
            .await;
        assert_eq!(streamed.unwrap_err(), BufferRefusal::BufferFull);
        assert_eq!(buffer.available(), 24, "a refused read holds nothing");
        drop((first, second));
        assert_eq!(buffer.available(), 2048);
    }

    #[tokio::test]
    async fn the_request_limit_is_a_413_however_the_body_arrives() {
        let buffer = buffer(1024, 4096);
        let declared = buffer
            .read(Provider::Anthropic, &sized(1025), Body::empty())
            .await;
        assert_eq!(declared.unwrap_err(), BufferRefusal::TooLarge);
        let streamed = buffer
            .read(
                Provider::Anthropic,
                &HeaderMap::new(),
                chunked(vec![&[0; 1000], &[0; 25]]),
            )
            .await;
        assert_eq!(streamed.unwrap_err(), BufferRefusal::TooLarge);
        assert_eq!(buffer.available(), 4096);
    }

    #[tokio::test]
    async fn a_stalled_or_failed_upload_is_refused() {
        let buffer = buffer(1024, 4096);
        let stalled =
            Body::from_stream(futures_util::stream::pending::<Result<Bytes, std::io::Error>>());
        let result = buffer
            .read(Provider::Anthropic, &HeaderMap::new(), stalled)
            .await;
        assert_eq!(result.unwrap_err(), BufferRefusal::UploadTimeout);
        let failed = Body::from_stream(futures_util::stream::iter([Err::<Bytes, _>(
            std::io::Error::other("gone"),
        )]));
        let result = buffer
            .read(Provider::Anthropic, &HeaderMap::new(), failed)
            .await;
        assert_eq!(result.unwrap_err(), BufferRefusal::UploadFailed);
    }

    #[test]
    fn options_validate() {
        assert!(BufferOptions::default().validate().is_ok());
        let tiny = BufferOptions {
            request_limits: RequestLimits {
                codex: 10,
                ..RequestLimits::default()
            },
            ..BufferOptions::default()
        };
        assert!(tiny.validate().is_err());
        let over = BufferOptions {
            request_limits: RequestLimits::all(2048),
            budget_bytes: 1024,
            ..BufferOptions::default()
        };
        assert!(over.validate().is_err());
    }

    #[tokio::test]
    async fn each_provider_has_its_own_limit() {
        let buffer = RequestBuffer::new(BufferOptions {
            request_limits: RequestLimits {
                anthropic: 1024,
                codex: 2048,
                openai: 4096,
            },
            budget_bytes: 8192,
            ..BufferOptions::default()
        });
        let body = || Body::from(vec![0_u8; 3000]);
        for (provider, accepted) in [
            (Provider::Anthropic, false),
            (Provider::Codex, false),
            (Provider::OpenAi, true),
        ] {
            let read = buffer.read(provider, &sized(3000), body()).await;
            assert_eq!(read.is_ok(), accepted, "{provider:?}");
        }
    }

    #[test]
    fn buffer_options_read_with_defaults() {
        let read = |toml_text: &str| toml::from_str::<BufferOptions>(toml_text);
        assert_eq!(read("").unwrap(), BufferOptions::default());
        assert!(read("request_limit_bytes = 2048").is_err());
        let split = read("[request_limits]\nanthropic = 4096").unwrap();
        assert_eq!(split.request_limits.anthropic, 4096);
        assert_eq!(split.request_limits.openai, RequestLimits::default().openai);
        assert!(read("unknown = 1").is_err());
        // The resolved form reads back unchanged.
        let written = toml::to_string(&split).unwrap();
        assert_eq!(read(&written).unwrap(), split);
    }

    #[test]
    fn the_model_is_read_from_identity_and_compressed_bodies() {
        let body = br#"{"model":"m-1","input":[]}"#;
        assert_eq!(
            Decoded::new(&HeaderMap::new(), body, 1024)
                .model()
                .as_deref(),
            Some("m-1")
        );
        let compressed = zstd::encode_all(&body[..], 3).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(http::header::CONTENT_ENCODING, "zstd".parse().unwrap());
        let decoded = Decoded::new(&headers, &compressed, 1024);
        assert_eq!(decoded.model().as_deref(), Some("m-1"));
        assert!(matches!(
            Decoded::new(&headers, &compressed, 4),
            Decoded::Opaque
        ));
        assert!(matches!(
            Decoded::new(&headers, b"junk", 1024),
            Decoded::Opaque
        ));
    }
}
