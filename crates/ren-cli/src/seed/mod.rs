//! Invented history for a deployment's empty store (docs/plans/deployments.md,
//! phase 1): sessions from each client Ren attributes (Claude Code, Codex,
//! Pi, opencode), with turns and tool calls, several callers, failures, and
//! long prompts, all about an invented project ("acme"). A [`Seed`] decides
//! everything, so the same seed always invents the same history.
//!
//! Each [`InventedCall`] is what the gateway would hand the recorder: the
//! route the call took, the headers attribution reads, its caller, and the
//! request and answer bytes in the client's own wire format. The store works
//! sessions, turns, tools, and tokens out of those bytes, as it does for real
//! calls. Pure: `ren seed` writes the calls.

#[cfg(test)]
mod purity;

use http::{HeaderMap, HeaderName, HeaderValue};
use ren_harness::Harness;
use ren_network::tailscale::Login;
use ren_protocol::{
    encoding::ResponseFormat,
    route::{Api, Endpoint, Provider},
};
use ren_recorder::caller::{Caller, Proof, UserId};
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;

/// The number an invented history is made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seed(u64);

impl std::str::FromStr for Seed {
    type Err = std::num::ParseIntError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(Self)
    }
}

/// Invented history: its calls, oldest first.
#[derive(Debug)]
pub struct History {
    calls: Vec<InventedCall>,
}

/// One invented call, as the gateway would record it.
#[derive(Debug)]
pub struct InventedCall {
    /// How long before the history's end the call started.
    pub ago: Duration,
    /// How long the call took.
    pub took: Duration,
    pub api: Api,
    pub endpoint: Endpoint,
    /// The harness the call's route expects, for attribution.
    pub harness: Option<Harness>,
    /// The headers attribution reads.
    pub headers: HeaderMap,
    pub caller: Caller,
    pub request: Vec<u8>,
    pub answer: Answer,
}

/// What an invented call got back.
#[derive(Debug)]
pub struct Answer {
    pub status: u16,
    pub format: ResponseFormat,
    pub body: Vec<u8>,
    pub ending: Ending,
}

/// How an answer ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// The body reached its end.
    Complete,
    /// The stream broke off after its head.
    Cut,
}

/// A deterministic source of choices (SplitMix64): the seed's only
/// randomness.
struct Dice(u64);

impl Dice {
    fn roll(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `low..=high`.
    fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.roll() % (high - low + 1)
    }

    /// One of `items`, which is never empty.
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        let len = u64::try_from(items.len()).unwrap_or(u64::MAX);
        let index = usize::try_from(self.roll() % len).unwrap_or(0);
        &items[index]
    }

    fn uuid(&mut self) -> Uuid {
        let high = u128::from(self.roll());
        let low = u128::from(self.roll());
        uuid::Builder::from_random_bytes(((high << 64) | low).to_be_bytes()).into_uuid()
    }

    fn seconds(&mut self, low: u64, high: u64) -> Duration {
        Duration::from_secs(self.between(low, high))
    }
}

/// The clients whose sessions are invented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Client {
    ClaudeCode,
    Codex,
    Pi,
    OpenCode,
}

/// How a client's calls are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wire {
    Messages,
    Responses,
}

/// Who sends a session's calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Who {
    Anyone,
    /// A user proven by their Tailscale login.
    Alice,
    /// A user proven by its token.
    Ci,
    /// A tailnet login no user lists.
    Carol,
}

/// What a tool step does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Read,
    Run,
    Edit,
}

/// One tool call a turn makes, and what it returned.
struct Step {
    id: String,
    name: &'static str,
    input: Value,
    lead: &'static str,
    output: String,
}

/// What one call of a turn answers: a tool call, or the turn's reply.
enum Reply<'a> {
    Tool(&'a Step),
    Text(String),
}

