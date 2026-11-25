//! The calls the dev instance's traffic and fake agents send through its
//! gateway, as pure data, with the headers each harness really sends.
//! Nothing here performs I/O.

use crate::dev::work::{ToolRun, Toolset};
use serde_json::{Value, json};

/// One HTTP call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// What the call stands for, printed as it is sent.
    pub label: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Call {
    /// The call, asking the dev provider for a reply other than plain text
    /// (`tools`, `long`, `slow`, `cut`, `error`, `rate_limited`). The header
    /// is forwarded but never recorded, so nothing shown names it.
    pub fn asking(mut self, reply: &str) -> Self {
        self.headers.push(("x-dev-reply".into(), reply.into()));
        self.label = format!("{reply}: {}", self.label);
        self
    }
}

/// Identifiers for one seed run: each run gets its own sessions, so runs
/// add history instead of piling onto the same sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run(pub u32);

impl Run {
    /// A UUID whose short form (its first eight digits) differs per
    /// session, as real ones do.
    pub(in crate::dev) fn uuid(self, n: u32) -> String {
        format!(
            "{:04x}{:04x}-0000-4000-8000-{:012x}",
            n & 0xffff,
            self.0 & 0xffff,
            u64::from(self.0) * 0x1_0000 + u64::from(n)
        )
    }

    /// The run and number a [`Run::uuid`] was made from, if `text` is one.
    pub(in crate::dev) fn parse_uuid(text: &str) -> Option<(Self, u32)> {
        let last = text.strip_prefix(text.get(..24)?)?;
        let value = u64::from_str_radix(last, 16).ok()?;
        let run = Self(u32::try_from(value >> 16).ok()?);
        let n = u32::try_from(value & 0xffff).ok()?;
        (run.uuid(n) == text).then_some((run, n))
    }
}

/// The headers of one harness's session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::dev) enum Client {
    ClaudeCode,
    Codex,
    Pi,
    OpenCode,
    /// Claude Code pointed at DeepSeek's Anthropic-compatible API: its own
    /// headers, on the DeepSeek route.
    DeepSeek,
}

impl Client {
    /// The tools this client declares and calls.
    pub(in crate::dev) fn toolset(self) -> Toolset {
        match self {
            Self::ClaudeCode | Self::DeepSeek => Toolset::ClaudeCode,
            Self::Codex => Toolset::Codex,
            Self::Pi => Toolset::Pi,
            Self::OpenCode => Toolset::OpenCode,
        }
    }
}

/// A call Claude Code makes beside a turn's work, with no turn of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::dev) enum Side {
    /// A short title for the conversation, from a small model.
    Title,
    /// A one-token request that reads the subscription's remaining quota.
    QuotaProbe,
}

/// Who sends a call.
#[derive(Debug, Clone)]
pub(in crate::dev) enum Caller {
    Unidentified,
    Login(&'static str),
    Token(String),
}

impl Caller {
    fn headers(&self) -> Vec<(String, String)> {
        match self {
            Self::Unidentified => vec![],
            // The dev daemon listens on loopback, where a request can carry
            // the header Tailscale Serve would add.
            Self::Login(login) => vec![("tailscale-user-login".into(), (*login).into())],
            Self::Token(token) => vec![("x-ren-user-token".into(), token.clone())],
        }
    }
}

/// Invented prompts about an invented project ("acme"), so the slot's
/// history never holds anyone's real work. One is long, for layouts that must
/// cut text.
pub(in crate::dev) const PROMPTS: [&str; 8] = [
    "Why does the release build of acme-web fail on the CI runner but not locally?",
    "Add a retry with backoff to the acme upload client, and test it.",
    "Rename the config field `listen_addr` to `listen` across the acme services.",
    "Explain what the acme job queue does when a worker stops responding.",
    "Write a migration that adds an index on orders(created_at).",
    "Summarize the open review comments on the checkout PR.",
    "Why should `acme-billing` depend on `acme-inventory`? Aren't these two separate concerns? Why should a billing crate know anything about stock levels at all, and what breaks if we cut that dependency?",
    "Find where the order list is sorted and make it newest first.",
];

impl Call {
    /// A call of `client`'s session `session`, turn `turn`, after `done` of
    /// the turn's tool calls: its history carries each of them and its
    /// result, as the client sends them back.
    pub(in crate::dev) fn step(
        client: Client,
        caller: &Caller,
        run: Run,
        (session, turn): (u32, u32),
        model: &str,
        prompt: &str,
        done: usize,
    ) -> Call {
        let (session_id, turn_id) = (run.uuid(session), run.uuid(turn));
        let toolset = client.toolset();
        let mut headers = caller.headers();
        let (path, body, name) = match client {
            Client::ClaudeCode | Client::DeepSeek => {
                let body = Self::messages(model, prompt, toolset, &session_id, done);
                headers.extend(Self::claude_code_headers(session_id, Some(turn_id)));
                match client {
                    Client::DeepSeek => ("/deepseek/v1/messages", body, "claude code via deepseek"),
                    Client::ClaudeCode | Client::Codex | Client::Pi | Client::OpenCode => {
                        ("/anthropic/v1/messages", body, "claude code")
                    }
                }
            }
            Client::Codex => {
                let body = Self::responses(model, prompt, toolset, &session_id, done);
                headers.extend([
                    ("session-id".into(), session_id.clone()),
                    ("thread-id".into(), session_id.clone()),
                    (
                        "x-codex-turn-metadata".into(),
                        json!({ "session_id": session_id, "turn_id": turn_id }).to_string(),
                    ),
                    ("originator".into(), "codex_cli_rs".into()),
                ]);
                ("/codex/responses", body, "codex")
            }
            Client::Pi => {
                let body = Self::messages(model, prompt, toolset, &session_id, done);
                headers.extend([
                    ("x-ren-pi-session-id".into(), session_id),
                    ("x-ren-pi-turn-id".into(), turn_id),
                ]);
                ("/anthropic/v1/messages", body, "pi")
            }
            Client::OpenCode => {
                // opencode's own shape: `ses_`, then 26 letters and digits.
                let id = format!(
                    "ses_{:012x}{session:08x}OpnCde",
                    u64::from(run.0) * 0x9E37 + 0x7A1F
                );
                headers.push(("x-opencode-session-id".into(), id.clone()));
                (
                    "/openai/v1/responses",
                    Self::responses(model, prompt, toolset, &id, done),
                    "opencode",
                )
            }
        };
        headers.push(("content-type".into(), "application/json".into()));
        let step = if done > 0 {
            format!(" (after {done} tools)")
        } else {
            String::new()
        };
        Call {
            label: format!("{name}: {}{step}", Self::short(prompt)),
            path: path.into(),
            headers,
            body: body.to_string(),
        }
    }

