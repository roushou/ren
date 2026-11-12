//! What the dev slot's fake Claude Code says in stream-json print mode, as a
//! pure state machine: Ren's lines in, lines out, and the model calls to
//! make. Like the fake Codex, a prompt about testing, running, or building
//! asks to run a command, and one about fixing, editing, changing, adding,
//! or writing asks to write a file; then one model call goes through the
//! gateway and its reply streams back. As the real client does, it echoes
//! each prompt under its own message id, which is not the prompt id its
//! model call carries.

use crate::dev::seed::{Call, Run, Scenario};
use serde_json::{Value, json};

/// What the shell does next.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// Write this message to stdout.
    Send(Value),
    /// Make the turn's model call; its reply goes to [`ClaudeScript::delta`]
    /// and [`ClaudeScript::replied`].
    Call(Call),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ask {
    Command,
    Write,
}

impl Ask {
    fn of(prompt: &str) -> Option<Self> {
        let prompt = prompt.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|word| prompt.contains(word));
        if has(&["test", "run", "build"]) {
            Some(Self::Command)
        } else if has(&["fix", "edit", "change", "add", "write"]) {
            Some(Self::Write)
        } else {
            None
        }
    }

    fn tool(self) -> &'static str {
        match self {
            Self::Command => "Bash",
            Self::Write => "Write",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Step {
    /// Waiting for Ren's answer to this permission prompt.
    Asking { ask: Ask, request: String },
    /// The model call is out.
    Calling,
}

#[derive(Debug, Clone, PartialEq)]
struct Active {
    index: u32,
    prompt: String,
    step: Step,
}

/// One fake Claude Code process's conversation.
pub struct ClaudeScript {
    /// This process's own number, for the ids it makes.
    own: Run,
    /// The conversation Ren named with `--session-id` or `--resume`.
    session: String,
    cwd: String,
    turns: u32,
    turn: Option<Active>,
}

impl ClaudeScript {
    /// `arguments` are those the daemon gave `claude`.
    pub fn new(own: Run, arguments: &[String], cwd: String) -> Option<Self> {
        let session = arguments
            .windows(2)
            .find(|pair| matches!(pair[0].as_str(), "--session-id" | "--resume"))
            .map(|pair| pair[1].clone())
            // A listing of models keeps no session.
            .or_else(|| {
                arguments
                    .iter()
                    .any(|argument| argument == "--no-session-persistence")
                    .then(String::new)
            })?;
        Some(Self {
            own,
            session,
            cwd,
            turns: 0,
            turn: None,
        })
    }

    fn message_id(&self) -> String {
        let index = self.turn.as_ref().map_or(0, |turn| turn.index);
        format!("msg_dev{:04}{index:04}", self.own.0 % 10_000)
    }

    fn line(&self, kind: &str, mut fields: Value) -> Value {
        fields["type"] = kind.into();
        fields["session_id"] = self.session.clone().into();
        fields["parent_tool_use_id"] = Value::Null;
        fields
    }

    /// Read one line from Ren.
    /// The models its `initialize` answer lists: invented, with the entry
    /// that stands for no choice first, as the real client lists it.
    fn models() -> Value {
        json!([
            { "value": "default", "resolvedModel": "dev-large-1",
              "displayName": "Default (recommended)", "description": "Dev Large · for most work" },
            { "value": "large", "resolvedModel": "dev-large-1", "displayName": "Dev Large",
              "description": "For most work", "supportsEffort": true,
              "supportedEffortLevels": ["low", "medium", "high", "xhigh", "max"] },
            { "value": "huge", "resolvedModel": "dev-huge-1", "displayName": "Dev Huge",
              "description": "For the hardest, longest tasks", "supportsEffort": true,
              "supportedEffortLevels": ["low", "medium", "high", "xhigh", "max"] },
            { "value": "small", "resolvedModel": "dev-small-1", "displayName": "Dev Small",
              "description": "Fastest, for quick answers" },
            { "value": "dev-large-0", "resolvedModel": "dev-large-0", "displayName": "Dev Large 0",
              "description": "The previous large model" },
        ])
    }

    pub fn receive(&mut self, message: &Value) -> Vec<Out> {
        match message["type"].as_str() {
            Some("control_request") => {
                let id = message["request_id"].clone();
                let answer = if message["request"]["subtype"] == "initialize" {
                    json!({ "models": Self::models() })
                } else {
                    json!({})
                };
                let mut out = vec![Out::Send(json!({ "type": "control_response", "response": {
                    "subtype": "success", "request_id": id, "response": answer } }))];
                if message["request"]["subtype"] == "interrupt" {
                    out.extend(self.end(Some("aborted_streaming")));
                }
                out
            }
            Some("user") => self.prompted(message),
            Some("control_response") => self.answered(&message["response"]),
            Some(_) | None => Vec::new(),
        }
    }

    fn prompted(&mut self, message: &Value) -> Vec<Out> {
        if self.turn.is_some() {
            // The real client queues it; this fake keeps one turn at a time.
            return Vec::new();
        }
        let prompt = message["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        self.turns += 1;
        let index = self.turns;
        self.turn = Some(Active {
            index,
            prompt: prompt.clone(),
            step: Step::Calling,
        });
        let mut echo = message.clone();
        echo["uuid"] = self.own.uuid(0x2000 + index).into();
        echo["isReplay"] = true.into();
        let mut out = vec![Out::Send(echo)];
        out.extend(match Ask::of(&prompt) {
            Some(ask) => self.ask(ask),
            None => self.call(),
        });
        out
    }

    fn tool_use(&self, ask: Ask) -> Value {
        let index = self.turn.as_ref().map_or(0, |turn| turn.index);
        let input = match ask {
            Ask::Command => json!({ "command": "cargo test --workspace",
                                    "description": "Run the tests to see what fails" }),
            Ask::Write => json!({ "file_path": format!("{}/src/lib.rs", self.cwd),
                                  "content": "pub fn parse() -> Result<(), Error> {\n    Ok(())\n}\n" }),
        };
        json!({ "type": "tool_use", "id": format!("toolu_dev{index:04}"), "name": ask.tool(),
                "input": input })
    }

    fn ask(&mut self, ask: Ask) -> Vec<Out> {
        let tool_use = self.tool_use(ask);
        let request = format!("perm-{}", tool_use["id"].as_str().unwrap_or_default());
        let Some(turn) = self.turn.as_mut() else {
            return Vec::new();
        };
        turn.step = Step::Asking {
            ask,
            request: request.clone(),
        };
        let assistant = self.line(
            "assistant",
            json!({ "message": { "id": self.message_id(), "role": "assistant",
                                 "content": [tool_use.clone()] } }),
        );
        let description = match ask {
            Ask::Command => "Run the tests to see what fails",
            Ask::Write => "src/lib.rs",
        };
        vec![
            Out::Send(assistant),
            Out::Send(
                json!({ "type": "control_request", "request_id": request, "request": {
                "subtype": "can_use_tool", "tool_name": ask.tool(), "display_name": ask.tool(),
                "input": tool_use["input"], "description": description,
                "tool_use_id": tool_use["id"],
                "permission_suggestions": [{ "type": "addRules", "behavior": "allow",
                    "rules": [{ "toolName": ask.tool() }], "destination": "localSettings" }] } }),
            ),
        ]
    }

    /// Ren's answer to the permission prompt.
    fn answered(&mut self, response: &Value) -> Vec<Out> {
        let Some(Step::Asking { ask, request }) = self.turn.as_ref().map(|turn| turn.step.clone())
        else {
            return Vec::new();
        };
        if response["request_id"].as_str() != Some(request.as_str()) {
            return Vec::new();
        }
        let answer = &response["response"];
        let allowed = answer["behavior"] == "allow";
        let tool_use = self.tool_use(ask);
        let (content, is_error, result) = match (allowed, ask) {
            (true, Ask::Command) => (
                "running 12 tests\ntest result: ok. 12 passed; 0 failed",
                false,
                json!({ "stdout": "running 12 tests\ntest result: ok. 12 passed; 0 failed",
                        "stderr": "", "interrupted": false }),
            ),
            (true, Ask::Write) => (
                "File written.",
                false,
                json!({ "type": "update", "filePath": tool_use["input"]["file_path"] }),
            ),
            (false, Ask::Command | Ask::Write) => (
                "The user doesn't want to proceed with this tool use.",
                true,
                json!("User rejected tool use"),
            ),
        };
        let mut meta = json!({ "id": tool_use["id"] });
        if !allowed {
            meta["non_execution_kind"] = "user-rejected".into();
        }
        let mut out = vec![Out::Send(self.line(
            "user",
            json!({ "message": { "role": "user", "content": [{ "type": "tool_result",
                        "tool_use_id": tool_use["id"], "content": content, "is_error": is_error }] },
                    "tool_use_result": result, "tool_result_meta": [meta] }),
        ))];
        if answer["interrupt"] == true {
            out.extend(self.end(Some("aborted_tools")));
        } else {
            out.extend(self.call());
        }
        out
    }

    fn call(&mut self) -> Vec<Out> {
        let message = self.message_id();
        let Some(turn) = self.turn.as_mut() else {
            return Vec::new();
        };
        turn.step = Step::Calling;
        // The prompt id the model call carries is Claude Code's own, apart
        // from the echo's message id.
        let prompt_id = self.own.uuid(0x3000 + turn.index);
        let call = Scenario::claude_turn_call(&self.session, &prompt_id, &turn.prompt);
        let stream = |event: Value| {
            json!({ "type": "stream_event", "event": event, "session_id": self.session,
                    "parent_tool_use_id": null })
        };
        vec![
            Out::Send(stream(
                json!({ "type": "message_start", "message": { "id": message } }),
            )),
            Out::Send(stream(json!({ "type": "content_block_start", "index": 0,
                "content_block": { "type": "text", "text": "" } }))),
            Out::Call(call),
        ]
    }

    /// A piece of the reply, as it streams.
    pub fn delta(&self, text: &str) -> Value {
        json!({ "type": "stream_event", "session_id": self.session, "parent_tool_use_id": null,
                "event": { "type": "content_block_delta", "index": 0,
                           "delta": { "type": "text_delta", "text": text } } })
    }

    /// The model call finished with `reply` (or failed with `error`).
    pub fn replied(&mut self, reply: Result<String, String>) -> Vec<Out> {
        if !matches!(
            self.turn.as_ref().map(|turn| &turn.step),
            Some(Step::Calling)
        ) {
            return Vec::new();
        }
        match reply {
            Ok(text) => {
                let mut out = vec![Out::Send(self.line(
                    "assistant",
                    json!({ "message": { "id": self.message_id(), "role": "assistant",
                                         "content": [{ "type": "text", "text": text }] } }),
                ))];
                out.extend(self.end(None));
                out
            }
            Err(message) => {
                let Some(_) = self.turn.take() else {
                    return Vec::new();
                };
                vec![Out::Send(self.line(
                    "result",
                    json!({ "subtype": "error_during_execution", "is_error": true,
                            "errors": [message], "terminal_reason": "model_error" }),
                ))]
            }
        }
    }

    /// End the turn: completed, or aborted for `aborted`.
    fn end(&mut self, aborted: Option<&str>) -> Vec<Out> {
        let Some(_) = self.turn.take() else {
            return Vec::new();
        };
        let result = match aborted {
            None => json!({ "subtype": "success", "is_error": false,
                            "terminal_reason": "completed" }),
            Some(reason) => json!({ "subtype": "error_during_execution", "is_error": true,
                                    "terminal_reason": reason }),
        };
        vec![Out::Send(self.line("result", result))]
    }
}

#[cfg(test)]
mod tests {
    use super::{ClaudeScript, Out};
    use crate::dev::seed::Run;
    use serde_json::{Value, json};

    const SESSION: &str = "0190f3a0-0000-7000-8000-0000000000d1";

    fn sent(out: &[Out]) -> Vec<Value> {
        out.iter()
            .filter_map(|out| match out {
                Out::Send(value) => Some(value.clone()),
                Out::Call(_) => None,
            })
            .collect()
    }

    fn opened() -> ClaudeScript {
        let arguments = [
            "-p".to_owned(),
            "--session-id".to_owned(),
            SESSION.to_owned(),
        ];
        let mut agent = ClaudeScript::new(Run(7), &arguments, "/work/acme".into()).unwrap();
        let out = agent.receive(&json!({ "type": "control_request", "request_id": "ren-0",
            "request": { "subtype": "initialize" } }));
        assert_eq!(sent(&out)[0]["response"]["subtype"], "success");
        agent
    }

    fn prompt(text: &str) -> Value {
        json!({ "type": "user", "message": { "role": "user", "content": text },
                "parent_tool_use_id": null, "session_id": SESSION })
    }

    #[test]
    fn a_plain_prompt_is_echoed_and_calls_the_model_in_the_named_session() {
        let mut agent = opened();
        let out = agent.receive(&prompt("explain the parser"));
        let echo = &sent(&out)[0];
        assert_eq!(echo["isReplay"], true);
        assert_eq!(echo["uuid"], Run(7).uuid(0x2001));
        let Some(Out::Call(call)) = out.last() else {
            panic!("a model call: {out:?}");
        };
        let header = |name: &str| {
            call.headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(header("x-claude-code-session-id").as_deref(), Some(SESSION));
        // Claude Code's prompt id is not the echo's message id.
        assert_ne!(header("x-claude-code-prompt-id"), Some(Run(7).uuid(0x2001)));
        assert_eq!(agent.delta("Hel")["event"]["delta"]["text"], "Hel");
        let done = agent.replied(Ok("Hello.".into()));
        let done = sent(&done);
        assert_eq!(done[0]["type"], "assistant");
        assert_eq!(done[1]["type"], "result");
        assert_eq!(done[1]["subtype"], "success");
    }

    #[test]
    fn a_prompt_to_run_tests_asks_first_and_runs_once_allowed() {
        let mut agent = opened();
        let out = agent.receive(&prompt("run the tests"));
        let ask = sent(&out).last().cloned().unwrap();
        assert_eq!(ask["request"]["subtype"], "can_use_tool");
        assert_eq!(ask["request"]["tool_name"], "Bash");
        assert!(!out.iter().any(|out| matches!(out, Out::Call(_))));
        let out = agent.receive(&json!({ "type": "control_response", "response": {
            "subtype": "success", "request_id": ask["request_id"], "response": {
                "behavior": "allow", "updatedInput": ask["request"]["input"] } } }));
        let result = &sent(&out)[0];
        assert_eq!(result["message"]["content"][0]["is_error"], false);
        assert!(matches!(out.last(), Some(Out::Call(_))));
    }

    #[test]
    fn cancelling_refuses_the_write_and_ends_the_turn_aborted() {
        let mut agent = opened();
        let out = agent.receive(&prompt("fix the bug"));
        let ask = sent(&out).last().cloned().unwrap();
        assert_eq!(ask["request"]["tool_name"], "Write");
        let out = agent.receive(&json!({ "type": "control_response", "response": {
            "subtype": "success", "request_id": ask["request_id"], "response": {
                "behavior": "deny", "message": "no", "interrupt": true } } }));
        let out = sent(&out);
        assert_eq!(
            out[0]["tool_result_meta"][0]["non_execution_kind"],
            "user-rejected"
        );
        assert_eq!(out[1]["terminal_reason"], "aborted_tools");
        assert!(agent.replied(Ok("late".into())).is_empty());
    }

    #[test]
    fn an_interrupt_ends_the_running_turn() {
        let mut agent = opened();
        let _ = agent.receive(&prompt("explain"));
        let out = agent.receive(&json!({ "type": "control_request", "request_id": "ren-1",
            "request": { "subtype": "interrupt" } }));
        let out = sent(&out);
        assert_eq!(out[0]["type"], "control_response");
        assert_eq!(out[1]["terminal_reason"], "aborted_streaming");
    }

    /// Ren's own listing codec reads every line this fake answers with.
    fn listed(
        harness: ren_harness::Harness,
        mut answer: impl FnMut(&Value) -> Vec<Value>,
    ) -> Vec<ren_harness::drive::Model> {
        use ren_harness::drive::{Framed, Listing, ModelListing};
        let (mut listing, mut lines) = ModelListing::open(harness, "0.1.0").unwrap();
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
    fn its_models_read_as_claude_code_lists_them() {
        let arguments = ren_harness::claude_code::ClaudeModels::arguments();
        let mut agent = ClaudeScript::new(Run(7), &arguments, "/work".into()).unwrap();
        let models = listed(ren_harness::Harness::ClaudeCode, |message| {
            sent(&agent.receive(message))
        });
        let names: Vec<&str> = models.iter().map(|model| model.name.as_str()).collect();
        assert_eq!(names, ["Dev Large", "Dev Huge", "Dev Small", "Dev Large 0"]);
        assert_eq!(models[0].choice, ren_harness::drive::DefaultChoice::Default);
    }
}
