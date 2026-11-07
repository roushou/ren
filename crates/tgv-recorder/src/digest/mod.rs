//! What a call's text becomes when it is stored: excerpts cut to a budget,
//! by field, after its secrets are redacted.

use crate::secrets::{Detection, SecretKind, SecretScanner};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use tgv_harness::tools::{InterpretedCall, ToolEffect};
use tgv_protocol::activity::{ActivityLimitation, ResultStatus, ToolInput};

/// Which part of a call a document holds.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum SearchField {
    /// The user's own words, without the context clients add.
    Prompt,
    /// The model's text output.
    Reply,
    /// Tool calls the model requested: name, targets, and input.
    ToolInput,
    /// Tool results the client sent back.
    ToolOutput,
}

/// Text cut to a budget, saying how much was left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Excerpt {
    pub text: String,
    pub omitted_chars: u64,
}

impl Excerpt {
    /// `text`, cut to `max_chars` characters.
    pub fn cut(text: &str, max_chars: usize) -> Self {
        let total = text.chars().count();
        Self {
            text: text.chars().take(max_chars).collect(),
            omitted_chars: u64::try_from(total.saturating_sub(max_chars)).unwrap_or(u64::MAX),
        }
    }

    pub fn of_input(input: &ToolInput, max_chars: usize) -> Self {
        match input {
            ToolInput::Json(value) => Self::cut(&value.to_string(), max_chars),
            ToolInput::Text(text) => Self::cut(text, max_chars),
        }
    }
}

/// How much of each text a digest keeps. Views cut further; the full text
/// stays in the stored body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigestBudget {
    pub prompt_chars: usize,
    pub reply_chars: usize,
    pub input_chars: usize,
    pub output_chars: usize,
    /// Per searchable field: every tool input together, or every output.
    pub search_chars: usize,
}

impl DigestBudget {
    pub const STORED: Self = Self {
        prompt_chars: 4_000,
        reply_chars: 4_000,
        input_chars: 2_000,
        output_chars: 2_000,
        search_chars: 32 * 1024,
    };
}

/// A secret a call carried, by field: what the scanner keeps of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn merge(exposures: &mut Vec<Self>, field: SearchField, found: Vec<Detection>) {
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

/// A tool call, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestTool {
    /// The identifier the client answers with.
    pub id: Option<String>,
    pub name: String,
    pub effects: Vec<ToolEffect>,
    pub input: Excerpt,
}

/// A tool result, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestResult {
    /// The tool call it answers.
    pub id: String,
    pub status: ResultStatus,
    pub output: Excerpt,
}

/// One searchable text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchText {
    pub field: SearchField,
    pub text: String,
}

/// What a call's bodies showed, as it is stored: every text redacted, then
/// cut to a [`DigestBudget`]. It never holds a secret, only what
/// [`Exposure`] keeps of one. Derived once, from the call's bytes; a newer
/// [`CallDigest::VERSION`] is derived again from the stored bodies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallDigest {
    pub version: u32,
    /// The user's own words, when the request starts with them.
    pub prompt: Option<Excerpt>,
    pub reply: Option<Excerpt>,
    pub tool_uses: Vec<DigestTool>,
    pub tool_results: Vec<DigestResult>,
    pub exposures: Vec<Exposure>,
    /// Each field's searchable text, when it has any.
    pub search: Vec<SearchText>,
    pub limitations: Vec<ActivityLimitation>,
}

impl CallDigest {
    pub const VERSION: u32 = 1;

    /// Redact every text once, keep what each secret leaves (its kind,
    /// hint, and fingerprint), and cut the rest to `budget`. The prompt's
    /// secrets are those of all the newest user input, client context
    /// included.
    pub fn seal(call: InterpretedCall, scanner: &SecretScanner, budget: DigestBudget) -> Self {
        let mut exposures = Vec::new();
        Exposure::merge(
            &mut exposures,
            SearchField::Prompt,
            scanner.detect(&call.user_text.join("\n")),
        );
        let prompt = call
            .prompt
            .map(|prompt| scanner.redact(&prompt).0)
            .filter(|prompt| !prompt.trim().is_empty());
        let (reply, found) = scanner.redact(&call.reply);
        Exposure::merge(&mut exposures, SearchField::Reply, found);
        let mut tool_input = Vec::with_capacity(call.tool_uses.len());
        let mut found_inputs = Vec::new();
        let tool_uses = call
            .tool_uses
            .into_iter()
            .map(|used| {
                let text = match &used.tool.input {
                    ToolInput::Json(value) => value.to_string(),
                    ToolInput::Text(text) => text.clone(),
                };
                let (input, mut found) = scanner.redact(&text);
                found_inputs.append(&mut found);
                // A target repeats part of the input: redacted, not counted.
                let effects: Vec<ToolEffect> = used
                    .effects
                    .into_iter()
                    .map(|effect| ToolEffect {
                        target: scanner.redact(&effect.target).0,
                        ..effect
                    })
                    .collect();
                let targets: Vec<&str> = effects
                    .iter()
                    .map(|effect| effect.target.as_str())
                    .collect();
                tool_input.push(format!(
                    "{} {} {}",
                    used.tool.name,
                    targets.join(" "),
                    input
                ));
                DigestTool {
                    id: used.tool.id,
                    name: used.tool.name,
                    input: Excerpt::cut(&input, budget.input_chars),
                    effects,
                }
            })
            .collect();
        Exposure::merge(&mut exposures, SearchField::ToolInput, found_inputs);
        let mut tool_output = Vec::with_capacity(call.tool_results.len());
        let mut found_outputs = Vec::new();
        let tool_results = call
            .tool_results
            .into_iter()
            .map(|result| {
                let (output, mut found) = scanner.redact(&result.output);
                found_outputs.append(&mut found);
                let excerpt = Excerpt::cut(&output, budget.output_chars);
                tool_output.push(output);
                DigestResult {
                    id: result.id,
                    status: result.status,
                    output: excerpt,
                }
            })
            .collect();
        Exposure::merge(&mut exposures, SearchField::ToolOutput, found_outputs);
        let search = [
            (SearchField::Prompt, prompt.clone().unwrap_or_default()),
            (SearchField::Reply, reply.clone()),
            (SearchField::ToolInput, tool_input.join("\n")),
            (SearchField::ToolOutput, tool_output.join("\n")),
        ]
        .into_iter()
        .filter(|(_, text)| !text.trim().is_empty())
        .map(|(field, text)| SearchText {
            field,
            text: Excerpt::cut(&text, budget.search_chars).text,
        })
        .collect();
        Self {
            version: Self::VERSION,
            prompt: prompt.map(|prompt| Excerpt::cut(&prompt, budget.prompt_chars)),
            reply: (!reply.trim().is_empty()).then(|| Excerpt::cut(&reply, budget.reply_chars)),
            tool_uses,
            tool_results,
            exposures,
            search,
            limitations: call.limitations,
        }
    }
}

#[cfg(test)]
mod tests;