    /// A Claude Code turn's model call in a session the caller names: the
    /// dev slot's fake `claude`, whose session Ren chose.
    pub(in crate::dev) fn claude_turn(session: &str, prompt_id: &str, prompt: &str) -> Call {
        let mut headers = Caller::Unidentified.headers();
        headers.extend(Self::claude_code_headers(
            session.to_owned(),
            Some(prompt_id.to_owned()),
        ));
        headers.push(("content-type".into(), "application/json".into()));
        let body = Self::messages(
            "claude-sonnet-5-5",
            prompt,
            Client::ClaudeCode.toolset(),
            session,
            0,
        );
        Call {
            label: format!("claude code: {}", Self::short(prompt)),
            path: "/anthropic/v1/messages".into(),
            headers,
            body: body.to_string(),
        }
    }

    /// A Pi turn's model call in a session the caller names, with the
    /// headers Ren's extension sends: the dev slot's fake `pi`.
    pub(in crate::dev) fn pi_turn(session: &str, turn: &str, prompt: &str) -> Call {
        let mut headers = Caller::Unidentified.headers();
        headers.extend([
            ("x-ren-pi-session-id".into(), session.to_owned()),
            ("x-ren-pi-turn-id".into(), turn.to_owned()),
            ("content-type".into(), "application/json".into()),
        ]);
        let body = Self::messages(
            "claude-sonnet-5-5",
            prompt,
            Client::Pi.toolset(),
            session,
            0,
        );
        Call {
            label: format!("pi: {}", Self::short(prompt)),
            path: "/anthropic/v1/messages".into(),
            headers,
            body: body.to_string(),
        }
    }

    /// An opencode turn's model call in the session it names (`ses_…`), as
    /// opencode sends it on its `openai` provider: the dev slot's fake
    /// `opencode`.
    pub(in crate::dev) fn opencode_turn(session: &str, prompt: &str) -> Call {
        let mut headers = Caller::Unidentified.headers();
        headers.extend([
            ("x-opencode-session-id".into(), session.to_owned()),
            ("content-type".into(), "application/json".into()),
        ]);
        let body = Self::responses("gpt-5.5", prompt, Client::OpenCode.toolset(), session, 0);
        Call {
            label: format!("opencode: {}", Self::short(prompt)),
            path: "/openai/v1/responses".into(),
            headers,
            body: body.to_string(),
        }
    }