/// What one call asks: its history so far, as the client sends it.
struct Ask<'a> {
    model: &'static str,
    session: &'a str,
    earlier: &'a [(String, String)],
    prompt: &'a str,
    steps: &'a [Step],
}

/// Invented prompts about an invented project, one long enough to be cut.
const PROMPTS: [&str; 10] = [
    "Why does the release build of acme-web fail on the CI runner but not locally?",
    "Add a retry with backoff to the acme upload client, and test it.",
    "Rename the config field `listen_addr` to `listen` across the acme services.",
    "Explain what the acme job queue does when a worker stops responding.",
    "Write a migration that adds an index on orders(created_at).",
    "Summarize the open review comments on the checkout PR.",
    "Why should `acme-billing` depend on `acme-inventory`? Aren't these two separate concerns? Why should a billing crate know anything about stock levels at all, and what breaks if we cut that dependency?",
    "Find where the order list is sorted and make it newest first.",
    "The nightly export job ran twice last night. Find out why and stop it.",
    "Make `acme-cli orders list` print a table instead of JSON.",
];

const FILES: [&str; 8] = [
    "src/invoice.rs",
    "src/stock.rs",
    "src/checkout.rs",
    "src/upload/client.rs",
    "src/queue/worker.rs",
    "migrations/0042_orders.sql",
    "src/config.rs",
    "src/export.rs",
];

const COMMANDS: [&str; 5] = [
    "cargo test -p acme-billing",
    "cargo build --release",
    "rg -n listen_addr",
    "git log --oneline -5",
    "cargo clippy --all-targets",
];

const REPLIES: [&str; 6] = [
    "The build uses the runner's older linker; pinning it in the toolchain file fixes the release build.",
    "Done: the upload client retries three times with exponential backoff, and a test covers a server that fails twice.",
    "Renamed in all four services and the example configs; the tests pass.",
    "A worker that stops sending heartbeats is marked lost after 30 seconds and its job goes back on the queue.",
    "Added the migration and checked the query plan: the order list now uses the index.",
    "The export runs from both the cron entry and the scheduler; I removed the cron entry.",
];

impl History {
    /// The history `seed` invents.
    pub fn invent(seed: Seed) -> Self {
        let mut dice = Dice(seed.0);
        let mut calls = Vec::new();
        let sessions = dice.between(10, 14);
        for number in 0..sessions {
            let client = *dice.pick(&[
                Client::ClaudeCode,
                Client::ClaudeCode,
                Client::Codex,
                Client::Pi,
                Client::OpenCode,
            ]);
            let who = match number {
                0 => Who::Alice,
                1 => Who::Ci,
                2 => Who::Carol,
                _ => Who::Anyone,
            };
            let start = dice.seconds(600, 3 * 24 * 3600);
            calls.extend(Self::session(&mut dice, client, who, start));
        }
        calls.extend(Self::failures(&mut dice));
        calls.extend(Self::long(&mut dice));
        calls.sort_by_key(|call| std::cmp::Reverse(call.ago));
        Self { calls }
    }

    /// The calls, oldest first.
    pub fn calls(&self) -> &[InventedCall] {
        &self.calls
    }

    /// A session of one to four turns, starting `start` ago.
    fn session(dice: &mut Dice, client: Client, who: Who, start: Duration) -> Vec<InventedCall> {
        let session = client.session_id(dice);
        let model = client.model(dice);
        let mut at = start;
        let mut calls = Vec::new();
        if client == Client::ClaudeCode {
            calls.push(InventedCall::title(dice, &session, who, at));
        }
        let mut earlier: Vec<(String, String)> = Vec::new();
        for _ in 0..dice.between(1, 4) {
            let prompt = (*dice.pick(&PROMPTS[..6])).to_owned();
            let turn = dice.uuid().to_string();
            let steps: Vec<Step> = (0..dice.between(0, 3)).map(|_| client.step(dice)).collect();
            for done in 0..=steps.len() {
                let ask = Ask {
                    model,
                    session: &session,
                    earlier: &earlier,
                    prompt: &prompt,
                    steps: &steps[..done],
                };
                let reply = match steps.get(done) {
                    Some(step) => Reply::Tool(step),
                    None => Reply::Text((*dice.pick(&REPLIES)).to_owned()),
                };
                let call = client.call(dice, who, (&session, &turn), &ask, &reply, at);
                at = at.saturating_sub(call.took + dice.seconds(2, 20));
                calls.push(call);
            }
            earlier.push((prompt, (*dice.pick(&REPLIES)).to_owned()));
            at = at.saturating_sub(dice.seconds(60, 900));
        }
        calls
    }

