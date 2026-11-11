//! Seed scenarios as pure data: the calls each one sends through the dev
//! gateway, with the headers each harness really sends, so the dev slot's
//! history looks like real use. Nothing here performs I/O.

use crate::dev::work::{ToolRun, Toolset};
use clap::ValueEnum;
use serde_json::{Value, json};

/// A set of calls to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Scenario {
    /// Sessions from Claude Code, Codex, Pi, opencode, and DeepSeek, with
    /// turns, tool calls, and follow-ups.
    Sessions,
    /// Named users by Tailscale login and by token, an unlisted login, an
    /// unidentified caller, a refused token, and a user-only entry.
    Users,
    /// Long prompts and a long reply, for layouts that must truncate.
    Long,
    /// An upstream error, a rate limit, a cut stream, and a slow stream.
    Errors,
    /// A Codex WebSocket channel with two calls.
    Websocket,
    /// Every scenario above.
    All,
}

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

/// One WebSocket channel: its handshake headers and the messages it sends,
/// each answered before the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub label: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub messages: Vec<String>,
}

/// What a scenario sends.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub calls: Vec<Call>,
    pub channels: Vec<Channel>,
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
#[derive(Debug, Clone, Copy)]
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

impl Scenario {
    pub fn plan(self, run: Run, token: &str) -> Plan {
        match self {
            Self::Sessions => Self::sessions(run),
            Self::Users => Self::users(run, token),
            Self::Long => Self::long(run),
            Self::Errors => Self::errors(run),
            Self::Websocket => Self::websocket(run),
            Self::All => {
                let mut plan = Plan::default();
                for scenario in [
                    Self::Sessions,
                    Self::Users,
                    Self::Long,
                    Self::Errors,
                    Self::Websocket,
                ] {
                    let part = scenario.plan(run, token);
                    plan.calls.extend(part.calls);
                    plan.channels.extend(part.channels);
                }
                plan
            }
        }
    }

    /// The first call of `client`'s session `session`, turn `turn`.
    pub(in crate::dev) fn call(
        client: Client,
        caller: &Caller,
        run: Run,
        ids: (u32, u32),
        model: &str,
        prompt: &str,
    ) -> Call {
        Self::step_call(client, caller, run, ids, model, prompt, 0)
    }

