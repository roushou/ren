//! Seed scenarios as pure data: the calls each one sends through the dev
//! gateway, with the headers each harness really sends, so the dev slot's
//! history looks like real use. Nothing here performs I/O.

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
#[derive(Debug, Clone, Copy)]
pub struct Run(pub u32);

impl Run {
    /// A UUID whose short form (its first eight digits) differs per
    /// session, as real ones do.
    fn uuid(self, n: u32) -> String {
        format!(
            "{:04x}{:04x}-0000-4000-8000-{:012x}",
            n & 0xffff,
            self.0 & 0xffff,
            u64::from(self.0) * 0x1_0000 + u64::from(n)
        )
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
            Self::Token(token) => vec![("x-tgv-user-token".into(), token.clone())],
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

    /// One call of `client`'s session `session`, turn `turn`.
    pub(in crate::dev) fn call(
        client: Client,
        caller: &Caller,
        run: Run,
        (session, turn): (u32, u32),
        model: &str,
        prompt: &str,
    ) -> Call {
        let (session_id, turn_id) = (run.uuid(session), run.uuid(turn));
        let mut headers = caller.headers();
        let (path, body, name) = match client {
            Client::ClaudeCode | Client::DeepSeek => {
                headers.extend([
                    ("x-claude-code-session-id".into(), session_id),
                    ("x-claude-code-prompt-id".into(), turn_id),
                    ("x-app".into(), "cli".into()),
                    (
                        "user-agent".into(),
                        "claude-cli/2.1.316 (external, cli)".into(),
                    ),
                    ("anthropic-version".into(), "2023-06-01".into()),
                ]);
                match client {
                    Client::DeepSeek => (
                        "/deepseek/v1/messages",
                        Self::messages(model, prompt),
                        "claude code via deepseek",
                    ),
                    Client::ClaudeCode | Client::Codex | Client::Pi | Client::OpenCode => (
                        "/anthropic/v1/messages",
                        Self::messages(model, prompt),
                        "claude code",
                    ),
                }
            }
            Client::Codex => {
                headers.extend([
                    ("session-id".into(), session_id.clone()),
                    ("thread-id".into(), session_id.clone()),
                    (
                        "x-codex-turn-metadata".into(),
                        json!({ "session_id": session_id, "turn_id": turn_id }).to_string(),
                    ),
                    ("originator".into(), "codex_cli_rs".into()),
                ]);
                ("/codex/responses", Self::responses(model, prompt), "codex")
            }
            Client::Pi => {
                headers.extend([
                    ("x-tgv-pi-session-id".into(), session_id),
                    ("x-tgv-pi-turn-id".into(), turn_id),
                ]);
                (
                    "/anthropic/v1/messages",
                    Self::messages(model, prompt),
                    "pi",
                )
            }
            Client::OpenCode => {
                headers.push((
                    "x-opencode-session-id".into(),
                    // opencode's own shape: `ses_`, then 26 letters and digits.
                    format!(
                        "ses_{:012x}{session:08x}OpnCde",
                        u64::from(run.0) * 0x9E37 + 0x7A1F
                    ),
                ));
                (
                    "/openai/v1/responses",
                    Self::responses(model, prompt),
                    "opencode",
                )
            }
        };
        headers.push(("content-type".into(), "application/json".into()));
        Call {
            label: format!("{name}: {}", Self::short(prompt)),
            path: path.into(),
            headers,
            body: body.to_string(),
        }
    }

    fn short(text: &str) -> String {
        match text.char_indices().nth(48) {
            Some((cut, _)) => format!("{}…", &text[..cut]),
            None => text.to_owned(),
        }
    }

    fn messages(model: &str, prompt: &str) -> Value {
        json!({ "model": model, "max_tokens": 1024, "stream": true,
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": prompt }] }] })
    }

    fn responses(model: &str, prompt: &str) -> Value {
        json!({ "model": model, "stream": true,
                "input": [{ "type": "message", "role": "user",
                            "content": [{ "type": "input_text", "text": prompt }] }] })
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
            Self::call(client, caller, run, ids, model, prompt),
        ]
    }

    fn sessions(run: Run) -> Plan {
        let anyone = Caller::Unidentified;
        let mut calls = Vec::new();
        for (n, prompt) in PROMPTS[..3].iter().enumerate() {
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
                "tgvu_000000000000000000000000000000000000000000000000000000000000dead".into(),
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
        let all = Scenario::All.plan(run, "tgvu_fixture");
        let parts: usize = [
            Scenario::Sessions,
            Scenario::Users,
            Scenario::Long,
            Scenario::Errors,
            Scenario::Websocket,
        ]
        .iter()
        .map(|scenario| {
            let plan = scenario.plan(run, "tgvu_fixture");
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
        let plan = Scenario::Users.plan(Run(1), "tgvu_fixture");
        let sent = |name: &str| {
            plan.calls
                .iter()
                .flat_map(|call| &call.headers)
                .filter(|(header, _)| header == name)
                .count()
        };
        assert_eq!(sent("x-tgv-user-token"), 3);
        assert_eq!(sent("tailscale-user-login"), 3);
        assert!(
            plan.calls
                .iter()
                .any(|call| call.path == "/upstreams/team-key/v1/messages")
        );
    }
}