    /// An upstream error, a rate limit, and a stream that broke off.
    fn failures(dice: &mut Dice) -> Vec<InventedCall> {
        let session = Client::ClaudeCode.session_id(dice);
        let turn = dice.uuid().to_string();
        let ask = Ask {
            model: "claude-opus-5-5",
            session: &session,
            earlier: &[],
            prompt: PROMPTS[7],
            steps: &[],
        };
        let mut at = dice.seconds(3600, 24 * 3600);
        let mut calls = Vec::new();
        for (status, kind) in [(529, "overloaded_error"), (429, "rate_limit_error")] {
            let mut call = Client::ClaudeCode.call(
                dice,
                Who::Anyone,
                (&session, &turn),
                &ask,
                &Reply::Text(String::new()),
                at,
            );
            call.took = Duration::from_millis(dice.between(80, 900));
            call.answer = Answer {
                status,
                format: ResponseFormat::Json,
                body: json!({ "type": "error", "error": { "type": kind, "message": "Invented for this history." } })
                    .to_string()
                    .into_bytes(),
                ending: Ending::Complete,
            };
            at = at.saturating_sub(dice.seconds(5, 30));
            calls.push(call);
        }
        let mut cut = Client::ClaudeCode.call(
            dice,
            Who::Anyone,
            (&session, &turn),
            &ask,
            &Reply::Text(REPLIES[0].to_owned()),
            at,
        );
        let kept = cut.answer.body.len() / 2;
        cut.answer.body.truncate(kept);
        cut.answer.ending = Ending::Cut;
        calls.push(cut);
        calls
    }

    /// A pasted test failure as the prompt, answered at length.
    fn long(dice: &mut Dice) -> Vec<InventedCall> {
        let frames: String = [
            "acme_billing::invoice::Invoice::issue (src/invoice.rs:212)",
            "acme_inventory::stock::StockLevel::require (src/stock.rs:61)",
            "acme_orders::checkout::Checkout::confirm (src/checkout.rs:97)",
        ]
        .iter()
        .cycle()
        .take(40)
        .enumerate()
        .map(|(n, frame)| format!("  {n:>2}: {frame}\n"))
        .collect();
        let prompt = format!(
            "This test started failing after the inventory refactor:\n\n\
             thread 'invoice::tests::an_invoice_for_a_paid_order' panicked at src/invoice.rs:214:9:\n\
             called `Result::unwrap()` on an `Err` value: OutOfStock {{ sku: \"SKU-4471\", requested: 3, available: 0 }}\n\
             stack backtrace:\n{frames}\n{}",
            PROMPTS[6]
        );
        let reply = REPLIES
            .iter()
            .cycle()
            .take(24)
            .copied()
            .collect::<Vec<_>>()
            .join("\n\n");
        [Client::ClaudeCode, Client::Codex]
            .into_iter()
            .map(|client| {
                let session = client.session_id(dice);
                let turn = dice.uuid().to_string();
                let ask = Ask {
                    model: client.model(dice),
                    session: &session,
                    earlier: &[],
                    prompt: &prompt,
                    steps: &[],
                };
                let at = dice.seconds(600, 48 * 3600);
                client.call(
                    dice,
                    Who::Anyone,
                    (&session, &turn),
                    &ask,
                    &Reply::Text(reply.clone()),
                    at,
                )
            })
            .collect()
    }
}

