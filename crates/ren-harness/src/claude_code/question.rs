//! Claude Code's `AskUserQuestion` tool, as its permission prompt asks it and
//! as Ren answers it: the tool's input read as questions, and the operator's
//! replies written back as the input's `answers` (each question's text to
//! its answer), which Claude Code reads as the tool's result. Read against
//! the tool's schema in Claude Code 2.1.321, which also asks free-text and
//! number questions (`kind`).

use crate::question::{
    Echo, FittedReplies, Pick, Question, QuestionForm, QuestionKey, QuestionOption, TextLines,
    WriteIn,
};
use ren_protocol::label::Stored;
use serde::Deserialize;
use serde_json::{Number, Value};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// One call's input to `AskUserQuestion`, as far as Ren reads it.
#[derive(Debug, Deserialize)]
pub(super) struct AskUserQuestion {
    #[serde(default)]
    title: Option<String>,
    questions: Vec<AskedQuestion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AskedQuestion {
    question: String,
    #[serde(default)]
    header: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// How it is answered; a choice when left out.
    #[serde(default)]
    kind: Option<Stored<ClaudeQuestionKind>>,
    #[serde(default)]
    options: Vec<AskedOption>,
    #[serde(default)]
    multi_select: bool,
    #[serde(default)]
    placeholder: Option<String>,
    #[serde(default)]
    min: Option<Number>,
    #[serde(default)]
    max: Option<Number>,
    #[serde(default)]
    step: Option<Number>,
    #[serde(default)]
    default_value: Option<Number>,
    #[serde(default)]
    unit: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AskedOption {
    label: String,
    #[serde(default)]
    description: Option<String>,
}

/// How an `AskUserQuestion` question is answered (its `kind`).
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ClaudeQuestionKind {
    Choice,
    Text,
    Number,
}

impl AskUserQuestion {
    /// The questions in a call's `input`, with their heading; none when the
    /// input is not one Ren reads (a kind it does not know, a number
    /// question without bounds), so the call is asked as any other tool.
    pub(super) fn read(input: &Value) -> Option<(Option<String>, Vec<Question>)> {
        let asked = Self::deserialize(input).ok()?;
        let questions = asked
            .questions
            .into_iter()
            .map(AskedQuestion::read)
            .collect::<Option<Vec<_>>>()?;
        (!questions.is_empty()).then_some((asked.title, questions))
    }

    /// The tool's `input` with the operator's answers added: Claude Code
    /// runs the tool on it and reads each question's answer by its text. A
    /// skipped question has none.
    pub(super) fn answered(input: &Value, replies: &FittedReplies) -> Value {
        let answers = Self::read(input)
            .map(|(_, questions)| questions)
            .unwrap_or_default()
            .iter()
            .filter_map(|question| {
                replies
                    .to(&question.key)
                    .text()
                    .map(|text| (question.key.as_str().to_owned(), Value::String(text)))
            })
            .collect::<serde_json::Map<_, _>>();
        let mut input = input.clone();
        if let Value::Object(fields) = &mut input {
            fields.insert("answers".into(), Value::Object(answers));
        }
        input
    }

    /// The answers a finished call reports (`tool_use_result.answers`), by
    /// question.
    pub(super) fn answers(result: &Value) -> std::collections::BTreeMap<QuestionKey, String> {
        result
            .get("answers")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(question, answer)| {
                Some((
                    QuestionKey::new(question.clone()),
                    answer.as_str()?.to_owned(),
                ))
            })
            .collect()
    }
}

impl AskedQuestion {
    fn read(self) -> Option<Question> {
        let kind = match self.kind {
            None => ClaudeQuestionKind::Choice,
            Some(Stored::Known(kind)) => kind,
            Some(Stored::Unknown(_)) => return None,
        };
        let form = match kind {
            // Claude Code always offers an answer in the operator's own
            // words beside the options.
            ClaudeQuestionKind::Choice => QuestionForm::Choice {
                options: self
                    .options
                    .into_iter()
                    .map(|option| QuestionOption {
                        label: option.label,
                        description: option.description,
                    })
                    .collect(),
                pick: if self.multi_select {
                    Pick::Many
                } else {
                    Pick::One
                },
                write_in: WriteIn::Allowed,
            },
            ClaudeQuestionKind::Text => QuestionForm::Text {
                lines: TextLines::Many,
                echo: Echo::Shown,
                placeholder: self.placeholder,
                prefill: None,
            },
            ClaudeQuestionKind::Number => QuestionForm::Number {
                min: self.min?,
                max: self.max?,
                step: self.step,
                start: self.default_value,
                unit: self.unit,
            },
        };
        Some(Question {
            key: QuestionKey::new(self.question.clone()),
            header: self.header,
            text: self.question,
            detail: self.description,
            form,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::AskUserQuestion;
    use crate::question::{
        Echo, Pick, Question, QuestionForm, QuestionKey, QuestionOption, Replies, Reply, TextLines,
        WriteIn,
    };
    use serde_json::json;

    #[test]
    fn reads_choice_text_and_number_questions() {
        let input = json!({ "title": "Before I start", "questions": [
            { "question": "Which store?", "header": "Store", "multiSelect": true,
              "options": [{ "label": "SQLite", "description": "One file" }, { "label": "Postgres" }] },
            { "question": "What is it called?", "header": "Name", "kind": "text",
              "placeholder": "acme" },
            { "question": "How many workers?", "header": "Workers", "kind": "number",
              "min": 1, "max": 8, "unit": "workers" },
        ] });
        let (title, questions) = AskUserQuestion::read(&input).unwrap();
        assert_eq!(title.as_deref(), Some("Before I start"));
        assert_eq!(
            questions[0],
            Question {
                key: QuestionKey::new("Which store?".into()),
                header: Some("Store".into()),
                text: "Which store?".into(),
                detail: None,
                form: QuestionForm::Choice {
                    options: vec![
                        QuestionOption {
                            label: "SQLite".into(),
                            description: Some("One file".into())
                        },
                        QuestionOption {
                            label: "Postgres".into(),
                            description: None
                        },
                    ],
                    pick: Pick::Many,
                    write_in: WriteIn::Allowed,
                },
            }
        );
        assert_eq!(
            questions[1].form,
            QuestionForm::Text {
                lines: TextLines::Many,
                echo: Echo::Shown,
                placeholder: Some("acme".into()),
                prefill: None
            }
        );
        assert!(matches!(questions[2].form, QuestionForm::Number { .. }));
    }

    #[test]
    fn input_it_cannot_read_is_no_questions() {
        for input in [
            json!({ "questions": [] }),
            json!({ "questions": [{ "question": "Rate it", "kind": "stars" }] }),
            json!({ "questions": [{ "question": "How many?", "kind": "number" }] }),
            json!({ "question": "Which?" }),
        ] {
            assert_eq!(AskUserQuestion::read(&input), None, "{input}");
        }
    }

    #[test]
    fn answers_by_question_text_and_leaves_skipped_ones_out() {
        let input = json!({ "questions": [
            { "question": "Which store?", "header": "Store", "multiSelect": true,
              "options": [{ "label": "SQLite" }, { "label": "Postgres" }] },
            { "question": "What is it called?", "header": "Name", "kind": "text" },
        ] });
        let (_, questions) = AskUserQuestion::read(&input).unwrap();
        let replies = Replies::new(
            [(
                QuestionKey::new("Which store?".into()),
                Reply::Chose {
                    picked: vec!["SQLite".into(), "Postgres".into()],
                    written: Some("both".into()),
                },
            )]
            .into(),
        )
        .fit(&questions)
        .unwrap();
        let answered = AskUserQuestion::answered(&input, &replies);
        assert_eq!(answered["questions"], input["questions"]);
        assert_eq!(
            answered["answers"],
            json!({ "Which store?": "SQLite, Postgres, both" })
        );
    }

    #[test]
    fn reads_the_answers_a_result_reports() {
        let answers = AskUserQuestion::answers(&json!({
            "questions": [], "answers": { "Which store?": "SQLite" } }));
        assert_eq!(
            answers.into_iter().collect::<Vec<_>>(),
            vec![(QuestionKey::new("Which store?".into()), "SQLite".into())]
        );
    }
}
