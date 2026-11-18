//! Claude Code's listing processes: the models and the folder's commands a
//! short `claude -p` process in stream-json answers `initialize` with.

use super::stream_json::{ClaudeMessageType, ClaudeStreamJson};
use crate::{
    commands::ClientCommand,
    lines::Line,
    models::{DefaultChoice, Listing, ListingAnswer, Model, ModelId, ModelListing},
};
use ren_protocol::label::Stored;
use serde_json::{Value, json};

/// A listing process's side of `claude -p` in stream-json: the models are in
/// Claude Code's answer to `initialize`. The listing keeps no session.
#[derive(Debug)]
pub struct ClaudeModels;

/// A listing process's side of `claude -p` for a folder's commands: its
/// answer to `initialize` lists them, Claude Code's own and those of the
/// folder it runs in.
#[derive(Debug)]
pub struct ClaudeCommands;

impl ClaudeStreamJson {
    /// What `message` says to the control request `request`: a listing's
    /// one question.
    pub(crate) fn answer_to<'a>(message: &'a Value, request: &str) -> ListingAnswer<'a> {
        let kind = message
            .get("type")
            .and_then(Value::as_str)
            .map(Stored::<ClaudeMessageType>::from_label);
        let response = &message["response"];
        if kind != Some(Stored::Known(ClaudeMessageType::ControlResponse))
            || response.get("request_id").and_then(Value::as_str) != Some(request)
        {
            return ListingAnswer::NotMine;
        }
        if response.get("subtype").and_then(Value::as_str) == Some("success") {
            ListingAnswer::Answered(&response["response"])
        } else {
            ListingAnswer::Refused(response)
        }
    }
}

impl ClaudeCommands {
    const REQUEST: &'static str = "ren-commands";

    /// The codec, and the first line to send.
    pub fn open() -> (Self, Line) {
        let line = Line::json(&json!({
            "type": "control_request",
            "request_id": Self::REQUEST,
            "request": { "subtype": "initialize" },
        }));
        (Self, line)
    }

    pub(crate) fn receive(&mut self, message: &Value) -> Listing<Vec<ClientCommand>> {
        let answer = match ClaudeStreamJson::answer_to(message, Self::REQUEST) {
            ListingAnswer::NotMine => return Listing::waiting(),
            ListingAnswer::Refused(refusal) => {
                return Listing::Failed(ListingAnswer::reason(refusal));
            }
            ListingAnswer::Answered(answer) => answer,
        };
        Listing::Listed(ClaudeStreamJson::listed(answer))
    }
}

impl ClaudeModels {
    const REQUEST: &'static str = "ren-models";
    /// The entry Claude Code lists for "no model chosen".
    const DEFAULT: &'static str = "default";

    /// The arguments that start a listing process, after `claude`.
    pub fn arguments() -> Vec<String> {
        let mut arguments: Vec<String> = ClaudeStreamJson::MODE.map(str::to_owned).into();
        arguments.push("--no-session-persistence".into());
        arguments
    }

    /// The codec, and the first line to send.
    pub fn open() -> (Self, Line) {
        let line = Line::json(&json!({
            "type": "control_request",
            "request_id": Self::REQUEST,
            "request": { "subtype": "initialize" },
        }));
        (Self, line)
    }

    pub(crate) fn receive(&mut self, message: &Value) -> Listing {
        let answer = match ClaudeStreamJson::answer_to(message, Self::REQUEST) {
            ListingAnswer::NotMine => return Listing::waiting(),
            ListingAnswer::Refused(refusal) => {
                return Listing::Failed(ListingAnswer::reason(refusal));
            }
            ListingAnswer::Answered(answer) => answer,
        };
        let Some(entries) = answer.get("models").and_then(Value::as_array) else {
            return Listing::Failed("Claude Code listed no models".into());
        };
        // The "default" entry stands for no choice; the first entry it
        // resolves to is marked as the default instead.
        let resolved = entries
            .iter()
            .find(|entry| ModelListing::text(entry, "value").as_deref() == Some(Self::DEFAULT))
            .and_then(|entry| ModelListing::text(entry, "resolvedModel"));
        let mut marked = false;
        let models = entries
            .iter()
            .filter_map(|entry| {
                let value = ModelListing::text(entry, "value")?;
                if value == Self::DEFAULT {
                    return None;
                }
                let id: ModelId = value.parse().ok()?;
                let default = !marked
                    && resolved.is_some()
                    && ModelListing::text(entry, "resolvedModel") == resolved;
                marked |= default;
                let (name, description) = Self::named(
                    ModelListing::text(entry, "displayName").unwrap_or_else(|| id.to_string()),
                    ModelListing::text(entry, "description"),
                );
                Some(Model {
                    name,
                    description,
                    provider: None,
                    choice: if default {
                        DefaultChoice::Default
                    } else {
                        DefaultChoice::Other
                    },
                    // Claude Code lists the efforts a model takes, not which
                    // one it starts at.
                    efforts: match entry.get("supportsEffort").and_then(Value::as_bool) {
                        Some(true) => {
                            ModelListing::efforts(entry.get("supportedEffortLevels"), None)
                        }
                        Some(false) | None => Vec::new(),
                    },
                    effort: None,
                    id,
                })
            })
            .collect();
        Listing::Listed(models)
    }

    /// A model's name and description as Claude Code lists them: its short
    /// name ("Opus"), and a description that opens with the versioned name
    /// ("Opus 5.5 · Best for everyday tasks"). The versioned name is the
    /// model's name, and the rest its description.
    fn named(name: String, description: Option<String>) -> (String, Option<String>) {
        const SEPARATOR: char = '·';
        let Some(text) = description else {
            return (name, None);
        };
        match text.split_once(SEPARATOR) {
            Some((versioned, rest))
                if versioned
                    .trim_end()
                    .strip_prefix(name.as_str())
                    .is_some_and(|version| version.starts_with(' ')) =>
            {
                let rest = rest.trim();
                (
                    versioned.trim().to_owned(),
                    (!rest.is_empty()).then(|| rest.to_owned()),
                )
            }
            Some(_) | None => (name, Some(text)),
        }
    }
}
