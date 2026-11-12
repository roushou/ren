//! The models a driven client offers, as it lists them itself. A listing is
//! its own short process: the client started in its structured mode with no
//! conversation, asked for its models, and stopped. Each client's listing is
//! a pure codec beside its conversation codec.

use super::{Effort, Framed, Line};
use crate::claude_code::ClaudeModels;
use crate::codex::CodexModels;
use crate::pi::PiModels;
use crate::{Harness, launch::NotDrivable};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// A model, named as its client names it on the command line or in its
/// protocol: Codex's model slug, Claude Code's `--model` value, Pi's
/// `provider/id`. Never empty.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr,
)]
pub struct ModelId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a model identifier cannot be blank")]
pub struct BlankModelId;

impl std::str::FromStr for ModelId {
    type Err = BlankModelId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            Err(BlankModelId)
        } else {
            Ok(Self(value.into()))
        }
    }
}

impl std::fmt::Display for ModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl ModelId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One model a client offers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Model {
    pub id: ModelId,
    /// The name the client shows for it.
    pub name: String,
    pub description: Option<String>,
    /// The provider serving it, where the client offers several (Pi).
    pub provider: Option<String>,
    /// Which model a run gets when none is chosen.
    pub choice: DefaultChoice,
    /// The efforts it offers, least first; empty when it offers no choice.
    pub efforts: Vec<Effort>,
    /// The effort it runs at when none is chosen, when the client says.
    pub effort: Option<Effort>,
}

/// Whether a model is the one a client uses when none is chosen.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum DefaultChoice {
    Default,
    Other,
}

/// Where a listing stands after one line from the client.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum Listing {
    /// Still waiting; write these lines to the client's stdin first.
    Waiting(Vec<Line>),
    /// The client's models, in the order it lists them.
    Listed(Vec<Model>),
    /// The client refused the request, or answered with something Ren could
    /// not read.
    Failed(String),
}

impl Listing {
    /// Nothing to send yet.
    pub(crate) fn waiting() -> Self {
        Self::Waiting(Vec::new())
    }
}

/// One listing process's codec, chosen by its harness.
#[derive(Debug)]
pub enum ModelListing {
    Codex(CodexModels),
    ClaudeCode(ClaudeModels),
    Pi(PiModels),
}

impl ModelListing {
    /// The codec for a freshly started listing process of `harness`, and the
    /// lines to send it first. `version` is Ren's own version.
    pub fn open(harness: Harness, version: &str) -> Result<(Self, Vec<Line>), NotDrivable> {
        match harness {
            Harness::Codex => {
                let (codec, first) = CodexModels::open(version);
                Ok((Self::Codex(codec), vec![first]))
            }
            Harness::ClaudeCode => {
                let (codec, first) = ClaudeModels::open();
                Ok((Self::ClaudeCode(codec), vec![first]))
            }
            Harness::Pi => {
                let (codec, first) = PiModels::open();
                Ok((Self::Pi(codec), vec![first]))
            }
            Harness::OpenCode => Err(NotDrivable(harness)),
        }
    }

    /// Read one frame from the client's stdout.
    pub fn receive(&mut self, frame: Framed) -> Listing {
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
        }
    }
}