    /// Claude Code's side call `side` in session `session`: its session's
    /// headers, but no turn.
    pub(in crate::dev) fn side(
        side: Side,
        caller: &Caller,
        run: Run,
        session: u32,
        prompt: &str,
    ) -> Call {
        let mut headers = caller.headers();
        headers.extend(Self::claude_code_headers(run.uuid(session), None));
        headers.push(("content-type".into(), "application/json".into()));
        let model = "claude-haiku-5-5";
        let call = |label: String, body: Value| Call {
            label,
            path: "/anthropic/v1/messages".into(),
            headers: headers.clone(),
            body: body.to_string(),
        };
        match side {
            Side::Title => call(
                format!("claude code title: {}", Self::short(prompt)),
                json!({ "model": model, "max_tokens": 512, "stream": true,
                        "system": [{ "type": "text", "text": "Write a title of at most six words for this coding conversation. Reply with the title only." }],
                        "messages": [{ "role": "user", "content": [{ "type": "text", "text": prompt }] }] }),
            )
            .asking("title"),
            Side::QuotaProbe => call(
                "claude code quota probe".into(),
                json!({ "model": model, "max_tokens": 1,
                        "messages": [{ "role": "user", "content": "quota" }] }),
            ),
        }
    }

    /// The headers Claude Code sends: its session, and its turn (a prompt
    /// id) on calls made for one.
    fn claude_code_headers(session: String, turn: Option<String>) -> Vec<(String, String)> {
        let mut headers = vec![("x-claude-code-session-id".into(), session)];
        if let Some(turn) = turn {
            headers.push(("x-claude-code-prompt-id".into(), turn));
        }
        headers.extend([
            ("x-app".into(), "cli".into()),
            (
                "user-agent".into(),
                "claude-cli/2.1.316 (external, cli)".into(),
            ),
            ("anthropic-version".into(), "2023-06-01".into()),
        ]);
        headers
    }

    fn short(text: &str) -> String {
        match text.char_indices().nth(48) {
            Some((cut, _)) => format!("{}…", &text[..cut]),
            None => text.to_owned(),
        }
    }

    /// A Messages request as coding agents send one: a cached system prompt,
    /// the client's tools, thinking on the models that offer it, and the
    /// turn's first `done` tool calls with their results.
    fn messages(model: &str, prompt: &str, toolset: Toolset, session: &str, done: usize) -> Value {
        let mut messages =
            vec![json!({ "role": "user", "content": [{ "type": "text", "text": prompt }] })];
        let work = ToolRun::new(toolset, prompt, session);
        for index in 0..done {
            let step = work.step(index);
            messages.push(json!({ "role": "assistant", "content": [
                { "type": "text", "text": step.lead },
                { "type": "tool_use", "id": step.id, "name": step.name, "input": step.input },
            ] }));
            messages.push(json!({ "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": step.id, "content": step.output,
                  "is_error": step.failed },
            ] }));
        }
        let mut body = json!({ "model": model, "max_tokens": 32000, "stream": true,
            "metadata": { "user_id": session },
            "system": [{ "type": "text", "text": "You are a coding agent working in the user's repository.",
                         "cache_control": { "type": "ephemeral" } }],
            "tools": toolset.messages_tools(),
            "messages": messages });
        if model.starts_with("claude-opus") || model.starts_with("claude-sonnet") {
            body["thinking"] = json!({ "type": "enabled", "budget_tokens": 31999 });
        }
        body
    }

    /// A Responses request as Codex sends one: reasoning with a summary, a
    /// prompt cache key for its session, its tools, and the turn's first
    /// `done` tool calls with their outputs.
    fn responses(
        model: &str,
        prompt: &str,
        toolset: Toolset,
        cache_key: &str,
        done: usize,
    ) -> Value {
        let mut input = vec![json!({ "type": "message", "role": "user",
                                     "content": [{ "type": "input_text", "text": prompt }] })];
        let work = ToolRun::new(toolset, prompt, cache_key);
        for index in 0..done {
            let step = work.step(index);
            input.push(
                json!({ "type": "function_call", "call_id": step.id, "name": step.name,
                               "arguments": step.input.to_string() }),
            );
            input.push(json!({ "type": "function_call_output", "call_id": step.id,
                               "output": step.output }));
        }
        json!({ "model": model, "stream": true, "prompt_cache_key": cache_key,
                "reasoning": { "effort": "medium", "summary": "auto" },
                "include": ["reasoning.encrypted_content"],
                "tools": toolset.responses_tools(),
                "input": input })
    }
}

#[cfg(test)]
mod tests {
    use super::{Call, Caller, Client, Run};

    #[test]
    fn runs_get_their_own_sessions() {
        let session = |run| {
            Call::step(
                Client::ClaudeCode,
                &Caller::Unidentified,
                Run(run),
                (1, 101),
                "claude-opus-5-5",
                "Explain the acme job queue.",
                0,
            )
            .headers
            .iter()
            .find(|(name, _)| name == "x-claude-code-session-id")
            .unwrap()
            .1
            .clone()
        };
        assert_ne!(session(1), session(2));
        assert_eq!(session(1), session(1));
    }
}
