//! Pi's listing processes: a folder's commands and the models, each read
//! from a short `pi --mode rpc` process.

use super::rpc::{PiEvent, PiRpc};
use crate::{
    commands::ClientCommand,
    lines::Line,
    models::{DefaultChoice, Listing, ListingAnswer, Model, ModelId, ModelListing},
    settings::Effort,
};
use ren_protocol::label::Stored;
use serde_json::{Value, json};

/// A listing process's side of `pi --mode rpc` for a folder's commands: its
/// answer to `get_commands`, its extensions' commands, prompt templates,
/// and skills as the folder it runs in has them.
#[derive(Debug)]
pub struct PiCommands;

impl PiRpc {
    /// What `message` says to the request numbered `id`: a listing's
    /// question.
    pub(crate) fn answer_to<'a>(message: &'a Value, id: &str) -> ListingAnswer<'a> {
        let kind = message
            .get("type")
            .and_then(Value::as_str)
            .map(Stored::<PiEvent>::from_label);
        if kind != Some(Stored::Known(PiEvent::Response))
            || message.get("id").and_then(Value::as_str) != Some(id)
        {
            return ListingAnswer::NotMine;
        }
        if message.get("success").and_then(Value::as_bool) == Some(true) {
            ListingAnswer::Answered(&message["data"])
        } else {
            ListingAnswer::Refused(message)
        }
    }
}

impl PiCommands {
    const ID: &'static str = "ren-commands";

    /// The codec, and the first line to send.
    pub fn open() -> (Self, Line) {
        let line = Line::json(&json!({ "id": Self::ID, "type": "get_commands" }));
        (Self, line)
    }

    pub(crate) fn receive(&mut self, message: &Value) -> Listing<Vec<ClientCommand>> {
        match PiRpc::answer_to(message, Self::ID) {
            ListingAnswer::NotMine => Listing::waiting(),
            ListingAnswer::Refused(refusal) => Listing::Failed(ListingAnswer::reason(refusal)),
            ListingAnswer::Answered(data) => Listing::Listed(PiRpc::listed(data)),
        }
    }
}

/// A listing process's side of `pi --mode rpc`, with no session:
/// `get_available_models`, then `get_state` for the model Pi starts on.
#[derive(Debug)]
pub struct PiModels {
    /// The models listed, until the state names the default.
    listed: Option<Vec<Model>>,
}

impl PiModels {
    const MODELS: &'static str = "ren-models";
    /// The thinking levels every reasoning model takes; `xhigh` and `max`
    /// only some, which the listing does not say.
    const THINKING: [Effort; 5] = [
        Effort::Off,
        Effort::Minimal,
        Effort::Low,
        Effort::Medium,
        Effort::High,
    ];
    const STATE: &'static str = "ren-state";

    /// The arguments that put Pi in RPC mode with no session, after the
    /// extension arguments of a launched Pi.
    pub fn arguments() -> Vec<String> {
        vec!["--mode".into(), "rpc".into(), "--no-session".into()]
    }

    /// The codec, and the first line to send.
    pub fn open() -> (Self, Line) {
        let line = Line::json(&json!({ "id": Self::MODELS, "type": "get_available_models" }));
        (Self { listed: None }, line)
    }

    pub(crate) fn receive(&mut self, message: &Value) -> Listing {
        match &mut self.listed {
            None => {
                let data = match PiRpc::answer_to(message, Self::MODELS) {
                    ListingAnswer::NotMine => return Listing::waiting(),
                    ListingAnswer::Refused(refusal) => {
                        return Listing::Failed(ListingAnswer::reason(refusal));
                    }
                    ListingAnswer::Answered(data) => data,
                };
                let Some(entries) = data.get("models").and_then(Value::as_array) else {
                    return Listing::Failed("Pi listed no models".into());
                };
                self.listed = Some(entries.iter().filter_map(Self::model).collect());
                Listing::Waiting(vec![Line::json(
                    &json!({ "id": Self::STATE, "type": "get_state" }),
                )])
            }
            Some(listed) => {
                let state = match PiRpc::answer_to(message, Self::STATE) {
                    ListingAnswer::NotMine => return Listing::waiting(),
                    ListingAnswer::Answered(state) => Some(state),
                    // Without a readable state, nothing is marked.
                    ListingAnswer::Refused(_) => None,
                };
                let mut models = std::mem::take(listed);
                if let Some(state) = state
                    && let Some(current) = Self::model(&state["model"])
                    && let Some(model) = models.iter_mut().find(|model| model.id == current.id)
                {
                    model.choice = DefaultChoice::Default;
                    model.effort = ModelListing::text(state, "thinkingLevel")
                        .and_then(|level| level.parse().ok())
                        .filter(|level| model.efforts.contains(level));
                }
                Listing::Listed(models)
            }
        }
    }

    /// One of Pi's models, named `provider/id` as `--model` takes it.
    fn model(entry: &Value) -> Option<Model> {
        let provider = ModelListing::text(entry, "provider")?;
        let model = ModelListing::text(entry, "id")?;
        let id: ModelId = format!("{provider}/{model}").parse().ok()?;
        Some(Model {
            name: ModelListing::text(entry, "name").unwrap_or(model),
            description: None,
            provider: Some(provider),
            choice: DefaultChoice::Other,
            efforts: match entry.get("reasoning").and_then(Value::as_bool) {
                Some(true) => Self::THINKING.to_vec(),
                Some(false) | None => Vec::new(),
            },
            effort: None,
            id,
        })
    }
}
