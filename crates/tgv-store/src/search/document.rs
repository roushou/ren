//! What a call contributes to the index: its documents and the secrets
//! it carried.

use crate::activity::{ActivityBudget, Excerpt, ReadCall};
use serde::Serialize;
use tgv_harness::Harness;
pub use tgv_recorder::digest::SearchField;
use tgv_recorder::secrets::{Detection, SecretKind, SecretScanner};
use tgv_recorder::{
    id::{CallId, SessionId, TurnId},
    time::UnixMillis,
};

/// One searchable text of a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub field: SearchField,
    pub text: String,
}

/// A call's documents, with what search filters on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallDocuments {
    pub call: CallId,
    pub session: Option<SessionId>,
    pub turn: Option<TurnId>,
    pub client: Option<Harness>,
    pub started_at: UnixMillis,
    /// The start of the user's prompt, redacted: what a turn listing shows.
    pub prompt: Option<Excerpt>,
    pub documents: Vec<Document>,
    /// Secrets the call carried; never the secrets themselves.
    pub exposures: Vec<Exposure>,
}

/// A call's searchable texts, before redaction.
struct CallTexts {
    /// All of the newest user input, including context the client added:
    /// it is sent to the provider too, so it is scanned for secrets.
    user_input: String,
    /// The user's own words, which the prompt document keeps.
    prompt: Option<String>,
    reply: String,
    tool_input: String,
    tool_output: String,
}

impl ReadCall {
    fn texts(&self, harness: Option<Harness>) -> CallTexts {
        let tool_input = self
            .response
            .tool_uses
            .iter()
            .map(|tool| {
                let targets: Vec<String> = harness
                    .map(|harness| harness.effects(tool))
                    .unwrap_or_default()
                    .into_iter()
                    .map(|effect| effect.target)
                    .collect();
                let input = match &tool.input {
                    tgv_protocol::activity::ToolInput::Json(value) => value.to_string(),
                    tgv_protocol::activity::ToolInput::Text(text) => text.clone(),
                };
                format!("{} {} {}", tool.name, targets.join(" "), input)
            })
            .collect::<Vec<_>>()
            .join("\n");
        let tool_output = self
            .request
            .tool_results
            .iter()
            .map(|result| result.output.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        CallTexts {
            user_input: self.request.user_text.join("\n"),
            prompt: self.prompt(),
            reply: self.response.reply.clone(),
            tool_input,
            tool_output,
        }
    }

    /// The secrets the call carried, by field, without building its
    /// documents.
    pub fn exposures(&self, scanner: &SecretScanner) -> Vec<Exposure> {
        let texts = self.texts(self.harness());
        let mut exposures = Vec::new();
        for (field, text) in [
            (SearchField::Prompt, &texts.user_input),
            (SearchField::Reply, &texts.reply),
            (SearchField::ToolInput, &texts.tool_input),
            (SearchField::ToolOutput, &texts.tool_output),
        ] {
            Exposure::merge(&mut exposures, field, scanner.detect(text));
        }
        exposures
    }

    /// The call's documents, each cut to `max_chars` after its secrets are
    /// redacted, and the secrets found. The prompt's secrets are those of
    /// all the newest user input; its document keeps only the user's words.
    pub fn documents(&self, max_chars: usize, scanner: &SecretScanner) -> CallDocuments {
        let harness = self.harness();
        let texts = self.texts(harness);
        let mut exposures = Vec::new();
        Exposure::merge(
            &mut exposures,
            SearchField::Prompt,
            scanner.detect(&texts.user_input),
        );
        let mut documents = Vec::new();
        let mut add = |field: SearchField, redacted: &str| {
            if !redacted.trim().is_empty() {
                documents.push(Document {
                    field,
                    text: Excerpt::cut(redacted, max_chars).text,
                });
            }
        };
        let prompt = texts.prompt.as_ref().map(|prompt| scanner.redact(prompt).0);
        if let Some(prompt) = &prompt {
            add(SearchField::Prompt, prompt);
        }
        for (field, text) in [
            (SearchField::Reply, &texts.reply),
            (SearchField::ToolInput, &texts.tool_input),
            (SearchField::ToolOutput, &texts.tool_output),
        ] {
            let (redacted, found) = scanner.redact(text);
            Exposure::merge(&mut exposures, field, found);
            add(field, &redacted);
        }
        CallDocuments {
            call: self.view.id,
            session: self.view.session,
            turn: self.view.turn,
            client: harness,
            started_at: self.view.started_at,
            prompt: prompt
                .filter(|prompt| !prompt.trim().is_empty())
                .map(|prompt| Excerpt::cut(&prompt, ActivityBudget::SUMMARY.prompt_chars)),
            documents,
            exposures,
        }
    }
}

/// A secret a call carried, by field: what the scanner keeps of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Exposure {
    pub field: SearchField,
    pub kind: SecretKind,
    pub hint: String,
    pub fingerprint: String,
    /// How often it appears in the field.
    pub count: u64,
}

impl Exposure {
    /// Add `found` in `field` to `exposures`, counting repeats of one
    /// secret once per field.
    fn merge(exposures: &mut Vec<Self>, field: SearchField, found: Vec<Detection>) {
        for detection in found {
            match exposures.iter_mut().find(|known| {
                known.field == field
                    && known.kind == detection.kind
                    && known.fingerprint == detection.fingerprint
            }) {
                Some(known) => known.count = known.count.saturating_add(1),
                None => exposures.push(Self {
                    field,
                    kind: detection.kind,
                    hint: detection.hint,
                    fingerprint: detection.fingerprint,
                    count: 1,
                }),
            }
        }
    }
}
