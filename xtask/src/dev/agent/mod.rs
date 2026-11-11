//! `cargo xtask dev agent`: a fake `codex app-server` for the dev slot's
//! runs. The slot puts a `codex` that runs it first on the dev daemon's
//! PATH, so the daemon starts and drives it as it would Codex. It speaks the
//! app-server protocol on stdio ([`script`]) and sends each turn's model call
//! through the dev gateway to the fake provider, as Codex would, so the
//! run's traffic is recorded under its thread's session. It never reads a
//! real client's configuration or credentials.

mod script;

use super::seed::{Caller, Client, Run, Scenario, Seeder};
use ren_protocol::sse::{EventName, SseFramer, SseLimits, SseOutput};
use script::{AgentScript, Out};
use serde_json::Value;
use std::{io::BufRead as _, path::Path};

/// Why the fake agent stopped.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("no gateway in the arguments: expected a model_providers base_url")]
    NoGateway,
    #[error("cannot start the agent's runtime: {0}")]
    Runtime(#[source] std::io::Error),
    #[error("stdio failed: {0}")]
    Stdio(#[source] std::io::Error),
    #[error(transparent)]
    Seed(#[from] super::seed::SeedError),
}

pub struct FakeAgent {
    /// The gateway's root, from the provider override the daemon passed.
    gateway: String,
    script: AgentScript,
}

impl FakeAgent {
    /// `arguments` are those the daemon gave `codex`; the provider override
    /// names the gateway as `base_url="http://…/codex"`.
    pub fn new(data_dir: &Path, arguments: &[String]) -> Result<Self, AgentError> {
        let base = arguments
            .iter()
            .find_map(|argument| {
                let rest = argument.split_once("base_url=\"")?.1;
                Some(rest.split_once('"')?.0.to_owned())
            })
            .ok_or(AgentError::NoGateway)?;
        let gateway = base
            .strip_suffix("/codex")
            .unwrap_or(&base)
            .trim_end_matches('/')
            .to_owned();
        let own = Run(Seeder::next_run(data_dir)?);
        Ok(Self {
            gateway,
            script: AgentScript::new(own),
        })
    }

    /// Serve stdin until it closes.
    pub fn run(mut self) -> Result<(), AgentError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(AgentError::Runtime)?;
        let http = reqwest::Client::new();
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout().lock();
        for line in stdin.lock().lines() {
            let line = line.map_err(AgentError::Stdio)?;
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let mut pending = self.script.receive(&message);
            while !pending.is_empty() {
                let mut next = Vec::new();
                for out in pending {
                    match out {
                        Out::Send(value) => Self::write(&mut stdout, &value)?,
                        Out::Call {
                            session,
                            numbers,
                            prompt,
                        } => {
                            let reply = runtime.block_on(self.call(
                                &http,
                                &mut stdout,
                                session,
                                numbers,
                                &prompt,
                            ));
                            next.extend(self.script.replied(reply));
                        }
                    }
                }
                pending = next;
            }
        }
        Ok(())
    }

    fn write(stdout: &mut impl std::io::Write, value: &Value) -> Result<(), AgentError> {
        let mut line = value.to_string();
        line.push('\n');
        stdout
            .write_all(line.as_bytes())
            .and_then(|()| stdout.flush())
            .map_err(AgentError::Stdio)
    }

    /// Send the turn's model call as Codex would and stream its reply text.
    async fn call(
        &self,
        http: &reqwest::Client,
        stdout: &mut impl std::io::Write,
        session: Run,
        numbers: (u32, u32),
        prompt: &str,
    ) -> Result<String, String> {
        let call = Scenario::step_call(
            Client::Codex,
            &Caller::Unidentified,
            session,
            numbers,
            "gpt-5.5-codex",
            prompt,
            0,
        );
        let mut request = http
            .post(format!("{}{}", self.gateway, call.path))
            .body(call.body);
        for (name, value) in &call.headers {
            request = request.header(name, value);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| "cannot reach the dev gateway".to_owned())?;
        if !response.status().is_success() {
            return Err(format!("the model call answered {}", response.status()));
        }
        let mut framer = SseFramer::new(SseLimits::default());
        let mut reply = String::new();
        let mut written = Ok(());
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "the reply stream broke".to_owned())?
        {
            framer.feed(&chunk, |output| {
                let SseOutput::Event { name, data } = output else {
                    return;
                };
                let Ok(event) = serde_json::from_slice::<Value>(data) else {
                    return;
                };
                let kind = match name {
                    EventName::Present(name) => name,
                    EventName::Absent | EventName::Invalid => {
                        event["type"].as_str().unwrap_or_default()
                    }
                };
                if kind == "response.output_text.delta"
                    && let Some(delta) = event["delta"].as_str()
                {
                    reply.push_str(delta);
                    if written.is_ok() {
                        written = Self::write(stdout, &self.script.delta(delta));
                    }
                }
            });
        }
        written.map_err(|error| error.to_string())?;
        Ok(reply)
    }
}
