//! The dev console: a small page, served by the traffic generator on
//! loopback, that shows and changes the live traffic settings and sends
//! bursts. It is the dev slot's own page, not part of TGV's UI, which stays
//! read-only; the UI only links to it. Changes are JSON requests, which a
//! cross-site form cannot send.

use super::{Reply, Settings, Traffic};
use crate::dev::seed::Run;
use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// Counts the console reports.
#[derive(Debug, Default)]
pub struct Stats {
    pub sent: AtomicU64,
    pub in_flight: AtomicU64,
}

/// What the console serves from.
pub struct Console {
    pub traffic: Traffic,
    pub stats: Arc<Stats>,
    pub client: reqwest::Client,
    pub run: Run,
}

#[derive(serde::Serialize)]
struct Status<'a> {
    settings: &'a Settings,
    sent: u64,
    in_flight: u64,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BurstRequest {
    count: u16,
    /// Every call of the burst gets this reply, whatever the mix says.
    #[serde(default)]
    reply: Option<String>,
}

impl Console {
    const PAGE: &'static str = include_str!("console.html");

    pub fn router(self) -> Router {
        Router::new()
            .route("/", get(|| async { Html(Self::PAGE) }))
            .route("/api/status", get(Self::status))
            .route("/api/settings", post(Self::save))
            .route("/api/burst", post(Self::burst))
            .layer(axum::middleware::map_response(
                |mut response: Response| async {
                    response.headers_mut().insert(
                        header::CACHE_CONTROL,
                        header::HeaderValue::from_static("no-store"),
                    );
                    response
                },
            ))
            .with_state(Arc::new(self))
    }

    async fn status(State(console): State<Arc<Self>>) -> Response {
        match console.traffic.settings() {
            Ok(settings) => Json(Status {
                settings: &settings,
                sent: console.stats.sent.load(Ordering::Relaxed),
                in_flight: console.stats.in_flight.load(Ordering::Relaxed),
            })
            .into_response(),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        }
    }

    async fn save(State(console): State<Arc<Self>>, Json(settings): Json<Settings>) -> Response {
        if let Err(error) = settings.check() {
            return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
        }
        match console.traffic.save(&settings) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        }
    }

    async fn burst(
        State(console): State<Arc<Self>>,
        Json(request): Json<BurstRequest>,
    ) -> Response {
        if !(1..=200).contains(&request.count) {
            return (StatusCode::BAD_REQUEST, "count is 1 to 200").into_response();
        }
        let mut settings = match console.traffic.settings() {
            Ok(settings) => settings,
            Err(error) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
            }
        };
        if let Some(reply) = &request.reply {
            match Reply::only(reply) {
                Some(only) => settings.replies = only,
                None => return (StatusCode::BAD_REQUEST, "unknown reply").into_response(),
            }
        }
        // Numbered past the generator's own calls, so turns stay distinct.
        let base = 1_000_000 + console.stats.sent.load(Ordering::Relaxed);
        let calls: Vec<_> = (0..u64::from(request.count))
            .filter_map(|n| Traffic::call(base + n, console.run, &settings, &console.traffic.token))
            .collect();
        let sends = calls.into_iter().map(|call| {
            console
                .traffic
                .counted(&console.client, call, &console.stats)
        });
        futures_util::future::join_all(sends).await;
        StatusCode::NO_CONTENT.into_response()
    }
}
