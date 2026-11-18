//! The commands a client offers a prompt: its slash commands, skills, and
//! prompt templates, as the client itself lists them for a run. Ren keeps
//! no list of its own; a prompt names one in the client's own way
//! ([`Sigil`]) and goes to the client as written.

use crate::Harness;
use crate::claude_code::ClaudeCommands;
use crate::codex::CodexCommands;
use crate::lines::{Framed, Line};
use crate::models::Listing;
use crate::opencode::OpenCodeCommands;
use crate::pi::PiCommands;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// One listing process's codec for the commands a client offers in a
/// folder, chosen by its harness: the client started briefly in that folder
/// in its structured mode and asked, as for its models, with no
/// conversation kept.
#[derive(Debug)]
pub enum CommandListing {
    Codex(CodexCommands),
    ClaudeCode(ClaudeCommands),
    Pi(PiCommands),
    OpenCode(OpenCodeCommands),
}

impl CommandListing {
    /// The codec for a freshly started listing process of `harness` in
    /// `folder` (absolute), and the lines to send it first. `version` is
    /// Ren's own version.
    pub fn open(harness: Harness, version: &str, folder: &str) -> (Self, Vec<Line>) {
        match harness {
            Harness::Codex => {
                let (codec, first) = CodexCommands::open(version, folder);
                (Self::Codex(codec), vec![first])
            }
            Harness::ClaudeCode => {
                let (codec, first) = ClaudeCommands::open();
                (Self::ClaudeCode(codec), vec![first])
            }
            Harness::Pi => {
                let (codec, first) = PiCommands::open();
                (Self::Pi(codec), vec![first])
            }
            Harness::OpenCode => {
                let (codec, first) = OpenCodeCommands::open(version, folder);
                (Self::OpenCode(codec), vec![first])
            }
        }
    }

    /// Read one frame from the client's stdout.
    pub fn receive(&mut self, frame: Framed) -> Listing<Vec<ClientCommand>> {
        let Framed::Line(bytes) = frame else {
            return Listing::waiting();
        };
        let Ok(message) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return Listing::waiting();
        };
        match self {
            Self::Codex(codec) => codec.receive(&message),
            Self::ClaudeCode(codec) => codec.receive(&message),
            Self::Pi(codec) => codec.receive(&message),
            Self::OpenCode(codec) => codec.receive(&message),
        }
    }
}

/// One command a running client offers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClientCommand {
    /// Its name as the client lists it, without the sigil.
    pub name: String,
    /// What it does, in the client's words; empty when it gives none.
    pub description: String,
    /// What it takes after its name (`<file or PR>`), when the client says.
    #[serde(default)]
    pub hint: Option<String>,
    pub kind: CommandKind,
    /// How a prompt names it.
    pub sigil: Sigil,
}

impl ClientCommand {
    /// What a prompt writes to run it: its sigil, then its name.
    pub fn invocation(&self) -> String {
        format!("{}{}", self.sigil, self.name)
    }
}

/// What a command is to its client.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum CommandKind {
    /// A command the client or one of its extensions runs.
    Command,
    /// A skill: instructions the model reads when the prompt names it.
    Skill,
    /// A prompt template, expanded into the prompt.
    Template,
}

/// How a prompt names a command: `/name` (Claude Code, Pi) or `$name`
/// (Codex's skills).
#[derive(
    Display,
    EnumString,
    IntoStaticStr,
    VariantArray,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    SerializeDisplay,
    DeserializeFromStr,
)]
pub enum Sigil {
    #[strum(to_string = "/")]
    Slash,
    #[strum(to_string = "$")]
    Dollar,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_named_by_its_sigil_and_name() {
        let command = ClientCommand {
            name: "review".into(),
            description: String::new(),
            hint: None,
            kind: CommandKind::Skill,
            sigil: Sigil::Dollar,
        };
        assert_eq!(command.invocation(), "$review");
        assert_eq!(
            serde_json::to_value(&command).unwrap(),
            serde_json::json!({ "name": "review", "description": "", "hint": null,
                                "kind": "skill", "sigil": "$" })
        );
    }

    /// Feed `answers` to a listing of `harness` in `/work/acme`, one line
    /// each, returning the lines it sent and how it ended.
    fn listing(
        harness: Harness,
        answers: &[serde_json::Value],
    ) -> (Vec<serde_json::Value>, Listing<Vec<ClientCommand>>) {
        let (mut codec, first) = CommandListing::open(harness, "0.1.0", "/work/acme");
        let read =
            |line: &Line| serde_json::from_slice::<serde_json::Value>(line.as_bytes()).unwrap();
        let mut sent: Vec<serde_json::Value> = first.iter().map(read).collect();
        let mut last = Listing::waiting();
        for answer in answers {
            last = codec.receive(Framed::Line(answer.to_string().into_bytes()));
            if let Listing::Waiting(lines) = &last {
                sent.extend(lines.iter().map(read));
            }
        }
        (sent, last)
    }

