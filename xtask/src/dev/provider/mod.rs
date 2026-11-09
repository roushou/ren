//! The dev slot's fake provider: one loopback server that answers every wire
//! format TGV forwards (Anthropic Messages, OpenAI Responses and Chat
//! Completions, Codex over HTTP and WebSocket) with scripted replies. It
//! accepts any credentials and stores nothing. What it says is decided by
//! the pure [`script`]; this module only writes it.

mod script;

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{
        FromRequestParts, Request,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt as _, StreamExt as _, stream};
use script::{Reply, Request as Scripted, SocketTurn, Wire};
use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// The fake provider, listening on one loopback address.
pub struct FakeProvider {
    /// Numbers each answered request, for response identifiers.
    serial: AtomicU64,
}

/// Why the fake provider stopped.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("the fake provider listens on loopback only, not {0}")]
    NotLoopback(SocketAddr),
    #[error("cannot start the fake provider's runtime: {0}")]
    Runtime(#[source] std::io::Error),
    #[error("cannot listen on {address}: {source}")]
    Bind {
        address: SocketAddr,
        source: std::io::Error,
    },
    #[error("the fake provider failed: {0}")]
    Serve(#[source] std::io::Error),
}

impl FakeProvider {
    /// The largest request body read; a fake needs no more.
    const MAX_BODY: usize = 64 * 1024 * 1024;

    /// Serve on `address` until the process is stopped.
    pub fn serve(address: SocketAddr) -> Result<(), ProviderError> {
        if !address.ip().is_loopback() {
            return Err(ProviderError::NotLoopback(address));
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(ProviderError::Runtime)?;
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind(address)
                .await
                .map_err(|source| ProviderError::Bind { address, source })?;
            println!("fake provider listening on http://{address}");
            axum::serve(listener, Self::router())
                .await
                .map_err(ProviderError::Serve)
        })
    }

    fn router() -> Router {
        let provider = Arc::new(Self {
            serial: AtomicU64::new(1),
        });
        Router::new().fallback(move |request: Request| {
            let provider = provider.clone();
            async move { provider.answer(request).await }
        })
    }

    async fn answer(self: Arc<Self>, request: Request) -> Response {
        let serial = self.serial.fetch_add(1, Ordering::Relaxed);
        let Some(wire) = Wire::for_path(request.uri().path()) else {
            return (
                StatusCode::NOT_FOUND,
                "the fake provider has no such endpoint\n",
            )
                .into_response();
        };
        let (mut parts, body) = request.into_parts();
        if parts.method == Method::GET && parts.headers.contains_key(header::UPGRADE) {
            return match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
                Ok(upgrade) => upgrade.on_upgrade(move |socket| self.socket(socket)),
                Err(rejection) => rejection.into_response(),
            };
        }
        let Ok(body) = axum::body::to_bytes(body, Self::MAX_BODY).await else {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                "request too large for the fake\n",
            )
                .into_response();
        };
        Self::write(Reply::to(&Scripted {
            wire,
            body: &body,
            serial,
        }))
    }

    /// Write a scripted reply, waiting before each piece as it says.
    fn write(reply: Reply) -> Response {
        let pieces = stream::iter(reply.pieces).then(|piece| async move {
            tokio::time::sleep(piece.after).await;
            Ok::<_, Infallible>(Bytes::from(piece.bytes))
        });
        let mut response = Response::new(Body::from_stream(pieces));
        *response.status_mut() = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::OK);
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(reply.content_type),
        );
        for (name, value) in reply.headers {
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.insert(name, value);
            }
        }
        response
    }

    /// A Codex WebSocket: each `response.create` gets its scripted events.
    async fn socket(self: Arc<Self>, socket: WebSocket) {
        let (mut sink, mut messages) = socket.split();
        while let Some(Ok(message)) = messages.next().await {
            let text = match message {
                Message::Text(text) => text,
                Message::Close(_) => break,
                Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => continue,
            };
            let serial = self.serial.fetch_add(1, Ordering::Relaxed);
            let Some(events) = SocketTurn::answer(text.as_str(), serial) else {
                continue;
            };
            for (after, event) in events {
                tokio::time::sleep(after).await;
                if sink.send(Message::Text(event.into())).await.is_err() {
                    return;
                }
            }
        }
    }
}