impl ModelListing {
    /// The efforts named in `levels` (strings, or objects naming theirs in
    /// `field`), least first; levels Ren does not know are left out, since a
    /// run could not ask for them.
    pub(crate) fn efforts(levels: Option<&serde_json::Value>, field: Option<&str>) -> Vec<Effort> {
        let mut efforts: Vec<Effort> = levels
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|level| match field {
                Some(field) => level.get(field),
                None => Some(level),
            })
            .filter_map(serde_json::Value::as_str)
            .filter_map(|level| level.parse().ok())
            .collect();
        efforts.sort();
        efforts.dedup();
        efforts
    }

    /// The text of `field` in `value`, when it is a non-blank string.
    pub(crate) fn text(value: &serde_json::Value, field: &str) -> Option<String> {
        value
            .get(field)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::{DefaultChoice, Listing, Model, ModelId, ModelListing};
    use crate::claude_code::ClaudeStreamJson;
    use crate::codex::CodexAppServer;
    use crate::drive::{ApprovalPolicy, Effort, RunSettings};
    use crate::pi::PiRpc;
    use crate::{
        Harness,
        drive::{Framed, Line, Opening},
    };
    use serde_json::{Value, json};

    fn sent(line: &Line) -> Value {
        serde_json::from_slice(line.as_bytes()).unwrap()
    }

    fn feed(listing: &mut ModelListing, message: Value) -> Listing {
        listing.receive(Framed::Line(message.to_string().into_bytes()))
    }

    fn model(id: &str, name: &str, provider: Option<&str>, choice: DefaultChoice) -> Model {
        Model {
            id: id.parse().unwrap(),
            name: name.into(),
            description: None,
            provider: provider.map(str::to_owned),
            choice,
            efforts: Vec::new(),
            effort: None,
        }
    }

    #[test]
    fn a_model_id_is_never_blank() {
        assert!(" ".parse::<ModelId>().is_err());
        assert_eq!(" gpt ".parse::<ModelId>().unwrap().as_str(), "gpt");
    }

    #[test]
    fn codex_lists_its_visible_models_page_by_page() {
        let (mut listing, first) = ModelListing::open(Harness::Codex, "0.1.0").unwrap();
        assert_eq!(sent(&first[0])["method"], "initialize");
        let Listing::Waiting(lines) = feed(&mut listing, json!({ "id": 0, "result": {} })) else {
            panic!("waiting for the list");
        };
        let lines: Vec<Value> = lines.iter().map(sent).collect();
        assert_eq!(
            lines,
            vec![
                json!({ "method": "initialized" }),
                json!({ "id": 1, "method": "model/list",
                    "params": { "includeHidden": false, "cursor": null } }),
            ]
        );
        // A notification in between is not the answer.
        assert_eq!(
            feed(&mut listing, json!({ "method": "thread/started" })),
            Listing::Waiting(Vec::new())
        );
        let Listing::Waiting(next) = feed(
            &mut listing,
            json!({ "id": 1, "result": { "nextCursor": "p2", "data": [
                { "id": "x", "model": "gpt-fast", "displayName": "GPT Fast",
                  "description": "Quick.", "isDefault": true, "hidden": false },
                { "id": "h", "model": "gpt-hidden", "displayName": "Hidden",
                  "description": "", "isDefault": false, "hidden": true },
            ] } }),
        ) else {
            panic!("waiting for the next page");
        };
        assert_eq!(sent(&next[0])["params"]["cursor"], "p2");
        let listed = feed(
            &mut listing,
            json!({ "id": 2, "result": { "data": [
                { "id": "d", "model": "gpt-deep", "displayName": "GPT Deep",
                  "description": " ", "isDefault": false, "hidden": false },
            ] } }),
        );
        let mut fast = model("gpt-fast", "GPT Fast", None, DefaultChoice::Default);
        fast.description = Some("Quick.".into());
        assert_eq!(
            listed,
            Listing::Listed(vec![
                fast,
                model("gpt-deep", "GPT Deep", None, DefaultChoice::Other),
            ])
        );
    }

    #[test]
    fn codex_refusing_the_list_fails_it() {
        let (mut listing, _) = ModelListing::open(Harness::Codex, "0.1.0").unwrap();
        let _ = feed(&mut listing, json!({ "id": 0, "result": {} }));
        assert_eq!(
            feed(
                &mut listing,
                json!({ "id": 1, "error": { "code": -1, "message": "not signed in" } })
            ),
            Listing::Failed("not signed in".into())
        );
    }

    #[test]
    fn claude_code_lists_the_models_its_initialize_answer_names() {
        let (mut listing, first) = ModelListing::open(Harness::ClaudeCode, "0.1.0").unwrap();
        assert_eq!(sent(&first[0])["request"]["subtype"], "initialize");
        let id = sent(&first[0])["request_id"].clone();
        assert_eq!(
            feed(&mut listing, json!({ "type": "system", "subtype": "init" })),
            Listing::Waiting(Vec::new())
        );
        let listed = feed(
            &mut listing,
            json!({ "type": "control_response", "response": {
                "subtype": "success", "request_id": id, "response": { "models": [
                    { "value": "default", "resolvedModel": "claude-big-1",
                      "displayName": "Default (recommended)" },
                    { "value": "big", "resolvedModel": "claude-big-1",
                      "displayName": "Big", "description": "For hard tasks" },
                    { "value": "claude-big-1", "resolvedModel": "claude-big-1",
                      "displayName": "Big 1" },
                    { "value": "small", "resolvedModel": "claude-small-1",
                      "displayName": "Small" },
                ] } } }),
        );
        let mut big = model("big", "Big", None, DefaultChoice::Default);
        big.description = Some("For hard tasks".into());
        assert_eq!(
            listed,
            Listing::Listed(vec![
                big,
                model("claude-big-1", "Big 1", None, DefaultChoice::Other),
                model("small", "Small", None, DefaultChoice::Other),
            ])
        );
    }

    #[test]
    fn pi_lists_models_by_provider_and_marks_the_one_it_starts_on() {
        let (mut listing, first) = ModelListing::open(Harness::Pi, "0.1.0").unwrap();
        assert_eq!(sent(&first[0])["type"], "get_available_models");
        let Listing::Waiting(state) = feed(
            &mut listing,
            json!({ "id": "ren-models", "type": "response", "command": "get_available_models",
                "success": true, "data": { "models": [
                    { "provider": "anthropic", "id": "claude-big-1", "name": "Claude Big" },
                    { "provider": "openai", "id": "gpt-fast", "name": "GPT Fast" },
                ] } }),
        ) else {
            panic!("waiting for the state");
        };
        assert_eq!(sent(&state[0])["type"], "get_state");
        let listed = feed(
            &mut listing,
            json!({ "id": "ren-state", "type": "response", "command": "get_state",
                "success": true, "data": { "model": { "provider": "openai", "id": "gpt-fast" } } }),
        );
        assert_eq!(
            listed,
            Listing::Listed(vec![
                model(
                    "anthropic/claude-big-1",
                    "Claude Big",
                    Some("anthropic"),
                    DefaultChoice::Other
                ),
                model(
                    "openai/gpt-fast",
                    "GPT Fast",
                    Some("openai"),
                    DefaultChoice::Default
                ),
            ])
        );
    }

    #[test]
    fn opencode_cannot_list() {
        assert!(ModelListing::open(Harness::OpenCode, "0.1.0").is_err());
    }

    fn opening(model: Option<&str>) -> Opening {
        opened(RunSettings {
            model: model.map(|model| model.parse().unwrap()),
            ..RunSettings::default()
        })
    }

    fn opened(settings: RunSettings) -> Opening {
        Opening::New {
            cwd: "/work/acme".into(),
            id: ren_protocol::native::NativeThreadId::new(uuid::Uuid::from_u128(9)).unwrap(),
            settings,
        }
    }

    /// Effort and approval policy go where each client takes them.
    #[test]
    fn a_conversation_opens_with_its_effort_and_approval_policy() {
        let settings = |effort, approvals| RunSettings {
            model: None,
            effort: Some(effort),
            approvals: Some(approvals),
        };
        let claude =
            ClaudeStreamJson::arguments(&opened(settings(Effort::High, ApprovalPolicy::Edits)));
        assert_eq!(
            claude[claude.len() - 4..],
            ["--effort", "high", "--permission-mode", "acceptEdits"]
        );
        let pi = PiRpc::arguments(&opened(settings(Effort::Minimal, ApprovalPolicy::Auto)));
        assert_eq!(pi[pi.len() - 2..], ["--thinking", "minimal"]);

        let (mut codex, _) = CodexAppServer::open(
            opened(settings(Effort::Xhigh, ApprovalPolicy::Ask)),
            "0.1.0",
        );
        let received = codex.receive(Framed::Line(
            json!({ "id": 0, "result": {} }).to_string().into_bytes(),
        ));
        assert_eq!(
            sent(&received.replies[1])["params"],
            json!({ "cwd": "/work/acme", "approvalPolicy": "untrusted", "sandbox": "read-only" })
        );
        let thread = "0190f3a0-0000-7000-8000-000000000001";
        let _ = codex.receive(Framed::Line(
            json!({ "id": 1, "result": { "thread": { "id": thread } } })
                .to_string()
                .into_bytes(),
        ));
        let turn = codex
            .command(crate::drive::DriveCommand::Prompt(
                crate::drive::Prompt::parse("go".into()).unwrap(),
            ))
            .unwrap();
        assert_eq!(sent(&turn)["params"]["effort"], "xhigh");
        assert!(ApprovalPolicy::offered(Harness::Pi).is_empty());
    }

    #[test]
    fn listed_models_say_which_efforts_they_take() {
        let (mut listing, _) = ModelListing::open(Harness::Codex, "0.1.0").unwrap();
        let _ = feed(&mut listing, json!({ "id": 0, "result": {} }));
        let Listing::Listed(codex) = feed(
            &mut listing,
            json!({ "id": 1, "result": { "data": [
                { "model": "gpt-fast", "displayName": "GPT Fast", "isDefault": true,
                  "defaultReasoningEffort": "medium", "supportedReasoningEfforts": [
                    { "reasoningEffort": "high", "description": "" },
                    { "reasoningEffort": "low", "description": "" },
                    { "reasoningEffort": "ludicrous", "description": "" },
                    { "reasoningEffort": "medium", "description": "" } ] },
            ] } }),
        ) else {
            panic!("listed");
        };
        assert_eq!(
            codex[0].efforts,
            [Effort::Low, Effort::Medium, Effort::High]
        );
        assert_eq!(codex[0].effort, Some(Effort::Medium));

        let (mut listing, first) = ModelListing::open(Harness::ClaudeCode, "0.1.0").unwrap();
        let Listing::Listed(claude) = feed(
            &mut listing,
            json!({ "type": "control_response", "response": {
                "subtype": "success", "request_id": sent(&first[0])["request_id"], "response": {
                "models": [
                    { "value": "big", "displayName": "Big", "supportsEffort": true,
                      "supportedEffortLevels": ["low", "medium", "high", "xhigh", "max"] },
                    { "value": "old", "displayName": "Old" },
                ] } } }),
        ) else {
            panic!("listed");
        };
        assert_eq!(claude[0].efforts.len(), 5);
        assert!(claude[1].efforts.is_empty());

        let (mut listing, _) = ModelListing::open(Harness::Pi, "0.1.0").unwrap();
        let _ = feed(
            &mut listing,
            json!({ "id": "ren-models", "type": "response", "success": true, "data": { "models": [
                { "provider": "a", "id": "thinks", "name": "Thinks", "reasoning": true },
                { "provider": "a", "id": "plain", "name": "Plain", "reasoning": false },
            ] } }),
        );
        let Listing::Listed(pi) = feed(
            &mut listing,
            json!({ "id": "ren-state", "type": "response", "success": true, "data": {
                "model": { "provider": "a", "id": "thinks" }, "thinkingLevel": "low" } }),
        ) else {
            panic!("listed");
        };
        assert_eq!(pi[0].efforts.first(), Some(&Effort::Off));
        assert_eq!(pi[0].effort, Some(Effort::Low));
        assert!(pi[1].efforts.is_empty());
    }

    /// A chosen model goes where each client takes it; none leaves the
    /// client on its own default.
    #[test]
    fn a_conversation_opens_on_its_chosen_model() {
        let claude = ClaudeStreamJson::arguments(&opening(Some("small")));
        assert_eq!(claude[claude.len() - 2..], ["--model", "small"]);
        assert!(!ClaudeStreamJson::arguments(&opening(None)).contains(&"--model".to_owned()));

        let pi = PiRpc::arguments(&opening(Some("openai/gpt-fast")));
        assert_eq!(pi[pi.len() - 2..], ["--model", "openai/gpt-fast"]);

        let (mut codex, _) = CodexAppServer::open(opening(Some("gpt-deep")), "0.1.0");
        let received = codex.receive(Framed::Line(
            json!({ "id": 0, "result": {} }).to_string().into_bytes(),
        ));
        assert_eq!(
            sent(&received.replies[1])["params"],
            json!({ "cwd": "/work/acme", "model": "gpt-deep" })
        );
    }
}
