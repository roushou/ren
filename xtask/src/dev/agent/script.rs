//! What the dev slot's fake Codex says, as a pure state machine: app-server
//! messages in, messages out, and the model calls to make. It opens a
//! thread, and for each prompt asks one approval when the prompt calls for
//! it (a command for "test", "run", or "build"; a file change for "fix",
//! "edit", "change", "add", or "write"), then makes one model call through
//! the gateway and streams its reply back as the turn's agent message.

use crate::dev::seed::Run;
use serde_json::{Value, json};

/// What the shell does next.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// Write this message to stdout.
    Send(Value),
    /// Make the turn's model call; its reply goes to [`AgentScript::delta`]
    /// and [`AgentScript::replied`].
    Call {
        session: Run,
        numbers: (u32, u32),
        prompt: String,
    },
}

/// What a prompt asks the operator to allow before the reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ask {
    Command,
    FileChange,
}

impl Ask {
    fn of(prompt: &str) -> Option<Self> {
        let prompt = prompt.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|word| prompt.contains(word));
        if has(&["test", "run", "build"]) {
            Some(Self::Command)
        } else if has(&["fix", "edit", "change", "add", "write"]) {
            Some(Self::FileChange)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Turn {
    /// Waiting for the operator's answer to this request id.
    Asking { ask: Ask, request: u64 },
    /// The model call is out.
    Calling,
}

#[derive(Debug, Clone, PartialEq)]
struct Active {
    id: String,
    index: u32,
    prompt: String,
    step: Turn,
}

/// One fake app server's conversation.
pub struct AgentScript {
    /// This process's own number, for the turns it makes.
    own: Run,
    /// The conversation's run and number: its thread is `session.uuid(n)`.
    thread: Option<(Run, u32)>,
    cwd: String,
    turns: u32,
    requests: u64,
    turn: Option<Active>,
}

impl AgentScript {
    /// The number of the thread a new conversation opens.
    const THREAD: u32 = 1;

    pub fn new(own: Run) -> Self {
        Self {
            own,
            thread: None,
            cwd: String::new(),
            turns: 0,
            requests: 1000,
            turn: None,
        }
    }

    fn thread_id(&self) -> String {
        self.thread.map_or_else(String::new, |(run, n)| run.uuid(n))
    }

    fn item(&self, phase: &str, item: Value) -> Value {
        let turn = self.turn.as_ref().map_or("", |turn| turn.id.as_str());
        json!({ "method": format!("item/{phase}"),
                "params": { "threadId": self.thread_id(), "turnId": turn, "item": item } })
    }

    /// The efforts each of its models takes.
    const EFFORTS: [&'static str; 4] = ["low", "medium", "high", "xhigh"];

    /// The models it lists: invented, one the default.
    const MODELS: [(&'static str, &'static str, &'static str, bool); 4] = [
        (
            "dev-swift",
            "Dev Swift",
            "Fast and light, for everyday edits.",
            true,
        ),
        (
            "dev-sage",
            "Dev Sage",
            "Slower and more thorough, for hard problems.",
            false,
        ),
        (
            "dev-scout",
            "Dev Scout",
            "Reads widely before it changes anything.",
            false,
        ),
        (
            "dev-spark",
            "Dev Spark",
            "The smallest, for quick answers.",
            false,
        ),
    ];

    /// Read one message from Ren.
    pub fn receive(&mut self, message: &Value) -> Vec<Out> {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match (method, id) {
            (Some("initialize"), Some(id)) => vec![Out::Send(json!({ "id": id, "result": {
                "userAgent": "ren-dev-agent", "codexHome": "/nonexistent", "platformFamily": "unix", "platformOs": "linux" } }))],
            (Some("thread/start"), Some(id)) => {
                self.cwd = params["cwd"].as_str().unwrap_or_default().to_owned();
                self.thread = Some((self.own, Self::THREAD));
                self.opened(id)
            }
            (Some("thread/resume"), Some(id)) => {
                self.cwd = params["cwd"].as_str().unwrap_or_default().to_owned();
                // A thread this fake made keeps its session; any other gets a new one.
                let resumed = params["threadId"].as_str().and_then(Run::parse_uuid);
                self.thread = Some(resumed.unwrap_or((self.own, Self::THREAD)));
                self.opened(id)
            }
            (Some("model/list"), Some(id)) => vec![Out::Send(json!({ "id": id, "result": {
                "data": Self::MODELS.map(|(model, name, description, default)| json!({
                    "id": model, "model": model, "displayName": name, "description": description,
                    "isDefault": default, "hidden": false, "defaultReasoningEffort": "medium",
                    "supportedReasoningEfforts": Self::EFFORTS.map(|effort| json!({
                        "reasoningEffort": effort, "description": "" })) })),
                "nextCursor": null } }))],
            (Some("turn/start"), Some(id)) => self.start(id, &params),
            (Some("turn/steer"), Some(id)) => self.steer(id, &params),
            (Some("skills/list"), Some(id)) => vec![Out::Send(json!({ "id": id, "result": {
                "data": [{ "cwd": self.cwd, "errors": [], "skills": [
                    { "name": "release-notes", "description": "Draft release notes from the log",
                      "enabled": true, "scope": "repo",
                      "path": format!("{}/.codex/skills/release-notes/SKILL.md", self.cwd) },
                    { "name": "acme-style", "description": "Follow the acme style guide",
                      "enabled": true, "scope": "user",
                      "path": "/home/dev/.codex/skills/acme-style/SKILL.md" },
                ] }] } }))],
            (Some("turn/interrupt"), Some(id)) => {
                let mut out = vec![Out::Send(json!({ "id": id, "result": {} }))];
                out.extend(self.end("interrupted"));
                out
            }
            (Some(_), Some(id)) => vec![Out::Send(json!({ "id": id,
                "error": { "code": -32601, "message": "the dev agent does not do that" } }))],
            (None, Some(id)) => self.answered(&id, message.get("result")),
            (Some(_) | None, None) => Vec::new(),
        }
    }

    fn opened(&self, id: Value) -> Vec<Out> {
        let thread = json!({ "id": self.thread_id(), "cwd": self.cwd });
        vec![
            Out::Send(
                json!({ "id": id, "result": { "thread": thread, "model": "gpt-5.5-codex" } }),
            ),
            Out::Send(json!({ "method": "thread/started", "params": { "thread": thread } })),
        ]
    }

    fn start(&mut self, id: Value, params: &Value) -> Vec<Out> {
        if self.turn.is_some() || self.thread.is_none() {
            return vec![Out::Send(json!({ "id": id,
                "error": { "code": -32600, "message": "a turn is running" } }))];
        }
        let prompt: String = params["input"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        self.turns += 1;
        let index = self.turns;
        let turn_id = self.turn_number(index);
        let id_text = self
            .thread
            .map_or_else(String::new, |(run, _)| run.uuid(turn_id));
        self.turn = Some(Active {
            id: id_text.clone(),
            index,
            prompt: prompt.clone(),
            step: Turn::Calling,
        });
        let turn = json!({ "id": id_text, "status": "inProgress", "items": [] });
        let mut out = vec![
            Out::Send(json!({ "id": id, "result": { "turn": turn } })),
            Out::Send(json!({ "method": "turn/started",
                              "params": { "threadId": self.thread_id(), "turn": turn } })),
            Out::Send(self.item(
                "completed",
                json!({ "type": "userMessage", "id": format!("user-{index}"),
                "content": [{ "type": "text", "text": prompt }] }),
            )),
        ];
        out.extend(match Ask::of(&prompt) {
            Some(ask) => self.ask(ask),
            None => self.call(),
        });
        out
    }

    /// A message into the running turn: taken as a user message of it.
    fn steer(&mut self, id: Value, params: &Value) -> Vec<Out> {
        let text: String = params["input"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        let Some(turn) = self.turn.as_ref() else {
            return vec![Out::Send(json!({ "id": id,
                "error": { "code": -32600, "message": "no turn is running" } }))];
        };
        let (turn_id, index) = (turn.id.clone(), turn.index);
        self.requests += 1;
        vec![
            Out::Send(json!({ "id": id, "result": { "turnId": turn_id } })),
            Out::Send(self.item(
                "completed",
                json!({ "type": "userMessage", "id": format!("steer-{index}-{}", self.requests),
                "content": [{ "type": "text", "text": text }] }),
            )),
        ]
    }

    /// The number a turn's id is made from: unique per process, so a
    /// resumed thread's turns never repeat an earlier process's.
    fn turn_number(&self, index: u32) -> u32 {
        0x100 + (self.own.0 % 0x100) * 0x40 + (index % 0x40)
    }

    fn ask(&mut self, ask: Ask) -> Option<Out> {
        self.requests += 1;
        let request = self.requests;
        let thread = self.thread_id();
        let turn = self.turn.as_mut()?;
        turn.step = Turn::Asking { ask, request };
        let item = format!("{}-{}", Self::ask_item(ask), turn.index);
        let params = match ask {
            Ask::Command => json!({ "threadId": thread, "turnId": turn.id, "itemId": item,
                "command": "cargo test --workspace", "cwd": self.cwd,
                "reason": "Run the tests to see what fails" }),
            Ask::FileChange => json!({ "threadId": thread, "turnId": turn.id, "itemId": item,
                "reason": "Edit src/lib.rs" }),
        };
        let method = match ask {
            Ask::Command => "item/commandExecution/requestApproval",
            Ask::FileChange => "item/fileChange/requestApproval",
        };
        Some(Out::Send(
            json!({ "id": request, "method": method, "params": params }),
        ))
    }

    fn ask_item(ask: Ask) -> &'static str {
        match ask {
            Ask::Command => "cmd",
            Ask::FileChange => "patch",
        }
    }

    fn call(&mut self) -> Option<Out> {
        let (session, thread) = self.thread?;
        let number = self.turn_number(self.turn.as_ref()?.index);
        let turn = self.turn.as_mut()?;
        turn.step = Turn::Calling;
        Some(Out::Call {
            session,
            numbers: (thread, number),
            prompt: turn.prompt.clone(),
        })
    }

    /// Ren's answer to an approval request.
    fn answered(&mut self, id: &Value, result: Option<&Value>) -> Vec<Out> {
        let Some(Turn::Asking { ask, request }) = self.turn.as_ref().map(|turn| turn.step.clone())
        else {
            return Vec::new();
        };
        if id.as_u64() != Some(request) {
            return Vec::new();
        }
        let decision = result
            .and_then(|result| result["decision"].as_str())
            .unwrap_or("decline");
        let index = self.turn.as_ref().map_or(0, |turn| turn.index);
        let item_id = format!("{}-{index}", Self::ask_item(ask));
        let resolved = Out::Send(json!({ "method": "serverRequest/resolved",
            "params": { "threadId": self.thread_id(), "requestId": request } }));
        let allowed = matches!(decision, "accept" | "acceptForSession");
        let status = if allowed { "completed" } else { "declined" };
        let item = match ask {
            Ask::Command => json!({ "type": "commandExecution", "id": item_id,
                "command": "cargo test --workspace", "cwd": self.cwd, "status": status,
                "commandActions": [],
                "aggregatedOutput": if allowed { "running 12 tests\ntest result: ok. 12 passed; 0 failed" } else { "" },
                "exitCode": if allowed { json!(0) } else { Value::Null } }),
            Ask::FileChange => json!({ "type": "fileChange", "id": item_id, "status": status,
                "changes": [{ "path": format!("{}/src/lib.rs", self.cwd), "kind": { "type": "update" },
                              "diff": "-    todo!()\n+    Ok(())\n" }] }),
        };
        let mut out = vec![resolved];
        if allowed {
            out.push(Out::Send(self.item("started", item.clone())));
        }
        out.push(Out::Send(self.item("completed", item)));
        if decision == "cancel" {
            out.extend(self.end("interrupted"));
        } else {
            out.extend(self.call());
        }
        out
    }

    /// A piece of the reply, as it streams.
    pub fn delta(&self, text: &str) -> Value {
        let index = self.turn.as_ref().map_or(0, |turn| turn.index);
        json!({ "method": "item/agentMessage/delta", "params": { "threadId": self.thread_id(),
            "turnId": self.turn.as_ref().map_or("", |turn| turn.id.as_str()),
            "itemId": format!("msg-{index}"), "delta": text } })
    }

    /// The model call finished with `reply` (or failed with `error`).
    pub fn replied(&mut self, reply: Result<String, String>) -> Vec<Out> {
        if !matches!(
            self.turn.as_ref().map(|turn| &turn.step),
            Some(Turn::Calling)
        ) {
            return Vec::new();
        }
        let index = self.turn.as_ref().map_or(0, |turn| turn.index);
        match reply {
            Ok(text) => {
                let mut out = vec![Out::Send(self.item(
                    "completed",
                    json!({ "type": "agentMessage", "id": format!("msg-{index}"), "text": text }),
                ))];
                out.extend(self.end("completed"));
                out
            }
            Err(message) => {
                let mut out = vec![Out::Send(json!({ "method": "error", "params": {
                    "threadId": self.thread_id(),
                    "turnId": self.turn.as_ref().map_or("", |turn| turn.id.as_str()),
                    "error": { "message": message }, "willRetry": false } }))];
                out.extend(self.end("failed"));
                out
            }
        }
    }

    fn end(&mut self, status: &str) -> Vec<Out> {
        let Some(turn) = self.turn.take() else {
            return Vec::new();
        };
        vec![Out::Send(json!({ "method": "turn/completed", "params": {
            "threadId": self.thread_id(),
            "turn": { "id": turn.id, "status": status, "items": [] } } }))]
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentScript, Out};
    use crate::dev::seed::Run;
    use serde_json::{Value, json};

    fn sent(out: &[Out]) -> Vec<Value> {
        out.iter()
            .filter_map(|out| match out {
                Out::Send(value) => Some(value.clone()),
                Out::Call { .. } => None,
            })
            .collect()
    }

    fn methods(out: &[Out]) -> Vec<String> {
        sent(out)
            .iter()
            .map(|value| value["method"].as_str().unwrap_or("(response)").to_owned())
            .collect()
    }

    fn opened() -> AgentScript {
        let mut agent = AgentScript::new(Run(7));
        agent.receive(&json!({ "id": 0, "method": "initialize", "params": {} }));
        let out = agent.receive(
            &json!({ "id": 1, "method": "thread/start", "params": { "cwd": "/work/acme" } }),
        );
        assert_eq!(sent(&out)[0]["result"]["thread"]["id"], Run(7).uuid(1));
        agent
    }

    #[test]
    fn a_steer_joins_the_running_turn_and_is_refused_without_one() {
        let mut agent = opened();
        let refused = agent.receive(&json!({ "id": 2, "method": "turn/steer",
            "params": { "input": [{ "type": "text", "text": "too early" }] } }));
        assert!(sent(&refused)[0]["error"].is_object());
        let out = agent.receive(&json!({ "id": 3, "method": "turn/start",
            "params": { "input": [{ "type": "text", "text": "explain the parser" }] } }));
        let turn = sent(&out)[0]["result"]["turn"]["id"].clone();
        let steered = agent.receive(&json!({ "id": 4, "method": "turn/steer",
            "params": { "input": [{ "type": "text", "text": "only the lexer" }] } }));
        assert_eq!(methods(&steered), ["(response)", "item/completed"]);
        assert_eq!(sent(&steered)[0]["result"]["turnId"], turn);
        assert_eq!(
            sent(&steered)[1]["params"]["item"]["content"][0]["text"],
            "only the lexer"
        );
    }

    #[test]
    fn a_plain_prompt_calls_the_model_and_streams_the_reply() {
        let mut agent = opened();
        let out = agent.receive(&json!({ "id": 2, "method": "turn/start",
            "params": { "input": [{ "type": "text", "text": "explain the parser" }] } }));
        assert_eq!(
            methods(&out),
            ["(response)", "turn/started", "item/completed"]
        );
        let Some(Out::Call {
            session,
            numbers,
            prompt,
        }) = out.last()
        else {
            panic!("a model call: {out:?}");
        };
        assert_eq!(
            (session.0, numbers.0, prompt.as_str()),
            (7, 1, "explain the parser")
        );
        // The call's turn number makes the turn id the turn reports.
        let turn = sent(&out)[0]["result"]["turn"]["id"].clone();
        assert_eq!(turn, Run(7).uuid(numbers.1));
        assert_eq!(agent.delta("Hel")["params"]["delta"], "Hel");
        let done = agent.replied(Ok("Hello.".into()));
        assert_eq!(methods(&done), ["item/completed", "turn/completed"]);
        assert_eq!(sent(&done)[1]["params"]["turn"]["status"], "completed");
        // The next prompt is a new turn.
        let again = agent.receive(&json!({ "id": 3, "method": "turn/start",
            "params": { "input": [{ "type": "text", "text": "thanks" }] } }));
        assert_ne!(sent(&again)[0]["result"]["turn"]["id"], turn);
    }

    #[test]
    fn a_prompt_to_run_tests_asks_first_and_runs_the_command_once_allowed() {
        let mut agent = opened();
        let out = agent.receive(&json!({ "id": 2, "method": "turn/start",
            "params": { "input": [{ "type": "text", "text": "run the tests" }] } }));
        let ask = sent(&out).last().cloned().unwrap();
        assert_eq!(ask["method"], "item/commandExecution/requestApproval");
        assert_eq!(ask["params"]["command"], "cargo test --workspace");
        assert!(!out.iter().any(|out| matches!(out, Out::Call { .. })));
        // An answer to some other request changes nothing.
        assert!(
            agent
                .receive(&json!({ "id": 5, "result": { "decision": "accept" } }))
                .is_empty()
        );
        let out = agent.receive(&json!({ "id": ask["id"], "result": { "decision": "accept" } }));
        assert_eq!(
            methods(&out),
            ["serverRequest/resolved", "item/started", "item/completed"]
        );
        assert_eq!(sent(&out)[2]["params"]["item"]["exitCode"], 0);
        assert!(matches!(out.last(), Some(Out::Call { .. })));
    }

    #[test]
    fn declining_skips_the_change_and_cancelling_ends_the_turn() {
        let mut agent = opened();
        let out = agent.receive(&json!({ "id": 2, "method": "turn/start",
            "params": { "input": [{ "type": "text", "text": "fix the bug" }] } }));
        let ask = sent(&out).last().cloned().unwrap();
        assert_eq!(ask["method"], "item/fileChange/requestApproval");
        let declined =
            agent.receive(&json!({ "id": ask["id"], "result": { "decision": "decline" } }));
        assert_eq!(sent(&declined)[1]["params"]["item"]["status"], "declined");
        assert!(matches!(declined.last(), Some(Out::Call { .. })));
        agent.replied(Ok("Left it.".into()));

        let out = agent.receive(&json!({ "id": 3, "method": "turn/start",
            "params": { "input": [{ "type": "text", "text": "edit it anyway" }] } }));
        let ask = sent(&out).last().cloned().unwrap();
        let cancelled =
            agent.receive(&json!({ "id": ask["id"], "result": { "decision": "cancel" } }));
        assert_eq!(
            sent(&cancelled).last().unwrap()["params"]["turn"]["status"],
            "interrupted"
        );
        assert!(agent.replied(Ok("late".into())).is_empty());
    }

    #[test]
    fn resuming_its_own_thread_keeps_the_session() {
        let mut agent = AgentScript::new(Run(9));
        let thread = Run(4).uuid(1);
        let out = agent.receive(&json!({ "id": 1, "method": "thread/resume",
            "params": { "threadId": thread, "cwd": "/work/acme" } }));
        assert_eq!(sent(&out)[0]["result"]["thread"]["id"], thread);
        let out = agent.receive(&json!({ "id": 2, "method": "turn/start",
            "params": { "input": [{ "type": "text", "text": "go on" }] } }));
        let Some(Out::Call {
            session, numbers, ..
        }) = out.last()
        else {
            panic!("a model call");
        };
        assert_eq!((session.0, numbers.0), (4, 1));
        // Turn ids come from this process's number, so they differ from run 4's own.
        assert_ne!(numbers.1, AgentScript::new(Run(4)).turn_number(1));
    }

    /// Ren's own listing codec reads every line this fake answers with.
    fn listed(
        harness: ren_harness::Harness,
        mut answer: impl FnMut(&Value) -> Vec<Value>,
    ) -> Vec<ren_harness::models::Model> {
        use ren_harness::{
            lines::Framed,
            models::{Listing, ModelListing},
        };
        let (mut listing, mut lines) = ModelListing::open(harness, "0.1.0", "/work").unwrap();
        for _ in 0..8 {
            let mut next = Vec::new();
            for line in &lines {
                let message: Value = serde_json::from_slice(line.as_bytes()).unwrap();
                for reply in answer(&message) {
                    match listing.receive(Framed::Line(reply.to_string().into_bytes())) {
                        Listing::Waiting(more) => next.extend(more),
                        Listing::Listed(models) => return models,
                        Listing::Failed(why) => panic!("{why}"),
                    }
                }
            }
            lines = next;
        }
        panic!("never listed");
    }

    #[test]
    fn its_models_read_as_codex_lists_them() {
        let mut agent = AgentScript::new(Run(7));
        let models = listed(ren_harness::Harness::Codex, |message| {
            sent(&agent.receive(message))
        });
        assert_eq!(models.len(), 4);
        assert_eq!(
            models[0].choice,
            ren_harness::models::DefaultChoice::Default
        );
    }
}