impl InventedCall {
    /// Claude Code's title for a session: a small model's call with the
    /// session's headers and no turn.
    fn title(dice: &mut Dice, session: &str, who: Who, at: Duration) -> Self {
        let request = json!({ "model": "claude-haiku-5-5", "max_tokens": 512, "stream": true,
            "system": [{ "type": "text", "text": "Write a title of at most six words for this coding conversation." }],
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": PROMPTS[0] }] }] });
        let mut headers = HeaderMap::new();
        Client::put(&mut headers, "x-claude-code-session-id", session);
        Self {
            ago: at,
            took: Duration::from_millis(dice.between(400, 1500)),
            api: Provider::Anthropic.native_api(),
            endpoint: Endpoint::Messages,
            harness: Some(Harness::ClaudeCode),
            headers,
            caller: who.caller(),
            request: request.to_string().into_bytes(),
            answer: Wire::Messages.answer(
                dice,
                "claude-haiku-5-5",
                &Reply::Text("Fix the release build".into()),
            ),
        }
    }
}

impl Client {
    /// The session id the client sends, in its own shape.
    fn session_id(self, dice: &mut Dice) -> String {
        match self {
            Self::ClaudeCode | Self::Codex | Self::Pi => dice.uuid().to_string(),
            // opencode's own shape: `ses_`, then 26 letters and digits.
            Self::OpenCode => format!("ses_{}", &dice.uuid().simple().to_string()[..26]),
        }
    }

    fn model(self, dice: &mut Dice) -> &'static str {
        match self {
            Self::ClaudeCode => dice.pick(&["claude-opus-5-5", "claude-sonnet-5-5"]),
            Self::Codex => "gpt-5.5-codex",
            Self::Pi => "claude-sonnet-5-5",
            Self::OpenCode => "gpt-5.5",
        }
    }

    fn wire(self) -> Wire {
        match self {
            Self::ClaudeCode | Self::Pi => Wire::Messages,
            Self::Codex | Self::OpenCode => Wire::Responses,
        }
    }

    /// The route the client's calls take: its API, endpoint, and the
    /// harness the route expects.
    fn route(self) -> (Api, Option<Harness>) {
        match self {
            Self::ClaudeCode | Self::Pi => {
                (Provider::Anthropic.native_api(), Some(Harness::ClaudeCode))
            }
            Self::Codex => (Provider::Codex.native_api(), Some(Harness::Codex)),
            Self::OpenCode => (Provider::OpenAi.native_api(), None),
        }
    }

    fn tool(self, act: Act) -> &'static str {
        match (self, act) {
            (Self::ClaudeCode, Act::Read) => "Read",
            (Self::ClaudeCode, Act::Edit) => "Edit",
            (Self::ClaudeCode, Act::Run) => "Bash",
            (Self::Codex, Act::Read) => "read_file",
            (Self::Codex, Act::Edit) => "apply_patch",
            (Self::Codex | Self::OpenCode, Act::Run) => "shell",
            (Self::Pi | Self::OpenCode, Act::Read) => "read",
            (Self::Pi | Self::OpenCode, Act::Edit) => "edit",
            (Self::Pi, Act::Run) => "bash",
        }
    }

