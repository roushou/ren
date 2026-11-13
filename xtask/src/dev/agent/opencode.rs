//! What the dev slot's fake opencode says in its ACP mode (`opencode acp`),
//! as a pure state machine: Ren's JSON-RPC messages in, lines out, and the
//! model calls to make. It names its own sessions (`ses_…`) and lists the
//! ones it made in a folder, so a run can be resumed in a later process; the
//! shell keeps that list in the slot's data folder. A prompt about testing,
//! running, or building runs `shell`, and one about fixing, editing,
//! changing, adding, or writing runs `write`; each asks first when the run's
//! permission rules say so (as Ren's approval policy sets them). Then one
//! model call goes through the gateway under the session's header, its reply
//! streams back, and the prompt is answered.

use crate::dev::seed::{Call, Run, Scenario};
use serde_json::{Value, json};

/// What the shell does next.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// Write this line to stdout.
    Send(Value),
    /// Make the turn's model call; its reply goes to
    /// [`OpenCodeScript::delta`] and [`OpenCodeScript::replied`].
    Call(Call),
    /// Keep this session of this folder for later processes.
    Remember { cwd: String, session: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ask {
    Shell,
    Write,
}

impl Ask {
    fn of(prompt: &str) -> Option<Self> {
        let prompt = prompt.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|word| prompt.contains(word));
        if has(&["test", "run", "build"]) {
            Some(Self::Shell)
        } else if has(&["fix", "edit", "change", "add", "write"]) {
            Some(Self::Write)
        } else {
            None
        }
    }

    /// The permission rule's action for this tool.
    fn action(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Write => "edit",
        }
    }
}

/// The turn running: the prompt request to answer when it ends.
#[derive(Debug, Clone, PartialEq)]
struct Active {
    request: Value,
    prompt: String,
    index: u32,
    /// The tool waiting on a permission answer, by the request id asked.
    waiting: Option<(u64, Ask)>,
}

/// One fake opencode process.
pub struct OpenCodeScript {
    /// This process's own number, for the ids it makes.
    own: Run,
    /// The folder's sessions earlier processes made, and this one's.
    known: Vec<(String, String)>,
    /// The actions whose permission rules ask first.
    asks: Vec<String>,
    session: Option<String>,
    made: u32,
    turns: u32,
    next_request: u64,
    turn: Option<Active>,
}

impl OpenCodeScript {
    /// The models it offers: invented.
    const MODELS: [(&'static str, &'static str); 3] = [
        ("opencode/dev-pickle", "opencode/Dev Pickle"),
        ("opencode/dev-flash-free", "opencode/Dev Flash Free"),
        ("anthropic/dev-large-1", "anthropic/Dev Large"),
    ];

    /// `known` are the sessions earlier processes made, by folder; `config`
    /// is the inline configuration Ren gave the process, if any.
    pub fn new(own: Run, known: Vec<(String, String)>, config: Option<&str>) -> Self {
        let asks = config
            .and_then(|config| serde_json::from_str::<Value>(config).ok())
            .and_then(|config| config.get("permissions").cloned())
            .and_then(|rules| rules.as_array().cloned())
            .into_iter()
            .flatten()
            .filter(|rule| rule["effect"] == "ask")
            .filter_map(|rule| rule["action"].as_str().map(str::to_owned))
            .collect();
        Self {
            own,
            known,
            asks,
            session: None,
            made: 0,
            turns: 0,
            next_request: 0,
            turn: None,
        }
    }

