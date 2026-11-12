//! What the dev slot's fake Pi says in RPC mode, as a pure state machine:
//! Ren's commands in, lines out, and the model calls to make. Pi asks for no
//! approvals, so a prompt about testing, running, or building runs `bash`
//! at once, and one about fixing, editing, changing, adding, or writing
//! runs `edit`; then one model call goes through the gateway, under the
//! session Ren named and a turn id of Pi's own (as Ren's extension sends
//! them), and its reply streams back. The run settles after it.

use crate::dev::seed::{Call, Run, Scenario};
use serde_json::{Value, json};

/// What the shell does next.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// Write this line to stdout.
    Send(Value),
    /// Make the turn's model call; its reply goes to [`PiScript::delta`]
    /// and [`PiScript::replied`].
    Call(Call),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ask {
    Command,
    Edit,
}

impl Ask {
    fn of(prompt: &str) -> Option<Self> {
        let prompt = prompt.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|word| prompt.contains(word));
        if has(&["test", "run", "build"]) {
            Some(Self::Command)
        } else if has(&["fix", "edit", "change", "add", "write"]) {
            Some(Self::Edit)
        } else {
            None
        }
    }
}

/// The turn running, while its model call is out.
#[derive(Debug, Clone, PartialEq)]
struct Active {
    index: u32,
}

/// One fake Pi process's session.
pub struct PiScript {
    /// This process's own number, for the ids it makes.
    own: Run,
    /// The session Ren named with `--session-id`.
    session: String,
    turns: u32,
    turn: Option<Active>,
}

impl PiScript {
    /// `arguments` are those the daemon gave `pi`.
    pub fn new(own: Run, arguments: &[String]) -> Option<Self> {
        let session = arguments
            .windows(2)
            .find(|pair| pair[0] == "--session-id")
            .map(|pair| pair[1].clone())
            // A listing of models keeps no session.
            .or_else(|| {
                arguments
                    .iter()
                    .any(|argument| argument == "--no-session")
                    .then(String::new)
            })?;
        Some(Self {
            own,
            session,
            turns: 0,
            turn: None,
        })
    }

