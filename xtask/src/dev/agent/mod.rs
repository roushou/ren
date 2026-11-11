//! `cargo xtask dev agent`: a fake `codex app-server` or `claude -p` for the
//! dev slot's runs. The slot puts a `codex` and a `claude` that run it first
//! on the dev daemon's PATH, so the daemon starts and drives them as it
//! would the real clients. Each speaks its client's protocol on stdio
//! ([`script`] for Codex, [`claude`] for Claude Code) and sends each turn's
//! model call through the dev gateway to the fake provider, as the client
//! would, so the run's traffic is recorded under its session. It never reads
//! a real client's configuration or credentials.

mod claude;
mod script;

use super::seed::{Call, Caller, Client, Run, Scenario, Seeder};
use clap::ValueEnum;
use ren_harness::host::HostEnv;
use ren_protocol::sse::{EventName, SseFramer, SseLimits, SseOutput};
use serde_json::Value;
use std::{io::BufRead as _, path::Path};

/// Why the fake agent stopped.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error(
        "no gateway: expected a model_providers base_url argument (codex) or ANTHROPIC_BASE_URL (claude)"
    )]
    NoGateway,
    #[error("no conversation: expected --session-id or --resume")]
    NoSession,
    #[error("cannot start the agent's runtime: {0}")]
    Runtime(#[source] std::io::Error),
    #[error("stdio failed: {0}")]
    Stdio(#[source] std::io::Error),
    #[error(transparent)]
    Seed(#[from] super::seed::SeedError),
}

/// The client a fake agent plays.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Played {
    Codex,
    Claude,
}

/// One client's script.
enum Script {
    Codex(script::AgentScript),
    Claude(claude::ClaudeScript),
}

/// What a script asks of the shell, whichever client it plays.
enum Step {
    Send(Value),
    Call(Call),
}

pub struct FakeAgent {
    /// The gateway's root, from where the daemon pointed the client.
    gateway: String,
    script: Script,
}

impl FakeAgent {
    /// `arguments` are those the daemon gave the client.
    pub fn new(
        played: Played,
        env: &HostEnv,
        data_dir: &Path,
        arguments: &[String],
    ) -> Result<Self, AgentError> {
        let own = Run(Seeder::next_run(data_dir)?);
        match played {
            Played::Codex => {
                let base = arguments
                    .iter()
                    .find_map(|argument| {
                        let rest = argument.split_once("base_url=\"")?.1;
                        Some(rest.split_once('"')?.0.to_owned())
                    })
                    .ok_or(AgentError::NoGateway)?;
                Ok(Self {
                    gateway: Self::root(&base, "/codex"),
                    script: Script::Codex(script::AgentScript::new(own)),
                })
            }
            Played::Claude => {
                let base = env.get("ANTHROPIC_BASE_URL").ok_or(AgentError::NoGateway)?;
                let cwd = std::env::current_dir()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_default();
                let script =
                    claude::ClaudeScript::new(own, arguments, cwd).ok_or(AgentError::NoSession)?;
                Ok(Self {
                    gateway: Self::root(base, "/anthropic"),
                    script: Script::Claude(script),
                })
            }
        }
    }

    fn root(base: &str, mount: &str) -> String {
        base.trim_end_matches('/')
            .strip_suffix(mount)
            .unwrap_or(base)
            .trim_end_matches('/')
            .to_owned()
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
            let mut pending = self.receive(&message);
            while !pending.is_empty() {
                let mut next = Vec::new();
                for step in pending {
                    match step {
                        Step::Send(value) => Self::write(&mut stdout, &value)?,
                        Step::Call(call) => {
                            let reply = runtime.block_on(self.call(&http, &mut stdout, call));
                            next.extend(self.replied(reply));
                        }
                    }
                }
                pending = next;
            }
        }
        Ok(())
    }

    fn receive(&mut self, message: &Value) -> Vec<Step> {
        match &mut self.script {
            Script::Codex(script) => script
                .receive(message)
                .into_iter()
                .map(Self::codex_step)
                .collect(),
            Script::Claude(script) => script
                .receive(message)
                .into_iter()
                .map(Self::claude_step)
                .collect(),
        }
    }

    fn replied(&mut self, reply: Result<String, String>) -> Vec<Step> {
        match &mut self.script {
            Script::Codex(script) => script
                .replied(reply)
                .into_iter()
                .map(Self::codex_step)
                .collect(),
            Script::Claude(script) => script
                .replied(reply)
                .into_iter()
                .map(Self::claude_step)
                .collect(),
        }
    }

    fn codex_step(out: script::Out) -> Step {
        match out {
            script::Out::Send(value) => Step::Send(value),
            script::Out::Call {
                session,
                numbers,
                prompt,
            } => Step::Call(Scenario::step_call(
                Client::Codex,
                &Caller::Unidentified,
                session,
                numbers,
                "gpt-5.5-codex",
                &prompt,
                0,
            )),
        }
    }

    fn claude_step(out: claude::Out) -> Step {
        match out {
            claude::Out::Send(value) => Step::Send(value),
            claude::Out::Call(call) => Step::Call(call),
        }
    }

    fn write(stdout: &mut impl std::io::Write, value: &Value) -> Result<(), AgentError> {
        let mut line = value.to_string();
        line.push('\n');
        stdout
            .write_all(line.as_bytes())
            .and_then(|()| stdout.flush())
            .map_err(AgentError::Stdio)
    }

    /// The text a reply's SSE event adds, in the wire the client speaks.
    fn text(&self, kind: &str, event: &Value) -> Option<String> {
        let text = match (&self.script, kind) {
            (Script::Codex(_), "response.output_text.delta") => event["delta"].as_str(),
            (Script::Claude(_), "content_block_delta") => event["delta"]["text"].as_str(),
            (Script::Codex(_) | Script::Claude(_), _) => None,
        };
        text.map(str::to_owned)
    }

    fn delta(&self, text: &str) -> Value {
        match &self.script {
            Script::Codex(script) => script.delta(text),
            Script::Claude(script) => script.delta(text),
        }
    }

    /// Send the turn's model call as the client would and stream its reply text.
    async fn call(
        &self,
        http: &reqwest::Client,
        stdout: &mut impl std::io::Write,
        call: Call,
    ) -> Result<String, String> {
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
                if let Some(text) = self.text(kind, &event) {
                    reply.push_str(&text);
                    if written.is_ok() {
                        written = Self::write(stdout, &self.delta(&text));
                    }
                }
            });
        }
        written.map_err(|error| error.to_string())?;
        Ok(reply)
    }
}
