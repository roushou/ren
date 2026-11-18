//! OpenCode's listing processes: a folder's commands and the models, each
//! read from a short `opencode acp` process.

use super::acp::{AcpMethod, AcpUpdate, OpenCodeAcp};
use crate::{
    commands::ClientCommand,
    lines::Line,
    models::{DefaultChoice, Listing, Model, ModelId, ModelListing},
};
use ren_protocol::label::Labeled;
use serde_json::{Value, json};

/// One `opencode acp` listing process for a folder's commands: a session
/// opened there only for the commands opencode lists to it, then deleted.
#[derive(Debug)]
pub struct OpenCodeCommands {
    /// The folder the throwaway session opens in: absolute, as ACP requires.
    folder: String,
    step: CommandStep,
    /// The throwaway session, once opencode named it.
    session: Option<String>,
    /// The commands, once opencode listed them to the session.
    commands: Option<Vec<ClientCommand>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandStep {
    Initializing,
    Opening,
    Deleting,
}

impl OpenCodeCommands {
    const INITIALIZE: u64 = 0;
    const NEW: u64 = 1;
    const DELETE: u64 = 2;

    /// A codec for a freshly started listing process in `folder`, and its
    /// first line.
    pub fn open(version: &str, folder: &str) -> (Self, Line) {
        let (_, line) = OpenCodeModels::open(version, folder);
        (
            Self {
                folder: folder.to_owned(),
                step: CommandStep::Initializing,
                session: None,
                commands: None,
            },
            line,
        )
    }

    pub(crate) fn receive(&mut self, message: &Value) -> Listing<Vec<ClientCommand>> {
        if message.get("method").and_then(Value::as_str) == Some(AcpMethod::SessionUpdate.label()) {
            let update = &message["params"]["update"];
            let commands = update.get("sessionUpdate").and_then(Value::as_str)
                == Some(AcpUpdate::AvailableCommandsUpdate.label());
            if commands {
                self.commands = Some(OpenCodeAcp::listed_commands(update));
            }
            return self.delete_when_listed();
        }
        if message.get("method").is_some() {
            return Listing::waiting();
        }
        let id = message.get("id").and_then(Value::as_u64);
        if let Some(error) = message.get("error") {
            if self.step == CommandStep::Deleting {
                // Listed whether or not the session could be deleted.
                return Listing::Listed(self.commands.take().unwrap_or_default());
            }
            return Listing::Failed(
                ModelListing::text(error, "message").unwrap_or_else(|| "opencode refused".into()),
            );
        }
        match (self.step, id) {
            (CommandStep::Initializing, Some(Self::INITIALIZE)) => {
                self.step = CommandStep::Opening;
                Listing::Waiting(vec![Line::json(&json!({
                    "jsonrpc": "2.0",
                    "id": Self::NEW,
                    "method": AcpMethod::SessionNew.label(),
                    "params": { "cwd": self.folder, "mcpServers": [] },
                }))])
            }
            (CommandStep::Opening, Some(Self::NEW)) => {
                self.session = ModelListing::text(&message["result"], "sessionId");
                if self.session.is_none() {
                    return Listing::Listed(self.commands.take().unwrap_or_default());
                }
                self.delete_when_listed()
            }
            (CommandStep::Deleting, Some(Self::DELETE)) => {
                Listing::Listed(self.commands.take().unwrap_or_default())
            }
            (CommandStep::Initializing | CommandStep::Opening | CommandStep::Deleting, _) => {
                Listing::waiting()
            }
        }
    }

    /// Once the session is named and its commands listed, delete it.
    fn delete_when_listed(&mut self) -> Listing<Vec<ClientCommand>> {
        if self.step == CommandStep::Opening
            && self.commands.is_some()
            && let Some(session) = &self.session
        {
            let line = Line::json(&json!({
                "jsonrpc": "2.0",
                "id": Self::DELETE,
                "method": AcpMethod::SessionDelete.label(),
                "params": { "sessionId": session },
            }));
            self.step = CommandStep::Deleting;
            return Listing::Waiting(vec![line]);
        }
        Listing::waiting()
    }
}

/// One `opencode acp` listing process: a session opened only to read the
/// model options opencode gives it, then deleted.
#[derive(Debug)]
pub struct OpenCodeModels {
    /// The folder the throwaway session opens in: absolute, as ACP requires.
    folder: String,
    step: ListStep,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ListStep {
    Initializing,
    Opening,
    /// The models are read; the session is being deleted.
    Deleting(Vec<Model>),
}

impl OpenCodeModels {
    const INITIALIZE: u64 = 0;
    const NEW: u64 = 1;
    const DELETE: u64 = 2;