    /// The models it lists, by provider: invented.
    const MODELS: [(&'static str, &'static str, &'static str); 6] = [
        ("anthropic", "dev-large-1", "Dev Large"),
        ("anthropic", "dev-small-1", "Dev Small"),
        ("openai", "dev-swift", "Dev Swift"),
        ("openai", "dev-sage", "Dev Sage"),
        ("openrouter", "dev-open-70b", "Dev Open 70B"),
        ("openrouter", "dev-open-8b", "Dev Open 8B"),
    ];

    fn response(id: &Value, command: &str, data: Value) -> Out {
        Out::Send(
            json!({ "id": id, "type": "response", "command": command, "success": true,
                          "data": data }),
        )
    }

    /// Read one command from Ren.
    pub fn receive(&mut self, message: &Value) -> Vec<Out> {
        let id = &message["id"];
        match message["type"].as_str() {
            Some("get_state") => vec![Self::response(
                id,
                "get_state",
                json!({ "sessionId": self.session, "isStreaming": self.turn.is_some(),
                        "messageCount": self.turns * 2,
                        "model": { "provider": "anthropic", "id": "dev-large-1" },
                        "thinkingLevel": "medium" }),
            )],
            Some("get_available_models") => vec![Self::response(
                id,
                "get_available_models",
                json!({ "models": Self::MODELS.map(|(provider, id, name)| json!({
                    "provider": provider, "id": id, "name": name,
                    "reasoning": !id.ends_with("8b") })) }),
            )],
            Some("prompt") => self.prompted(id, message["message"].as_str().unwrap_or_default()),
            Some("abort") => {
                let mut out = self.settle(None, true);
                out.push(Self::response(id, "abort", Value::Null));
                out
            }
            Some(_) | None => Vec::new(),
        }
    }

    fn prompted(&mut self, id: &Value, prompt: &str) -> Vec<Out> {
        if self.turn.is_some() {
            // The real client asks for a streaming behavior; Ren never sends one.
            return vec![Out::Send(
                json!({ "id": id, "type": "response", "command": "prompt",
                "success": false, "error": "Agent is already processing" }),
            )];
        }
        self.turns += 1;
        let index = self.turns;
        self.turn = Some(Active { index });
        let mut out = vec![
            Self::response(id, "prompt", json!({ "disposition": "started" })),
            Out::Send(json!({ "type": "agent_start" })),
            Out::Send(json!({ "type": "turn_start" })),
            Out::Send(
                json!({ "type": "message_start", "message": { "role": "user",
                "content": [{ "type": "text", "text": prompt }], "timestamp": index } }),
            ),
            Out::Send(json!({ "type": "message_end", "message": { "role": "user",
                "content": [{ "type": "text", "text": prompt }], "timestamp": index } })),
        ];
        if let Some(ask) = Ask::of(prompt) {
            out.extend(self.tool(index, ask));
        }
        out.push(Out::Send(json!({ "type": "message_start",
            "message": { "role": "assistant", "content": [], "stopReason": "pending" } })));
        out.push(Out::Call(Scenario::pi_turn_call(
            &self.session,
            &self.own.uuid(0x4000 + index),
            prompt,
        )));
        out
    }

    /// The tool call the model asked for, run at once.
    fn tool(&self, index: u32, ask: Ask) -> Vec<Out> {
        let id = format!("toolu_pi{:04}{index:04}", self.own.0 % 10_000);
        let (name, args, result) = match ask {
            Ask::Command => (
                "bash",
                json!({ "command": "cargo test --workspace" }),
                "running 12 tests\ntest result: ok. 12 passed; 0 failed",
            ),
            Ask::Edit => (
                "edit",
                json!({ "path": "src/lib.rs", "oldText": "todo!()", "newText": "Ok(())" }),
                "Successfully replaced text in src/lib.rs.",
            ),
        };
        let call = json!({ "type": "toolCall", "id": id, "name": name, "arguments": args });
        vec![
            Out::Send(json!({ "type": "message_start",
                "message": { "role": "assistant", "content": [], "stopReason": "pending" } })),
            Out::Send(
                json!({ "type": "message_end", "message": { "role": "assistant",
                "content": [call], "stopReason": "toolUse" } }),
            ),
            Out::Send(json!({ "type": "tool_execution_start", "toolCallId": id,
                "toolName": name, "args": args })),
            Out::Send(
                json!({ "type": "tool_execution_end", "toolCallId": id, "toolName": name,
                "result": { "content": [{ "type": "text", "text": result }] },
                "isError": false }),
            ),
            Out::Send(json!({ "type": "turn_end", "message": {}, "toolResults": [] })),
            Out::Send(json!({ "type": "turn_start" })),
        ]
    }

    /// A piece of the reply, as it streams.
    pub fn delta(&self, text: &str) -> Value {
        json!({ "type": "message_update", "assistantMessageEvent": { "type": "text_delta",
                "contentIndex": 0, "delta": text } })
    }

    /// The model call finished with `reply` (or failed with `error`).
    pub fn replied(&mut self, reply: Result<String, String>) -> Vec<Out> {
        if self.turn.is_none() {
            return Vec::new();
        }
        let message = match &reply {
            Ok(text) => json!({ "role": "assistant", "stopReason": "stop",
                                "content": [{ "type": "text", "text": text }] }),
            Err(error) => json!({ "role": "assistant", "stopReason": "error",
                                  "errorMessage": error, "content": [] }),
        };
        let mut out = vec![
            Out::Send(json!({ "type": "message_end", "message": message })),
            Out::Send(json!({ "type": "turn_end", "message": message, "toolResults": [] })),
        ];
        out.extend(self.settle(Some(message), false));
        out
    }

    /// End the run: after its last message, or aborted.
    fn settle(&mut self, last: Option<Value>, aborted: bool) -> Vec<Out> {
        let Some(_) = self.turn.take() else {
            return Vec::new();
        };
        vec![
            Out::Send(
                json!({ "type": "agent_end", "messages": last.into_iter().collect::<Vec<_>>(),
                              "willRetry": false }),
            ),
            Out::Send(json!({ "type": "agent_settled", "aborted": aborted })),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{Out, PiScript};
    use crate::dev::seed::Run;
    use serde_json::{Value, json};

    const SESSION: &str = "0190f3a0-0000-7000-8000-0000000000e1";

    fn sent(out: &[Out]) -> Vec<Value> {
        out.iter()
            .filter_map(|out| match out {
                Out::Send(value) => Some(value.clone()),
                Out::Call(_) => None,
            })
            .collect()
    }

    fn types(out: &[Out]) -> Vec<String> {
        sent(out)
            .iter()
            .map(|line| line["type"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    fn opened() -> PiScript {
        let arguments = [
            "--extension".to_owned(),
            "/cache/ren.ts".to_owned(),
            "--mode".to_owned(),
            "rpc".to_owned(),
            "--session-id".to_owned(),
            SESSION.to_owned(),
        ];
        let mut agent = PiScript::new(Run(7), &arguments).unwrap();
        let out = agent.receive(&json!({ "type": "get_state", "id": "ren-0" }));
        assert_eq!(sent(&out)[0]["data"]["sessionId"], SESSION);
        agent
    }

    #[test]
    fn a_prompt_runs_to_a_reply_in_the_named_session_and_settles() {
        let mut agent = opened();
        let out = agent.receive(&json!({ "type": "prompt", "id": "ren-1",
                                         "message": "explain the parser" }));
        assert_eq!(sent(&out)[0]["data"]["disposition"], "started");
        let Some(Out::Call(call)) = out.last() else {
            panic!("a model call: {out:?}");
        };
        let header = |name: &str| {
            call.headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(header("x-ren-pi-session-id").as_deref(), Some(SESSION));
        assert!(header("x-ren-pi-turn-id").is_some());
        // A second prompt while the turn runs is refused, as Pi does.
        let busy = agent.receive(&json!({ "type": "prompt", "id": "ren-2", "message": "and" }));
        assert_eq!(sent(&busy)[0]["success"], false);
        assert_eq!(agent.delta("Hel")["assistantMessageEvent"]["delta"], "Hel");
        let done = agent.replied(Ok("Hello.".into()));
        assert_eq!(
            types(&done),
            ["message_end", "turn_end", "agent_end", "agent_settled"]
        );
        assert_eq!(sent(&done)[3]["aborted"], false);
    }

    #[test]
    fn a_prompt_to_run_tests_runs_bash_without_asking() {
        let mut agent = opened();
        let out = agent.receive(&json!({ "type": "prompt", "id": "ren-1",
                                         "message": "run the tests" }));
        let lines = sent(&out);
        let start = lines
            .iter()
            .find(|line| line["type"] == "tool_execution_start")
            .unwrap();
        assert_eq!(start["toolName"], "bash");
        assert_eq!(start["args"]["command"], "cargo test --workspace");
        assert!(matches!(out.last(), Some(Out::Call(_))));
    }

    #[test]
    fn an_abort_settles_the_running_turn_aborted() {
        let mut agent = opened();
        let _ = agent.receive(&json!({ "type": "prompt", "id": "ren-1", "message": "explain" }));
        let out = agent.receive(&json!({ "type": "abort", "id": "ren-2" }));
        assert_eq!(types(&out), ["agent_end", "agent_settled", "response"]);
        assert_eq!(sent(&out)[1]["aborted"], true);
        assert!(agent.replied(Ok("late".into())).is_empty());
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
    fn its_models_read_as_pi_lists_them() {
        let arguments = ren_harness::pi::PiModels::arguments();
        let mut agent = PiScript::new(Run(7), &arguments).unwrap();
        let models = listed(ren_harness::Harness::Pi, |message| {
            sent(&agent.receive(message))
        });
        assert_eq!(models.len(), 6);
        assert_eq!(models[0].id.as_str(), "anthropic/dev-large-1");
        assert_eq!(
            models[0].choice,
            ren_harness::models::DefaultChoice::Default
        );
    }
}