    fn result(id: &Value, result: Value) -> Out {
        Out::Send(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    fn error(id: &Value, message: &str) -> Out {
        Out::Send(json!({ "jsonrpc": "2.0", "id": id,
            "error": { "code": -32602, "message": message } }))
    }

    fn update(&self, update: Value) -> Out {
        Out::Send(json!({ "jsonrpc": "2.0", "method": "session/update",
            "params": { "sessionId": self.session, "update": update } }))
    }

    fn options() -> Value {
        json!([
            { "id": "model", "name": "Model", "category": "model", "type": "select",
              "currentValue": Self::MODELS[0].0,
              "options": Self::MODELS.map(|(value, name)| json!({ "value": value, "name": name })) },
            { "id": "effort", "name": "Effort", "category": "thought_level", "type": "select",
              "currentValue": "default",
              "options": [{ "value": "low" }, { "value": "high" }, { "value": "default" }] },
            { "id": "mode", "name": "Session Mode", "category": "mode", "type": "select",
              "currentValue": "build", "options": [{ "value": "build", "name": "Build" }] },
        ])
    }

    fn commands(&self) -> Out {
        self.update(json!({ "sessionUpdate": "available_commands_update", "availableCommands": [
            { "name": "review", "description": "review changes [commit|branch|pr], defaults to uncommitted" },
            { "name": "compact", "description": "Compact the session" },
        ]}))
    }

    /// Read one message from Ren.
    pub fn receive(&mut self, message: &Value) -> Vec<Out> {
        let id = &message["id"];
        let params = &message["params"];
        let Some(method) = message["method"].as_str() else {
            // An answer to a permission this fake asked for.
            return self.answered(id, &message["result"]);
        };
        match method {
            "initialize" => vec![Self::result(
                id,
                json!({ "protocolVersion": 1,
                    "agentCapabilities": { "loadSession": true,
                        "sessionCapabilities": { "list": {}, "resume": {}, "delete": {} } },
                    "agentInfo": { "name": "OpenCode", "version": "dev" } }),
            )],
            "session/new" => {
                self.made += 1;
                // opencode's own shape: `ses_`, then 26 letters and digits.
                let session = format!(
                    "ses_{:012x}{:08x}DevOpn",
                    u64::from(self.own.0) * 0x9E37 + 0x7A1F,
                    self.made
                );
                let cwd = params["cwd"].as_str().unwrap_or_default().to_owned();
                self.session = Some(session.clone());
                self.known.push((cwd.clone(), session.clone()));
                vec![
                    Self::result(
                        id,
                        json!({ "sessionId": session, "configOptions": Self::options() }),
                    ),
                    Out::Remember { cwd, session },
                    self.commands(),
                ]
            }
            "session/list" => {
                let cwd = params["cwd"].as_str().unwrap_or_default();
                let sessions: Vec<Value> = self
                    .known
                    .iter()
                    .rev()
                    .filter(|(folder, _)| folder == cwd)
                    .map(|(folder, session)| json!({ "sessionId": session, "cwd": folder }))
                    .collect();
                vec![Self::result(id, json!({ "sessions": sessions }))]
            }
            "session/resume" => {
                let wanted = params["sessionId"].as_str().unwrap_or_default();
                if !self.known.iter().any(|(_, session)| session == wanted) {
                    return vec![Self::error(id, &format!("session not found: {wanted}"))];
                }
                self.session = Some(wanted.to_owned());
                vec![
                    Self::result(id, json!({ "configOptions": Self::options() })),
                    self.commands(),
                ]
            }
            "session/set_config_option" | "session/delete" => {
                vec![Self::result(
                    id,
                    json!({ "configOptions": Self::options() }),
                )]
            }
            "session/prompt" => {
                let prompt = params["prompt"][0]["text"].as_str().unwrap_or_default();
                self.prompted(id, prompt)
            }
            "_session/steering" => {
                let outcome = if self.turn.is_some() {
                    json!({ "outcome": "injected" })
                } else {
                    json!({ "outcome": "promptRequired", "reason": "noRunningTurn" })
                };
                vec![Self::result(id, outcome)]
            }
            "session/cancel" => self.end("cancelled"),
            _ => vec![Out::Send(json!({ "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": "Method not found" } }))],
        }
    }

    fn prompted(&mut self, id: &Value, prompt: &str) -> Vec<Out> {
        if self.turn.is_some() {
            return vec![Self::error(id, "the session is busy")];
        }
        self.turns += 1;
        let index = self.turns;
        self.turn = Some(Active {
            request: id.clone(),
            prompt: prompt.to_owned(),
            index,
            waiting: None,
        });
        let mut out = vec![self.update(json!({ "sessionUpdate": "agent_thought_chunk",
            "messageId": format!("msg_dev{index:04}:reasoning:0"),
            "content": { "type": "text", "text": "Looking at what was asked." } }))];
        match Ask::of(prompt) {
            Some(ask) => {
                out.push(self.tool_update(index, ask, "pending"));
                if self.asks.iter().any(|action| action == ask.action()) {
                    let request = self.next_request;
                    self.next_request += 1;
                    if let Some(turn) = &mut self.turn {
                        turn.waiting = Some((request, ask));
                    }
                    out.push(Out::Send(json!({ "jsonrpc": "2.0", "id": request,
                        "method": "session/request_permission", "params": {
                            "sessionId": self.session,
                            "toolCall": Self::tool_body(index, ask),
                            "options": [
                                { "optionId": "once", "kind": "allow_once", "name": "Allow once" },
                                { "optionId": "always", "kind": "allow_always", "name": "Always allow" },
                                { "optionId": "reject", "kind": "reject_once", "name": "Reject" },
                            ] } })));
                    return out;
                }
                out.extend(self.ran(index, ask, true));
            }
            None => out.extend(self.call()),
        }
        out
    }

    /// An answer to the permission the running turn waits on.
    fn answered(&mut self, id: &Value, result: &Value) -> Vec<Out> {
        let Some(turn) = &mut self.turn else {
            return Vec::new();
        };
        let Some((request, ask)) = turn.waiting else {
            return Vec::new();
        };
        if id.as_u64() != Some(request) {
            return Vec::new();
        }
        turn.waiting = None;
        let index = turn.index;
        let allowed = matches!(
            result["outcome"]["optionId"].as_str(),
            Some("once" | "always")
        );
        self.ran(index, ask, allowed)
    }

    fn tool_body(index: u32, ask: Ask) -> Value {
        match ask {
            Ask::Shell => json!({ "toolCallId": format!("call_dev{index:04}"), "name": "shell",
                "title": "cargo test --workspace", "kind": "execute",
                "rawInput": { "command": "cargo test --workspace" } }),
            Ask::Write => json!({ "toolCallId": format!("call_dev{index:04}"), "name": "write",
                "title": "src/lib.rs", "kind": "edit",
                "locations": [{ "path": "src/lib.rs" }],
                "rawInput": { "path": "src/lib.rs", "content": "pub fn ok() {}\n" } }),
        }
    }

    fn tool_update(&self, index: u32, ask: Ask, status: &str) -> Out {
        let mut body = Self::tool_body(index, ask);
        if let Value::Object(fields) = &mut body {
            fields.insert("sessionUpdate".into(), "tool_call".into());
            fields.insert("status".into(), status.into());
        }
        self.update(body)
    }

    /// The tool runs (or was refused), then the model is called.
    fn ran(&mut self, index: u32, ask: Ask, allowed: bool) -> Vec<Out> {
        let id = format!("call_dev{index:04}");
        let finished = match (allowed, ask) {
            (false, Ask::Shell | Ask::Write) => json!({ "sessionUpdate": "tool_call_update",
                "toolCallId": id, "status": "failed",
                "content": [{ "type": "content", "content": { "type": "text",
                    "text": "The user rejected permission to use this tool." } }] }),
            (true, Ask::Shell) => json!({ "sessionUpdate": "tool_call_update",
                "toolCallId": id, "status": "completed",
                "content": [{ "type": "content", "content": { "type": "text",
                    "text": "running 12 tests\ntest result: ok. 12 passed; 0 failed\n" } }],
                "rawOutput": { "metadata": { "status": "completed", "exit": 0 } } }),
            (true, Ask::Write) => json!({ "sessionUpdate": "tool_call_update",
                "toolCallId": id, "status": "completed",
                "content": [{ "type": "content", "content": { "type": "text",
                    "text": "Created file successfully: src/lib.rs" } }] }),
        };
        let mut out = vec![self.update(finished)];
        out.extend(self.call());
        out
    }

    fn call(&self) -> Vec<Out> {
        match (&self.turn, &self.session) {
            (Some(turn), Some(session)) => {
                vec![Out::Call(Scenario::opencode_turn_call(
                    session,
                    &turn.prompt,
                ))]
            }
            (None | Some(_), _) => Vec::new(),
        }
    }

    /// A piece of the reply, as it streams.
    pub fn delta(&self, text: &str) -> Value {
        let index = self.turn.as_ref().map_or(0, |turn| turn.index);
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": self.session,
            "update": { "sessionUpdate": "agent_message_chunk",
                "messageId": format!("msg_dev{index:04}"),
                "content": { "type": "text", "text": text } } } })
    }

    /// The model call finished: the turn ends (failed, if the call did).
    pub fn replied(&mut self, reply: Result<String, String>) -> Vec<Out> {
        if self.turn.is_none() {
            // Cancelled while the call was out.
            return Vec::new();
        }
        match reply {
            Ok(_) => {
                let mut out = vec![self.update(json!({ "sessionUpdate": "usage_update",
                    "used": 1200, "size": 200_000, "cost": { "amount": 0, "currency": "USD" } }))];
                out.extend(self.end("end_turn"));
                out
            }
            Err(error) => match self.turn.take() {
                Some(turn) => vec![Out::Send(json!({ "jsonrpc": "2.0", "id": turn.request,
                    "error": { "code": -32603, "message": error } }))],
                None => Vec::new(),
            },
        }
    }

    /// Answer the running turn's prompt with `reason`.
    fn end(&mut self, reason: &str) -> Vec<Out> {
        let Some(turn) = self.turn.take() else {
            return Vec::new();
        };
        vec![Self::result(
            &turn.request,
            json!({ "stopReason": reason,
                "usage": { "inputTokens": 1000, "outputTokens": 200, "totalTokens": 1200 } }),
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenCodeScript, Out};
    use crate::dev::seed::Run;
    use serde_json::{Value, json};

    fn sent(out: &[Out]) -> Vec<Value> {
        out.iter()
            .filter_map(|out| match out {
                Out::Send(value) => Some(value.clone()),
                Out::Call(_) | Out::Remember { .. } => None,
            })
            .collect()
    }

    /// Ren's own codec drives this fake, each line answered in turn, until
    /// nothing more is sent.
    fn driven(
        agent: &mut OpenCodeScript,
        opening: &ren_harness::drive::Opening,
    ) -> ren_harness::drive::Driver {
        use ren_harness::{Harness, drive::Driver, lines::Framed};
        let (mut driver, mut lines) = Driver::open(
            Harness::OpenCode,
            opening.clone(),
            "0.1.0",
            uuid::Uuid::from_u128(5),
        )
        .unwrap();
        for _ in 0..8 {
            let mut next = Vec::new();
            for line in &lines {
                let message: Value = serde_json::from_slice(line.as_bytes()).unwrap();
                for reply in sent(&agent.receive(&message)) {
                    let received = driver.receive(Framed::Line(reply.to_string().into_bytes()));
                    next.extend(received.replies);
                }
            }
            lines = next;
        }
        driver
    }

    fn opening(
        thread: Option<ren_protocol::native::NativeThreadId>,
    ) -> ren_harness::drive::Opening {
        let settings = ren_harness::settings::RunSettings::default();
        match thread {
            None => ren_harness::drive::Opening::New {
                cwd: "/work/acme".into(),
                id: ren_protocol::native::NativeThreadId::new(uuid::Uuid::from_u128(1)).unwrap(),
                settings,
            },
            Some(thread) => ren_harness::drive::Opening::Resume {
                thread,
                cwd: "/work/acme".into(),
                settings,
            },
        }
    }

    #[test]
    fn rens_codec_opens_a_session_and_resumes_it_in_a_later_process() {
        let mut first = OpenCodeScript::new(Run(7), Vec::new(), None);
        let mut driver = driven(&mut first, &opening(None));
        assert!(!driver.commands().is_empty());
        let known = first.known.clone();
        assert_eq!(known.len(), 1);
        let ren_harness::drive::Driver::OpenCode(codec) = &mut driver else {
            panic!("an opencode driver");
        };
        let thread = codec.thread().unwrap();
        // A later process knows the folder's session from the shell's list.
        let mut later = OpenCodeScript::new(Run(8), known, None);
        let ren_harness::drive::Driver::OpenCode(resumed) =
            driven(&mut later, &opening(Some(thread)))
        else {
            panic!("an opencode driver");
        };
        assert_eq!(resumed.thread(), Some(thread));
    }

    #[test]
    fn a_prompt_to_run_tests_asks_first_when_shell_asks() {
        let config =
            json!({ "permissions": [{ "action": "shell", "resource": "*", "effect": "ask" }] });
        let mut agent = OpenCodeScript::new(Run(7), Vec::new(), Some(&config.to_string()));
        let _ = agent.receive(&json!({ "jsonrpc": "2.0", "id": 0, "method": "session/new",
            "params": { "cwd": "/work/acme", "mcpServers": [] } }));
        let out = agent.receive(&json!({ "jsonrpc": "2.0", "id": 1, "method": "session/prompt",
            "params": { "sessionId": agent.session, "prompt": [{ "type": "text", "text": "run the tests" }] } }));
        let asked = sent(&out)
            .into_iter()
            .find(|line| line["method"] == "session/request_permission");
        let asked = asked.expect("asks first");
        assert!(
            !matches!(out.last(), Some(Out::Call(_))),
            "waits for the answer"
        );
        let ran = agent.receive(&json!({ "jsonrpc": "2.0", "id": asked["id"],
            "result": { "outcome": { "outcome": "selected", "optionId": "once" } } }));
        assert!(matches!(ran.last(), Some(Out::Call(_))));
        let done = agent.replied(Ok("Done.".into()));
        assert_eq!(
            sent(&done).last().unwrap()["result"]["stopReason"],
            "end_turn"
        );
    }

    #[test]
    fn a_cancel_answers_the_prompt_cancelled() {
        let mut agent = OpenCodeScript::new(Run(7), Vec::new(), None);
        let _ = agent.receive(&json!({ "jsonrpc": "2.0", "id": 0, "method": "session/new",
            "params": { "cwd": "/work/acme" } }));
        let _ = agent.receive(
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "session/prompt",
            "params": { "prompt": [{ "type": "text", "text": "explain" }] } }),
        );
        let out =
            agent.receive(&json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": {} }));
        assert_eq!(sent(&out)[0]["result"]["stopReason"], "cancelled");
        assert!(agent.replied(Ok("late".into())).is_empty());
    }
}