    fn names(listing: &Listing<Vec<ClientCommand>>) -> Vec<String> {
        match listing {
            Listing::Listed(commands) => commands.iter().map(ClientCommand::invocation).collect(),
            other @ (Listing::Waiting(_) | Listing::Failed(_)) => panic!("not listed: {other:?}"),
        }
    }

    #[test]
    fn claude_code_lists_its_commands_in_its_answer_to_initialize() {
        let (sent, listed) = listing(
            Harness::ClaudeCode,
            &[
                serde_json::json!({ "type": "control_response", "response": {
                "subtype": "success", "request_id": "ren-commands",
                "response": { "commands": [
                    { "name": "review", "description": "Review a pull request", "argumentHint": "<PR>" },
                    { "name": "release-notes", "description": "" } ] } } }),
            ],
        );
        assert_eq!(sent[0]["request"]["subtype"], "initialize");
        assert_eq!(names(&listed), ["/review", "/release-notes"]);
    }

    #[test]
    fn codex_lists_the_skills_of_the_folder() {
        let (sent, listed) = listing(
            Harness::Codex,
            &[
                serde_json::json!({ "id": 0, "result": { "userAgent": "codex" } }),
                serde_json::json!({ "id": 1, "result": { "data": [{ "cwd": "/work/acme", "skills": [
                    { "name": "changelog", "path": "/work/acme/.codex/skills/changelog/SKILL.md",
                      "description": "Keep the changelog", "enabled": true },
                    { "name": "off", "path": "/x", "description": "", "enabled": false } ] }] } }),
            ],
        );
        assert_eq!(sent[2]["method"], "skills/list");
        assert_eq!(
            sent[2]["params"],
            serde_json::json!({ "cwds": ["/work/acme"] })
        );
        assert_eq!(names(&listed), ["$changelog"]);
    }

    #[test]
    fn pi_lists_its_commands_in_its_answer_to_get_commands() {
        let (sent, listed) = listing(
            Harness::Pi,
            &[
                serde_json::json!({ "id": "ren-commands", "type": "response",
                "command": "get_commands", "success": true,
                "data": { "commands": [{ "name": "fix-tests", "description": "Fix the tests",
                                         "source": "prompt" }] } }),
            ],
        );
        assert_eq!(sent[0]["type"], "get_commands");
        assert_eq!(names(&listed), ["/fix-tests"]);
        let (_, refused) = listing(
            Harness::Pi,
            &[
                serde_json::json!({ "id": "ren-commands", "type": "response",
                "command": "get_commands", "success": false, "error": "no" }),
            ],
        );
        assert_eq!(refused, Listing::Failed("no".into()));
    }

    #[test]
    fn opencode_lists_the_commands_of_a_throwaway_session_then_deletes_it() {
        let update = serde_json::json!({ "jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": "ses_1", "update": { "sessionUpdate": "available_commands_update",
            "availableCommands": [{ "name": "review", "description": "review changes" }] } } });
        let (sent, listed) = listing(
            Harness::OpenCode,
            &[
                serde_json::json!({ "jsonrpc": "2.0", "id": 0, "result": { "protocolVersion": 1 } }),
                serde_json::json!({ "jsonrpc": "2.0", "id": 1, "result": { "sessionId": "ses_1" } }),
                update.clone(),
                serde_json::json!({ "jsonrpc": "2.0", "id": 2, "result": {} }),
            ],
        );
        assert_eq!(sent[1]["method"], "session/new");
        assert_eq!(sent[1]["params"]["cwd"], "/work/acme");
        assert_eq!(sent[2]["method"], "session/delete");
        assert_eq!(sent[2]["params"]["sessionId"], "ses_1");
        assert_eq!(names(&listed), ["/review"]);
        // The commands may come before the session's answer.
        let (sent, listed) = listing(
            Harness::OpenCode,
            &[
                serde_json::json!({ "jsonrpc": "2.0", "id": 0, "result": { "protocolVersion": 1 } }),
                update,
                serde_json::json!({ "jsonrpc": "2.0", "id": 1, "result": { "sessionId": "ses_1" } }),
                serde_json::json!({ "jsonrpc": "2.0", "id": 2, "error": { "code": 1, "message": "gone" } }),
            ],
        );
        assert_eq!(sent[2]["method"], "session/delete");
        assert_eq!(
            names(&listed),
            ["/review"],
            "listed even when the delete fails"
        );
    }
}
