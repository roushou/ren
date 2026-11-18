//! Codex's listing processes: a folder's skills and the models, each read
//! from a short `codex app-server` process.

use super::app_server::{CodexAppServer, CodexMethod, METHOD_NOT_FOUND};
use crate::{
    commands::ClientCommand,
    lines::Line,
    models::{DefaultChoice, Listing, Model, ModelId, ModelListing},
};
use ren_protocol::label::Labeled;
use serde_json::{Value, json};

/// A listing process's side of `codex app-server` for a folder's skills:
/// `initialize`, then `skills/list` for that folder.
#[derive(Debug)]
pub struct CodexCommands {
    /// The folder whose skills are listed.
    folder: String,
    waiting: CodexListStep,
}

impl CodexCommands {
    const INITIALIZE: u64 = 0;
    const SKILLS: u64 = 1;

    /// The codec, and the first line to send. `version` is Ren's own version.
    pub fn open(version: &str, folder: &str) -> (Self, Line) {
        let (_, line) = CodexModels::open(version);
        (
            Self {
                folder: folder.to_owned(),
                waiting: CodexListStep::Initialize,
            },
            line,
        )
    }

    pub(crate) fn receive(&mut self, message: &Value) -> Listing<Vec<ClientCommand>> {
        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            return Listing::waiting();
        };
        if message.get("method").is_some() {
            // A request from the server: a listing answers none.
            return Listing::Waiting(vec![Line::json(&json!({
                "id": message["id"],
                "error": { "code": METHOD_NOT_FOUND, "message": "Ren is only listing skills" },
            }))]);
        }
        if let Some(error) = message.get("error") {
            return Listing::Failed(
                ModelListing::text(error, "message").unwrap_or_else(|| "refused".into()),
            );
        }
        match self.waiting {
            CodexListStep::Initialize if id == Self::INITIALIZE => {
                self.waiting = CodexListStep::List(Self::SKILLS);
                Listing::Waiting(vec![
                    Line::json(&json!({ "method": CodexMethod::Initialized.label() })),
                    Line::json(&json!({
                        "id": Self::SKILLS,
                        "method": CodexMethod::SkillsList.label(),
                        "params": { "cwds": [self.folder] },
                    })),
                ])
            }
            CodexListStep::List(request) if id == request => {
                self.waiting = CodexListStep::Done;
                Listing::Listed(
                    CodexAppServer::skills(&message["result"])
                        .into_iter()
                        .map(|(skill, _)| skill)
                        .collect(),
                )
            }
            CodexListStep::Initialize | CodexListStep::List(_) | CodexListStep::Done => {
                Listing::waiting()
            }
        }
    }
}

/// A listing process's side of `codex app-server`: `initialize`, then
/// `model/list`, page by page, without the models Codex hides from its own
/// picker.
#[derive(Debug)]
pub struct CodexModels {
    models: Vec<Model>,
    /// The request Ren waits on.
    waiting: CodexListStep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexListStep {
    Initialize,
    List(u64),
    Done,
}

impl CodexModels {
    const INITIALIZE: u64 = 0;

    /// The codec, and the first line to send. `version` is Ren's own version.
    pub fn open(version: &str) -> (Self, Line) {
        let line = Line::json(&json!({
            "id": Self::INITIALIZE,
            "method": CodexMethod::Initialize.label(),
            "params": { "clientInfo": { "name": "ren", "title": "Ren", "version": version } },
        }));
        (
            Self {
                models: Vec::new(),
                waiting: CodexListStep::Initialize,
            },
            line,
        )
    }

    pub(crate) fn receive(&mut self, message: &Value) -> Listing {
        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            return Listing::waiting();
        };
        if message.get("method").is_some() {
            // A request from the server: a listing answers none.
            return Listing::Waiting(vec![Line::json(&json!({
                "id": message["id"],
                "error": { "code": METHOD_NOT_FOUND, "message": "Ren is only listing models" },
            }))]);
        }
        if let Some(error) = message.get("error") {
            return Listing::Failed(
                ModelListing::text(error, "message").unwrap_or_else(|| "refused".into()),
            );
        }
        match self.waiting {
            CodexListStep::Initialize if id == Self::INITIALIZE => Listing::Waiting(vec![
                Line::json(&json!({ "method": CodexMethod::Initialized.label() })),
                self.page(1, None),
            ]),
            CodexListStep::List(request) if id == request => {
                let result = &message["result"];
                let Some(data) = result.get("data").and_then(Value::as_array) else {
                    return Listing::Failed("Codex listed no models".into());
                };
                self.models.extend(data.iter().filter_map(Self::model));
                match ModelListing::text(result, "nextCursor") {
                    Some(cursor) => Listing::Waiting(vec![self.page(request + 1, Some(cursor))]),
                    None => {
                        self.waiting = CodexListStep::Done;
                        Listing::Listed(std::mem::take(&mut self.models))
                    }
                }
            }
            CodexListStep::Initialize | CodexListStep::List(_) | CodexListStep::Done => {
                Listing::waiting()
            }
        }
    }

    fn page(&mut self, request: u64, cursor: Option<String>) -> Line {
        self.waiting = CodexListStep::List(request);
        Line::json(&json!({
            "id": request,
            "method": CodexMethod::ModelList.label(),
            "params": { "includeHidden": false, "cursor": cursor },
        }))
    }

    /// One listed model; its `model` slug is what `thread/start` takes.
    fn model(entry: &Value) -> Option<Model> {
        if entry.get("hidden").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let id: ModelId = ModelListing::text(entry, "model")
            .or_else(|| ModelListing::text(entry, "id"))?
            .parse()
            .ok()?;
        Some(Model {
            name: ModelListing::text(entry, "displayName").unwrap_or_else(|| id.to_string()),
            description: ModelListing::text(entry, "description"),
            provider: None,
            choice: match entry.get("isDefault").and_then(Value::as_bool) {
                Some(true) => DefaultChoice::Default,
                Some(false) | None => DefaultChoice::Other,
            },
            efforts: ModelListing::efforts(
                entry.get("supportedReasoningEfforts"),
                Some("reasoningEffort"),
            ),
            effort: ModelListing::text(entry, "defaultReasoningEffort")
                .and_then(|effort| effort.parse().ok()),
            id,
        })
    }
}