    /// The arguments for a listing process: the same mode as a run.
    pub fn arguments() -> Vec<String> {
        OpenCodeAcp::arguments()
    }

    /// A codec for a freshly started listing process in `folder`, and its
    /// first line.
    pub fn open(version: &str, folder: &str) -> (Self, Line) {
        let line = Line::json(&json!({
            "jsonrpc": "2.0",
            "id": Self::INITIALIZE,
            "method": AcpMethod::Initialize.label(),
            "params": OpenCodeAcp::initialize(version),
        }));
        (
            Self {
                folder: folder.to_owned(),
                step: ListStep::Initializing,
            },
            line,
        )
    }

    pub(crate) fn receive(&mut self, message: &Value) -> Listing {
        let id = message.get("id").and_then(Value::as_u64);
        if message.get("method").is_some() {
            return Listing::waiting();
        }
        let error = message.get("error").map(|error| {
            ModelListing::text(error, "message").unwrap_or_else(|| "opencode refused".into())
        });
        let result = &message["result"];
        match (
            std::mem::replace(&mut self.step, ListStep::Initializing),
            id,
        ) {
            (ListStep::Initializing, Some(Self::INITIALIZE)) => {
                if let Some(error) = error {
                    return Listing::Failed(error);
                }
                self.step = ListStep::Opening;
                Listing::Waiting(vec![Line::json(&json!({
                    "jsonrpc": "2.0",
                    "id": Self::NEW,
                    "method": AcpMethod::SessionNew.label(),
                    "params": { "cwd": self.folder, "mcpServers": [] },
                }))])
            }
            (ListStep::Opening, Some(Self::NEW)) => {
                if let Some(error) = error {
                    return Listing::Failed(error);
                }
                let models = Self::models(result);
                // The session was only for its options: delete it, then list.
                match ModelListing::text(result, "sessionId") {
                    Some(session) => {
                        self.step = ListStep::Deleting(models);
                        Listing::Waiting(vec![Line::json(&json!({
                            "jsonrpc": "2.0",
                            "id": Self::DELETE,
                            "method": AcpMethod::SessionDelete.label(),
                            "params": { "sessionId": session },
                        }))])
                    }
                    None => Listing::Listed(models),
                }
            }
            // Listed whether or not the session could be deleted.
            (ListStep::Deleting(models), Some(Self::DELETE)) => Listing::Listed(models),
            (step @ (ListStep::Initializing | ListStep::Opening | ListStep::Deleting(_)), _) => {
                self.step = step;
                Listing::waiting()
            }
        }
    }

    /// The models in a session's `model` option, its current one the default,
    /// and the efforts of that one's `effort` option.
    fn models(result: &Value) -> Vec<Model> {
        let options = result
            .get("configOptions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let option = |id: &str| {
            options
                .iter()
                .find(|option| option.get("id").and_then(Value::as_str) == Some(id))
                .cloned()
        };
        let Some(model) = option("model") else {
            return Vec::new();
        };
        let current = ModelListing::text(&model, "currentValue");
        let effort = option("effort");
        let efforts = ModelListing::efforts(
            effort.as_ref().map(|effort| &effort["options"]),
            Some("value"),
        );
        let default_effort = effort
            .as_ref()
            .and_then(|effort| ModelListing::text(effort, "currentValue"))
            .and_then(|value| value.parse().ok());
        model
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let value = ModelListing::text(entry, "value")?;
                let id: ModelId = value.parse().ok()?;
                let chosen = current.as_deref() == Some(value.as_str());
                let provider = value
                    .split_once('/')
                    .map(|(provider, _)| provider.to_owned());
                let name = ModelListing::text(entry, "name").unwrap_or_else(|| value.clone());
                // opencode names a model `provider/Name`; the provider is
                // shown on its own.
                let name = provider
                    .as_ref()
                    .and_then(|provider| name.strip_prefix(&format!("{provider}/")))
                    .map_or_else(|| name.clone(), str::to_owned);
                Some(Model {
                    id,
                    name,
                    description: ModelListing::text(entry, "description"),
                    provider,
                    choice: if chosen {
                        DefaultChoice::Default
                    } else {
                        DefaultChoice::Other
                    },
                    efforts: if chosen { efforts.clone() } else { Vec::new() },
                    effort: if chosen { default_effort } else { None },
                })
            })
            .collect()
    }
}