    /// A call of `client`'s session `session`, turn `turn`, after `done` of
    /// the turn's tool calls: its history carries each of them and its
    /// result, as the client sends them back.
    pub(in crate::dev) fn step_call(
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

    /// Claude Code's side call `side` in session `session`: its session's
    /// headers, but no turn.
    pub(in crate::dev) fn side_call(
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

    /// A turn: a tool call, then its follow-up in the same turn.
    fn turn(
        client: Client,
        caller: &Caller,
        run: Run,
        ids: (u32, u32),
        model: &str,
        prompt: &str,
    ) -> [Call; 2] {
        [
            Self::call(client, caller, run, ids, model, prompt).asking("tools"),
            Self::step_call(client, caller, run, ids, model, prompt, 1),
        ]
    }

    fn sessions(run: Run) -> Plan {
        let anyone = Caller::Unidentified;
        let mut calls = Vec::new();
        calls.push(Self::side_call(Side::QuotaProbe, &anyone, run, 1, ""));
        for (n, prompt) in PROMPTS[..3].iter().enumerate() {
            if n == 0 {
                calls.push(Self::side_call(Side::Title, &anyone, run, 1, prompt));
            }
            let turn = u32::try_from(n).unwrap_or(0) + 101;
            calls.extend(Self::turn(
                Client::ClaudeCode,
                &anyone,
                run,
                (1, turn),
                "claude-opus-5-5",
                prompt,
            ));
        }
        for (n, prompt) in PROMPTS[3..5].iter().enumerate() {
            let turn = u32::try_from(n).unwrap_or(0) + 201;
            calls.extend(Self::turn(
                Client::Codex,
                &anyone,
                run,
                (2, turn),
                "gpt-5.5-codex",
                prompt,
            ));
        }
        calls.extend(Self::turn(
            Client::Pi,
            &anyone,
            run,
            (3, 301),
            "claude-sonnet-5-5",
            PROMPTS[5],
        ));
        calls.push(Self::call(
            Client::OpenCode,
            &anyone,
            run,
            (4, 401),
            "gpt-5.5",
            PROMPTS[7],
        ));
        calls.push(Self::call(
            Client::DeepSeek,
            &anyone,
            run,
            (5, 501),
            "deepseek-chat",
            PROMPTS[1],
        ));
        Plan {
            calls,
            channels: vec![],
        }
    }

    fn users(run: Run, token: &str) -> Plan {
        let alice = Caller::Login("alice@example.com");
        let ci = Caller::Token(token.to_owned());
        let carol = Caller::Login("carol@example.com");
        let mut calls = vec![
            Self::call(
                Client::ClaudeCode,
                &alice,
                run,
                (11, 1101),
                "claude-opus-5-5",
                PROMPTS[0],
            ),
            Self::call(
                Client::Codex,
                &ci,
                run,
                (12, 1201),
                "gpt-5.5-codex",
                PROMPTS[4],
            ),
            Self::call(
                Client::ClaudeCode,
                &carol,
                run,
                (13, 1301),
                "claude-opus-5-5",
                PROMPTS[2],
            ),
            Self::call(
                Client::ClaudeCode,
                &Caller::Unidentified,
                run,
                (14, 1401),
                "claude-opus-5-5",
                PROMPTS[3],
            ),
        ];
        // A token no user has: refused with 401.
        let mut refused = Self::call(
            Client::ClaudeCode,
            &Caller::Token(
                "renu_000000000000000000000000000000000000000000000000000000000000dead".into(),
            ),
            run,
            (15, 1501),
            "claude-opus-5-5",
            PROMPTS[1],
        );
        refused.label = format!("refused token: {}", refused.label);
        calls.push(refused);
        // The ci-only operator-key entry: ci gets in, alice is refused.
        for caller in [&ci, &alice] {
            let mut direct = Self::call(
                Client::ClaudeCode,
                caller,
                run,
                (16, 1601),
                "claude-opus-5-5",
                PROMPTS[6],
            );
            direct.path = "/upstreams/team-key/v1/messages".into();
            direct.label = format!("team-key: {}", direct.label);
            calls.push(direct);
        }
        Plan {
            calls,
            channels: vec![],
        }
    }

    fn long(run: Run) -> Plan {
        let anyone = Caller::Unidentified;
        // A pasted test failure and a question: long, as real prompts with a
        // log in them are.
        let frames: String = [
            "acme_billing::invoice::Invoice::issue (src/invoice.rs:212)",
            "acme_billing::invoice::Invoice::lines (src/invoice.rs:148)",
            "acme_inventory::stock::StockLevel::require (src/stock.rs:61)",
            "acme_inventory::stock::Warehouse::level (src/stock.rs:33)",
            "acme_orders::checkout::Checkout::confirm (src/checkout.rs:97)",
        ]
        .iter()
        .cycle()
        .take(40)
        .enumerate()
        .map(|(n, frame)| format!("  {n:>2}: {frame}\n"))
        .collect();
        let long_prompt = format!(
            "This test started failing after the inventory refactor and I can't see why. \
             Here is the full output:\n\n\
             ---- invoice::tests::an_invoice_for_a_paid_order stdout ----\n\
             thread 'invoice::tests::an_invoice_for_a_paid_order' panicked at src/invoice.rs:214:9:\n\
             called `Result::unwrap()` on an `Err` value: OutOfStock {{ sku: \"SKU-4471\", requested: 3, available: 0 }}\n\
             stack backtrace:\n{frames}\n\
             The order was already paid, so stock shouldn't matter at this point. Why does issuing \
             the invoice check stock at all, and what's the cleanest way to fix it without \
             breaking the checkout flow?"
        );
        Plan {
            calls: vec![
                Self::call(
                    Client::ClaudeCode,
                    &anyone,
                    run,
                    (21, 2101),
                    "claude-opus-5-5",
                    PROMPTS[6],
                ),
                Self::call(
                    Client::ClaudeCode,
                    &anyone,
                    run,
                    (22, 2201),
                    "claude-opus-5-5",
                    &long_prompt,
                )
                .asking("long"),
                Self::call(
                    Client::Codex,
                    &anyone,
                    run,
                    (23, 2301),
                    "gpt-5.5-codex",
                    &long_prompt,
                )
                .asking("long"),
            ],
            channels: vec![],
        }
    }

    fn errors(run: Run) -> Plan {
        let anyone = Caller::Unidentified;
        Plan {
            calls: ["error", "rate_limited", "cut", "slow"]
                .iter()
                .enumerate()
                .map(|(n, reply)| {
                    let n = u32::try_from(n).unwrap_or(0);
                    Self::call(
                        Client::ClaudeCode,
                        &anyone,
                        run,
                        (31, 3101 + n),
                        "claude-opus-5-5",
                        PROMPTS[0],
                    )
                    .asking(reply)
                })
                .collect(),
            channels: vec![],
        }
    }

    fn websocket(run: Run) -> Plan {
        let session = run.uuid(41);
        let message = |turn: u32, prompt: &str| {
            json!({ "type": "response.create", "model": "gpt-5.5-codex",
                    "prompt_cache_key": session, "reasoning": { "effort": "medium", "summary": "auto" },
                    "client_metadata": { "x-codex-turn-metadata": json!({ "session_id": session, "turn_id": run.uuid(turn) }).to_string() },
                    "input": [{ "type": "message", "role": "user",
                                "content": [{ "type": "input_text", "text": prompt }] }] })
            .to_string()
        };
        Plan {
            calls: vec![],
            channels: vec![Channel {
                label: "codex websocket: two calls".into(),
                path: "/codex/responses".into(),
                headers: vec![
                    ("session-id".into(), session.clone()),
                    ("thread-id".into(), session.clone()),
                    ("originator".into(), "codex_cli_rs".into()),
                ],
                messages: vec![message(4101, PROMPTS[3]), message(4102, PROMPTS[7])],
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Run, Scenario};
    use serde_json::Value;

    #[test]
    fn all_is_every_scenario_and_bodies_are_json() {
        let run = Run(3);
        let all = Scenario::All.plan(run, "renu_fixture");
        let parts: usize = [
            Scenario::Sessions,
            Scenario::Users,
            Scenario::Long,
            Scenario::Errors,
            Scenario::Websocket,
        ]
        .iter()
        .map(|scenario| {
            let plan = scenario.plan(run, "renu_fixture");
            plan.calls.len() + plan.channels.len()
        })
        .sum();
        assert_eq!(all.calls.len() + all.channels.len(), parts);
        for call in &all.calls {
            let body: Value = serde_json::from_str(&call.body).unwrap();
            assert!(body["model"].is_string(), "{}", call.label);
            assert!(call.path.starts_with('/'));
        }
    }

    #[test]
    fn runs_get_their_own_sessions() {
        let session = |run| {
            Scenario::Sessions.plan(Run(run), "t").calls[0]
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

    #[test]
    fn the_users_scenario_sends_the_token_and_logins() {
        let plan = Scenario::Users.plan(Run(1), "renu_fixture");
        let sent = |name: &str| {
            plan.calls
                .iter()
                .flat_map(|call| &call.headers)
                .filter(|(header, _)| header == name)
                .count()
        };
        assert_eq!(sent("x-ren-user-token"), 3);
        assert_eq!(sent("tailscale-user-login"), 3);
        assert!(
            plan.calls
                .iter()
                .any(|call| call.path == "/upstreams/team-key/v1/messages")
        );
    }
}
