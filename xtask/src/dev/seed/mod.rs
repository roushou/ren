//! `cargo xtask dev seed`: play a [`Scenario`] through the dev gateway. The
//! calls are pure data ([`scenario`]); this module only sends them, reads
//! each answer whole, and prints one line per call.

mod scenario;

pub use scenario::{Call, Run, Scenario};
pub(in crate::dev) use scenario::{Caller, Client, PROMPTS};

use futures_util::{SinkExt as _, StreamExt as _};
use scenario::{Channel, Plan};
use std::{fs, net::SocketAddr, path::Path, time::Duration};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest as _, http::HeaderValue};

/// Why seeding failed.
#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("cannot start the seeder's runtime: {0}")]
    Runtime(#[source] std::io::Error),
    #[error("cannot build the HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("the dev gateway at {0} did not answer; is the slot up (`cargo xtask dev up`)?")]
    Unreachable(SocketAddr),
    #[error("cannot keep the seed run count in {path}: {source}")]
    Runs {
        path: String,
        source: std::io::Error,
    },
}

/// Sends seed traffic to one gateway.
pub struct Seeder {
    gateway: SocketAddr,
    token: String,
}

impl Seeder {
    pub fn new(gateway: SocketAddr, token: String) -> Self {
        Self { gateway, token }
    }

    /// The next run number, counted in the slot's data directory, which a
    /// reset removes.
    pub fn next_run(data_dir: &Path) -> Result<u32, SeedError> {
        let path = data_dir.join("seed-runs");
        let error = |source| SeedError::Runs {
            path: path.display().to_string(),
            source,
        };
        let previous: u32 = fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0);
        let next = previous.saturating_add(1);
        fs::write(&path, next.to_string()).map_err(error)?;
        Ok(next)
    }

    /// Play `scenario` once, with new sessions.
    pub fn play(&self, scenario: Scenario, data_dir: &Path) -> Result<(), SeedError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(SeedError::Runtime)?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(SeedError::Client)?;
        let run = Run(Self::next_run(data_dir)?);
        let plan = scenario.plan(run, &self.token);
        runtime.block_on(self.send(&client, &plan))
    }

    async fn send(&self, client: &reqwest::Client, plan: &Plan) -> Result<(), SeedError> {
        for call in &plan.calls {
            let mut request = client
                .post(format!("http://{}{}", self.gateway, call.path))
                .body(call.body.clone());
            for (name, value) in &call.headers {
                request = request.header(name, value);
            }
            let response = request
                .send()
                .await
                .map_err(|_| SeedError::Unreachable(self.gateway))?;
            let status = response.status().as_u16();
            // Read the whole answer, so the call ends as a client's would.
            drop(response.bytes().await);
            println!("{status} {}", call.label);
        }
        for channel in &plan.channels {
            let outcome = match self.channel(channel).await {
                Ok(()) => "ok".to_owned(),
                Err(reason) => format!("failed: {reason}"),
            };
            println!("{outcome} {}", channel.label);
        }
        Ok(())
    }

    /// Open a WebSocket channel, send each message, and wait for its
    /// terminal event before the next.
    async fn channel(&self, channel: &Channel) -> Result<(), String> {
        let mut request = format!("ws://{}{}", self.gateway, channel.path)
            .into_client_request()
            .map_err(|error| error.to_string())?;
        for (name, value) in &channel.headers {
            if let (Ok(name), Ok(value)) = (
                tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                request.headers_mut().insert(name, value);
            }
        }
        let stream = tokio::net::TcpStream::connect(self.gateway)
            .await
            .map_err(|error| error.to_string())?;
        let (mut socket, _) = tokio_tungstenite::client_async(request, stream)
            .await
            .map_err(|error| error.to_string())?;
        for message in &channel.messages {
            socket
                .send(Message::Text(message.clone().into()))
                .await
                .map_err(|error| error.to_string())?;
            loop {
                let Some(Ok(Message::Text(event))) = socket.next().await else {
                    return Err("the channel closed before the call ended".into());
                };
                let event: serde_json::Value =
                    serde_json::from_str(event.as_str()).unwrap_or_default();
                if let Some("response.completed" | "response.failed" | "error") =
                    event["type"].as_str()
                {
                    break;
                }
            }
        }
        drop(socket.close(None).await);
        Ok(())
    }
}
