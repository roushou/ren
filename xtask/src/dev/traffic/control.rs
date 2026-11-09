//! The traffic generator's control API, on loopback: read and change the
//! live settings, and send bursts. The dev UI's toolbar reaches it through
//! the UI server, which forwards only to this address and only when the dev
//! slot configures it. Changes are JSON requests, which a cross-site form
//! cannot send.

use super::{Reply, Settings, Traffic};
use crate::dev::seed::Run;
use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// Counts the control API reports.
#[derive(Debug, Default)]
pub struct Stats {
    pub sent: AtomicU64,
    pub in_flight: AtomicU64,
}

/// What the control API serves from.
pub struct Control {
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

impl Control {
    pub fn router(self) -> Router {
        Router::new()
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

    async fn status(State(control): State<Arc<Self>>) -> Response {
        match control.traffic.settings() {
            Ok(settings) => Json(Status {
                settings: &settings,
                sent: control.stats.sent.load(Ordering::Relaxed),
                in_flight: control.stats.in_flight.load(Ordering::Relaxed),
            })
            .into_response(),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        }
    }

    async fn save(State(control): State<Arc<Self>>, Json(settings): Json<Settings>) -> Response {
        if let Err(error) = settings.check() {
            return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
        }
        match control.traffic.save(&settings) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        }
    }

    async fn burst(
        State(control): State<Arc<Self>>,
        Json(request): Json<BurstRequest>,
    ) -> Response {
        if !(1..=200).contains(&request.count) {
            return (StatusCode::BAD_REQUEST, "count is 1 to 200").into_response();
        }
        let mut settings = match control.traffic.settings() {
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
        let base = 1_000_000 + control.stats.sent.load(Ordering::Relaxed);
        let calls: Vec<_> = (0..u64::from(request.count))
            .filter_map(|n| Traffic::call(base + n, control.run, &settings, &control.traffic.token))
            .collect();
        let sends = calls.into_iter().map(|call| {
            control
                .traffic
                .counted(&control.client, call, &control.stats)
        });
        futures_util::future::join_all(sends).await;
        StatusCode::NO_CONTENT.into_response()
    }
}