    fn tools(self) -> Vec<&'static str> {
        let mut names: Vec<_> = [Act::Read, Act::Edit, Act::Run]
            .into_iter()
            .map(|act| self.tool(act))
            .collect();
        names.dedup();
        names
    }

    fn step(self, dice: &mut Dice) -> Step {
        let act = *dice.pick(&[Act::Read, Act::Read, Act::Run, Act::Edit]);
        let file = *dice.pick(&FILES);
        let (input, lead, output) = match act {
            Act::Read => (
                json!({ "path": file }),
                "Let me read the code first.",
                format!("// {file}\nfn main() {{}}\n"),
            ),
            Act::Run => {
                let command = *dice.pick(&COMMANDS);
                (
                    json!({ "command": command }),
                    "Running it to see where it stands.",
                    format!("$ {command}\nfinished in {}s", dice.between(1, 90)),
                )
            }
            Act::Edit => (
                json!({ "path": file, "old": "listen_addr", "new": "listen" }),
                "Making the change.",
                format!("Edited {file}."),
            ),
        };
        Step {
            id: format!("call_{}", &dice.uuid().simple().to_string()[..24]),
            name: self.tool(act),
            input,
            lead,
            output,
        }
    }

    /// The headers the client sends for a turn of a session.
    fn headers(self, session: &str, turn: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        match self {
            Self::ClaudeCode => {
                Self::put(&mut headers, "x-claude-code-session-id", session);
                Self::put(&mut headers, "x-claude-code-prompt-id", turn);
            }
            Self::Codex => {
                Self::put(&mut headers, "session-id", session);
                let metadata = json!({ "session_id": session, "turn_id": turn }).to_string();
                Self::put(&mut headers, "x-codex-turn-metadata", &metadata);
            }
            Self::Pi => {
                Self::put(&mut headers, "x-ren-pi-session-id", session);
                Self::put(&mut headers, "x-ren-pi-turn-id", turn);
            }
            Self::OpenCode => Self::put(&mut headers, "x-opencode-session-id", session),
        }
        headers
    }

    fn put(headers: &mut HeaderMap, name: &'static str, value: &str) {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(HeaderName::from_static(name), value);
        }
    }

    fn call(
        self,
        dice: &mut Dice,
        who: Who,
        (session, turn): (&str, &str),
        ask: &Ask<'_>,
        reply: &Reply<'_>,
        at: Duration,
    ) -> InventedCall {
        let (api, harness) = self.route();
        let wire = self.wire();
        InventedCall {
            ago: at,
            took: Duration::from_millis(dice.between(1_500, 40_000)),
            api,
            endpoint: match wire {
                Wire::Messages => Endpoint::Messages,
                Wire::Responses => Endpoint::Responses,
            },
            harness,
            headers: self.headers(session, turn),
            caller: who.caller(),
            request: wire.request(self, ask).to_string().into_bytes(),
            answer: wire.answer(dice, ask.model, reply),
        }
    }
}

