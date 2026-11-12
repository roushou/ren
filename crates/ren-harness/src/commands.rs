//! The commands a client offers a prompt: its slash commands, skills, and
//! prompt templates, as the client itself lists them for a run. Ren keeps
//! no list of its own; a prompt names one in the client's own way
//! ([`Sigil`]) and goes to the client as written.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

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
}