impl Wire {
    /// The request a client sends: its history so far, the steps of this
    /// turn with their results, and its tools.
    fn request(self, client: Client, ask: &Ask<'_>) -> Value {
        match self {
            Self::Messages => {
                let mut messages = Vec::new();
                for (prompt, reply) in ask.earlier {
                    messages.push(
                        json!({ "role": "user", "content": [{ "type": "text", "text": prompt }] }),
                    );
                    messages.push(json!({ "role": "assistant", "content": [{ "type": "text", "text": reply }] }));
                }
                messages.push(
                    json!({ "role": "user", "content": [{ "type": "text", "text": ask.prompt }] }),
                );
                for step in ask.steps {
                    messages.push(json!({ "role": "assistant", "content": [
                        { "type": "text", "text": step.lead },
                        { "type": "tool_use", "id": step.id, "name": step.name, "input": step.input },
                    ] }));
                    messages.push(json!({ "role": "user", "content": [
                        { "type": "tool_result", "tool_use_id": step.id, "content": step.output },
                    ] }));
                }
                let tools: Vec<Value> = client
                    .tools()
                    .into_iter()
                    .map(|name| json!({ "name": name, "description": format!("The {name} tool."), "input_schema": { "type": "object" } }))
                    .collect();
                json!({ "model": ask.model, "max_tokens": 32000, "stream": true,
                    "metadata": { "user_id": ask.session },
                    "system": [{ "type": "text", "text": "You are a coding agent working in the user's repository.",
                                 "cache_control": { "type": "ephemeral" } }],
                    "tools": tools, "messages": messages })
            }
            Self::Responses => {
                let mut input = Vec::new();
                for (prompt, reply) in ask.earlier {
                    input.push(json!({ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": prompt }] }));
                    input.push(json!({ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": reply }] }));
                }
                input.push(json!({ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": ask.prompt }] }));
                for step in ask.steps {
                    input.push(
                        json!({ "type": "function_call", "call_id": step.id, "name": step.name,
                                       "arguments": step.input.to_string() }),
                    );
                    input.push(json!({ "type": "function_call_output", "call_id": step.id, "output": step.output }));
                }
                let tools: Vec<Value> = client
                    .tools()
                    .into_iter()
                    .map(|name| json!({ "type": "function", "name": name, "parameters": { "type": "object" } }))
                    .collect();
                json!({ "model": ask.model, "stream": true, "prompt_cache_key": ask.session,
                        "reasoning": { "effort": "medium", "summary": "auto" },
                        "tools": tools, "input": input })
            }
        }
    }

    /// The streamed answer to a call.
    fn answer(self, dice: &mut Dice, model: &str, reply: &Reply<'_>) -> Answer {
        let input_tokens = dice.between(200, 4_000);
        let cached = dice.between(8_000, 60_000);
        let id = dice.uuid().simple().to_string();
        let (text, tool) = match reply {
            Reply::Tool(step) => (step.lead.to_owned(), Some(*step)),
            Reply::Text(text) => (text.clone(), None),
        };
        let output_tokens = u64::try_from(text.len() / 4).unwrap_or(0) + 20;
        let event = |name: &str, data: Value| format!("event: {name}\ndata: {data}\n\n");
        let mut body = String::new();
        match self {
            Self::Messages => {
                body += &event(
                    "message_start",
                    json!({ "type": "message_start", "message": {
                    "id": format!("msg_01{id}"), "type": "message", "role": "assistant", "model": model,
                    "content": [], "stop_reason": null,
                    "usage": { "input_tokens": input_tokens, "cache_read_input_tokens": cached,
                               "cache_creation_input_tokens": 0, "output_tokens": 1 } } }),
                );
                body += &event(
                    "content_block_start",
                    json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
                );
                body += &event(
                    "content_block_delta",
                    json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": text } }),
                );
                body += &event(
                    "content_block_stop",
                    json!({ "type": "content_block_stop", "index": 0 }),
                );
                if let Some(step) = tool {
                    body += &event(
                        "content_block_start",
                        json!({ "type": "content_block_start", "index": 1,
                        "content_block": { "type": "tool_use", "id": step.id, "name": step.name, "input": {} } }),
                    );
                    body += &event(
                        "content_block_delta",
                        json!({ "type": "content_block_delta", "index": 1,
                        "delta": { "type": "input_json_delta", "partial_json": step.input.to_string() } }),
                    );
                    body += &event(
                        "content_block_stop",
                        json!({ "type": "content_block_stop", "index": 1 }),
                    );
                }
                let stop = if tool.is_some() {
                    "tool_use"
                } else {
                    "end_turn"
                };
                body += &event(
                    "message_delta",
                    json!({ "type": "message_delta",
                    "delta": { "stop_reason": stop, "stop_sequence": null }, "usage": { "output_tokens": output_tokens } }),
                );
                body += &event("message_stop", json!({ "type": "message_stop" }));
            }
            Self::Responses => {
                let mut output = vec![
                    json!({ "type": "message", "id": format!("msg_{id}"), "role": "assistant",
                    "status": "completed", "content": [{ "type": "output_text", "text": text, "annotations": [] }] }),
                ];
                if let Some(step) = tool {
                    output.push(json!({ "type": "function_call", "id": format!("fc_{id}"), "call_id": step.id,
                        "name": step.name, "arguments": step.input.to_string(), "status": "completed" }));
                }
                let response = |status: &str, output: Value, usage: Value| {
                    json!({ "id": format!("resp_{id}"),
                    "object": "response", "model": model, "status": status, "output": output, "usage": usage })
                };
                body += &event(
                    "response.created",
                    json!({ "type": "response.created", "response": response("in_progress", json!([]), Value::Null) }),
                );
                body += &event(
                    "response.output_text.delta",
                    json!({ "type": "response.output_text.delta",
                    "item_id": format!("msg_{id}"), "output_index": 0, "content_index": 0, "delta": text }),
                );
                for (index, item) in output.iter().enumerate() {
                    body += &event(
                        "response.output_item.done",
                        json!({ "type": "response.output_item.done", "output_index": index, "item": item }),
                    );
                }
                // Responses counts cached tokens inside `input_tokens`.
                let usage = json!({ "input_tokens": input_tokens + cached, "input_tokens_details": { "cached_tokens": cached },
                    "output_tokens": output_tokens, "output_tokens_details": { "reasoning_tokens": 0 },
                    "total_tokens": input_tokens + cached + output_tokens });
                body += &event(
                    "response.completed",
                    json!({ "type": "response.completed", "response": response("completed", Value::Array(output), usage) }),
                );
            }
        }
        Answer {
            status: 200,
            format: ResponseFormat::EventStream,
            body: body.into_bytes(),
            ending: Ending::Complete,
        }
    }
}

impl Who {
    /// The caller the gateway would record.
    #[expect(
        clippy::expect_used,
        reason = "invented constants, each parsed by the test below"
    )]
    fn caller(self) -> Caller {
        let user = |id: &str| id.parse::<UserId>().expect("an invented user id parses");
        let login = |login: &str| login.parse::<Login>().expect("an invented login parses");
        match self {
            Self::Anyone => Caller::Unidentified,
            Self::Alice => Caller::User {
                user: user("alice"),
                proof: Proof::Tailscale(login("alice@example.com")),
            },
            Self::Ci => Caller::User {
                user: user("ci"),
                proof: Proof::Token { login: None },
            },
            Self::Carol => Caller::Unlisted(login("carol@example.com")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Ending, History, Seed, Who};
    use ren_harness::Harness;
    use ren_recorder::caller::Caller;

    #[test]
    fn a_seed_always_invents_the_same_history_and_another_seed_another() {
        let one = History::invent(Seed(7));
        let again = History::invent(Seed(7));
        let other = History::invent(Seed(8));
        let bodies = |history: &History| -> Vec<Vec<u8>> {
            history
                .calls()
                .iter()
                .map(|call| call.request.clone())
                .collect()
        };
        assert_eq!(bodies(&one), bodies(&again));
        assert_ne!(bodies(&one), bodies(&other));
    }

    #[test]
    fn the_history_holds_every_client_caller_and_ending_oldest_first() {
        let history = History::invent(Seed(1));
        let calls = history.calls();
        assert!(calls.windows(2).all(|pair| pair[0].ago >= pair[1].ago));
        for request in calls.iter().map(|call| &call.request) {
            serde_json::from_slice::<serde_json::Value>(request).unwrap();
        }
        let sessions: std::collections::BTreeSet<String> = calls
            .iter()
            .filter_map(|call| {
                let attribution = Harness::attribution(call.harness, &call.headers);
                attribution.hint.map(|hint| format!("{:?}", hint.source))
            })
            .collect();
        assert!(sessions.len() >= 2, "{sessions:?}");
        assert!(
            calls
                .iter()
                .any(|call| matches!(call.caller, Caller::User { .. }))
        );
        assert!(
            calls
                .iter()
                .any(|call| matches!(call.caller, Caller::Unlisted(_)))
        );
        assert!(calls.iter().any(|call| call.answer.ending == Ending::Cut));
        assert!(calls.iter().any(|call| call.answer.status == 429));
    }

    #[test]
    fn every_invented_caller_parses() {
        for who in [Who::Anyone, Who::Alice, Who::Ci, Who::Carol] {
            drop(who.caller());
        }
    }
}
