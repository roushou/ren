//! Questions a driven client asks the operator mid-turn, and the operator's
//! replies: Claude Code's `AskUserQuestion` tool, Codex's
//! `item/tool/requestUserInput` request, and the dialogs a Pi extension
//! opens. Each codec reads its client's questions into [`Question`]s, which
//! an approval carries (`ApprovalAsk::Questions`), and writes the operator's
//! [`Replies`] back in its client's terms once they fit the questions
//! ([`FittedReplies`]).

use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::collections::BTreeMap;
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// What a client names one question by, so a reply finds it: Claude Code
/// names each by its text, Codex by its id, and Pi asks one at a time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QuestionKey(String);

impl QuestionKey {
    pub fn new(key: String) -> Self {
        Self(key)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for QuestionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// One question, as its client asked it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub key: QuestionKey,
    /// A short label for it (Claude Code's and Codex's `header`).
    pub header: Option<String>,
    pub text: String,
    /// A line more about it, where its client gave one.
    pub detail: Option<String>,
    pub form: QuestionForm,
}

/// How a question is answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum QuestionForm {
    /// Pick from options.
    Choice {
        options: Vec<QuestionOption>,
        pick: Pick,
        /// Whether the operator may answer in their own words instead.
        write_in: WriteIn,
    },
    /// Free text.
    Text {
        lines: TextLines,
        echo: Echo,
        placeholder: Option<String>,
        /// The text the answer starts from.
        prefill: Option<String>,
    },
    /// A number between two bounds.
    Number {
        min: serde_json::Number,
        max: serde_json::Number,
        step: Option<serde_json::Number>,
        /// The value the answer starts at.
        start: Option<serde_json::Number>,
        unit: Option<String>,
    },
    /// Yes or no.
    Confirm,
}

/// One option of a choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

/// How many options a choice takes.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Pick {
    One,
    Many,
}

/// Whether a choice takes an answer in the operator's own words.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum WriteIn {
    Allowed,
    Refused,
}

/// How much text an answer is.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum TextLines {
    One,
    Many,
}

/// Whether a text answer is shown as it is typed, or is a secret (Codex's
/// `isSecret`): a secret reply is never kept ([`SecretText`]).
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Echo {
    Shown,
    Hidden,
}

/// The operator's yes or no.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Confirmation {
    Yes,
    No,
}

/// The operator's replies to an ask's questions, by question. A question
/// with no reply is skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Replies(BTreeMap<QuestionKey, Reply>);

impl Replies {
    pub fn new(replies: BTreeMap<QuestionKey, Reply>) -> Self {
        Self(replies)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&QuestionKey, &Reply)> {
        self.0.iter()
    }

    /// These replies, once each fits its question in `questions`.
    pub fn fit(self, questions: &[Question]) -> Result<FittedReplies, ReplyRefused> {
        for (key, reply) in &self.0 {
            let question = questions
                .iter()
                .find(|question| question.key == *key)
                .ok_or_else(|| ReplyRefused::NoSuchQuestion(key.clone()))?;
            question.fits(reply)?;
        }
        Ok(FittedReplies(self))
    }
}

/// The operator's reply to one question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "as", rename_all = "snake_case")]
pub enum Reply {
    /// Options picked by label, with an answer in the operator's own words
    /// where the choice takes one.
    Chose {
        picked: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        written: Option<String>,
    },
    Wrote {
        text: String,
    },
    /// The answer to a question whose text is hidden.
    Secret {
        text: SecretText,
    },
    Number {
        value: serde_json::Number,
    },
    Confirmed {
        answer: Confirmation,
    },
    Skipped,
}

/// The text of a secret reply: sent to its client and kept nowhere. It is
/// written empty (so a journal that replays the reply never holds it) and
/// never shown by `Debug`.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct SecretText(String);

impl SecretText {
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The secret itself, for the line to its client.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl Serialize for SecretText {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("")
    }
}

impl std::fmt::Debug for SecretText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretText(hidden)")
    }
}

/// Replies that fit the questions they answer: made only by
/// [`Replies::fit`], so a codec writes them without checking again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FittedReplies(Replies);

impl FittedReplies {
    pub fn replies(&self) -> &Replies {
        &self.0
    }

    pub fn into_replies(self) -> Replies {
        self.0
    }

    /// The reply to `key`; none is a skip.
    pub(crate) fn to(&self, key: &QuestionKey) -> &Reply {
        self.0.0.get(key).unwrap_or(&Reply::Skipped)
    }
}

/// Why replies do not fit their questions.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplyRefused {
    #[error("no question {0} was asked")]
    NoSuchQuestion(QuestionKey),
    #[error("the reply to {0} does not answer that kind of question")]
    WrongForm(QuestionKey),
    #[error("{label:?} is not an option of {key}")]
    NoSuchOption { key: QuestionKey, label: String },
    #[error("{0} takes one option")]
    OneOption(QuestionKey),
    #[error("{0} takes no answer in the operator's own words")]
    NoWriteIn(QuestionKey),
    #[error("the reply to {0} picks nothing and says nothing; skip it instead")]
    Empty(QuestionKey),
    #[error("{0} takes a number between its bounds")]
    OutOfRange(QuestionKey),
}

impl Question {
    /// Whether `reply` answers this question.
    fn fits(&self, reply: &Reply) -> Result<(), ReplyRefused> {
        let key = || self.key.clone();
        match (&self.form, reply) {
            (_, Reply::Skipped) => Ok(()),
            (
                QuestionForm::Choice {
                    options,
                    pick,
                    write_in,
                },
                Reply::Chose { picked, written },
            ) => {
                if let Some(label) = picked
                    .iter()
                    .find(|label| !options.iter().any(|option| option.label == **label))
                {
                    return Err(ReplyRefused::NoSuchOption {
                        key: key(),
                        label: label.clone(),
                    });
                }
                let written = written.as_deref().map(str::trim).filter(|w| !w.is_empty());
                let chosen = picked.len() + usize::from(written.is_some());
                match (pick, write_in, written) {
                    (_, WriteIn::Refused, Some(_)) => Err(ReplyRefused::NoWriteIn(key())),
                    _ if chosen == 0 => Err(ReplyRefused::Empty(key())),
                    (Pick::One, _, _) if chosen > 1 => Err(ReplyRefused::OneOption(key())),
                    (Pick::One | Pick::Many, WriteIn::Allowed | WriteIn::Refused, _) => Ok(()),
                }
            }
            (
                QuestionForm::Text {
                    echo: Echo::Shown, ..
                },
                Reply::Wrote { .. },
            )
            | (
                QuestionForm::Text {
                    echo: Echo::Hidden, ..
                },
                Reply::Secret { .. },
            )
            | (QuestionForm::Confirm, Reply::Confirmed { .. }) => Ok(()),
            (QuestionForm::Number { min, max, .. }, Reply::Number { value }) => {
                let within = match (min.as_f64(), max.as_f64(), value.as_f64()) {
                    (Some(min), Some(max), Some(value)) => min <= value && value <= max,
                    (None, _, _) | (_, None, _) | (_, _, None) => false,
                };
                if within {
                    Ok(())
                } else {
                    Err(ReplyRefused::OutOfRange(key()))
                }
            }
            (
                QuestionForm::Choice { .. }
                | QuestionForm::Text { .. }
                | QuestionForm::Number { .. }
                | QuestionForm::Confirm,
                Reply::Chose { .. }
                | Reply::Wrote { .. }
                | Reply::Secret { .. }
                | Reply::Number { .. }
                | Reply::Confirmed { .. },
            ) => Err(ReplyRefused::WrongForm(key())),
        }
    }
}

impl Reply {
    /// The reply as one line of text, as Claude Code and Pi take it: picked
    /// labels and the operator's own words joined with ", "; none for a
    /// skip.
    pub(crate) fn text(&self) -> Option<String> {
        match self {
            Self::Chose { picked, written } => Some(
                picked
                    .iter()
                    .map(String::as_str)
                    .chain(written.as_deref().map(str::trim).filter(|w| !w.is_empty()))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Self::Wrote { text } => Some(text.clone()),
            Self::Secret { text } => Some(text.expose().to_owned()),
            Self::Number { value } => Some(value.to_string()),
            Self::Confirmed { answer } => Some(answer.to_string()),
            Self::Skipped => None,
        }
    }

    /// The reply as a list of answers, as Codex takes it: each picked label,
    /// then the operator's own words; empty for a skip.
    pub(crate) fn answers(&self) -> Vec<String> {
        match self {
            Self::Chose { picked, written } => picked
                .iter()
                .cloned()
                .chain(
                    written
                        .as_deref()
                        .map(str::trim)
                        .filter(|w| !w.is_empty())
                        .map(str::to_owned),
                )
                .collect(),
            Self::Wrote { .. }
            | Self::Secret { .. }
            | Self::Number { .. }
            | Self::Confirmed { .. }
            | Self::Skipped => self.text().into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Confirmation, Echo, Pick, Question, QuestionForm, QuestionKey, QuestionOption, Replies,
        Reply, ReplyRefused, SecretText, TextLines, WriteIn,
    };
    use std::collections::BTreeMap;

    fn key(text: &str) -> QuestionKey {
        QuestionKey::new(text.into())
    }

    fn choice(pick: Pick, write_in: WriteIn) -> Question {
        Question {
            key: key("lib"),
            header: Some("Library".into()),
            text: "Which library?".into(),
            detail: None,
            form: QuestionForm::Choice {
                options: ["chrono", "time"]
                    .map(|label| QuestionOption {
                        label: label.into(),
                        description: None,
                    })
                    .into(),
                pick,
                write_in,
            },
        }
    }

    fn text(echo: Echo) -> Question {
        Question {
            key: key("name"),
            header: None,
            text: "Its name?".into(),
            detail: None,
            form: QuestionForm::Text {
                lines: TextLines::One,
                echo,
                placeholder: None,
                prefill: None,
            },
        }
    }

    fn replies(pairs: Vec<(&str, Reply)>) -> Replies {
        Replies::new(
            pairs
                .into_iter()
                .map(|(question, reply)| (key(question), reply))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    fn chose(picked: &[&str], written: Option<&str>) -> Reply {
        Reply::Chose {
            picked: picked.iter().map(|label| (*label).to_owned()).collect(),
            written: written.map(str::to_owned),
        }
    }

    #[test]
    fn a_choice_takes_its_own_options_as_many_as_it_allows() {
        let one = [choice(Pick::One, WriteIn::Allowed)];
        assert!(
            replies(vec![("lib", chose(&["time"], None))])
                .fit(&one)
                .is_ok()
        );
        assert!(
            replies(vec![("lib", chose(&[], Some("jiff")))])
                .fit(&one)
                .is_ok()
        );
        assert_eq!(
            replies(vec![("lib", chose(&["time", "chrono"], None))]).fit(&one),
            Err(ReplyRefused::OneOption(key("lib")))
        );
        assert_eq!(
            replies(vec![("lib", chose(&["time"], Some("jiff")))]).fit(&one),
            Err(ReplyRefused::OneOption(key("lib")))
        );
        assert_eq!(
            replies(vec![("lib", chose(&["jiff"], None))]).fit(&one),
            Err(ReplyRefused::NoSuchOption {
                key: key("lib"),
                label: "jiff".into()
            })
        );
        assert_eq!(
            replies(vec![("lib", chose(&[], Some("  ")))]).fit(&one),
            Err(ReplyRefused::Empty(key("lib")))
        );
        let many = [choice(Pick::Many, WriteIn::Refused)];
        assert!(
            replies(vec![("lib", chose(&["time", "chrono"], None))])
                .fit(&many)
                .is_ok()
        );
        assert_eq!(
            replies(vec![("lib", chose(&["time"], Some("jiff")))]).fit(&many),
            Err(ReplyRefused::NoWriteIn(key("lib")))
        );
    }

    #[test]
    fn a_reply_answers_a_question_asked_in_its_form() {
        let questions = [
            choice(Pick::One, WriteIn::Allowed),
            text(Echo::Shown),
            Question {
                key: key("ok"),
                header: None,
                text: "Go on?".into(),
                detail: None,
                form: QuestionForm::Confirm,
            },
        ];
        let fitted = replies(vec![
            (
                "name",
                Reply::Wrote {
                    text: "acme".into(),
                },
            ),
            (
                "ok",
                Reply::Confirmed {
                    answer: Confirmation::No,
                },
            ),
        ])
        .fit(&questions)
        .unwrap();
        // A question with no reply is skipped.
        assert_eq!(fitted.to(&key("lib")), &Reply::Skipped);
        assert_eq!(
            replies(vec![("name", chose(&["time"], None))]).fit(&questions),
            Err(ReplyRefused::WrongForm(key("name")))
        );
        assert_eq!(
            replies(vec![("other", Reply::Skipped)]).fit(&questions),
            Err(ReplyRefused::NoSuchQuestion(key("other")))
        );
    }

    #[test]
    fn a_hidden_question_takes_only_a_secret_that_is_never_written() {
        let hidden = [text(Echo::Hidden)];
        assert_eq!(
            replies(vec![("name", Reply::Wrote { text: "x".into() })]).fit(&hidden),
            Err(ReplyRefused::WrongForm(key("name")))
        );
        let secret: Reply =
            serde_json::from_value(serde_json::json!({ "as": "secret", "text": "hunter2" }))
                .unwrap();
        assert_eq!(secret.text().as_deref(), Some("hunter2"));
        let fitted = replies(vec![("name", secret)]).fit(&hidden).unwrap();
        let written = serde_json::to_string(fitted.replies()).unwrap();
        assert!(!written.contains("hunter2"), "{written}");
        assert!(
            !format!("{:?}", SecretText("hunter2".into())).contains("hunter2"),
            "Debug shows no secret"
        );
    }

    #[test]
    fn a_number_stays_within_its_bounds() {
        let questions = [Question {
            key: key("slides"),
            header: None,
            text: "How many slides?".into(),
            detail: None,
            form: QuestionForm::Number {
                min: 1.into(),
                max: 10.into(),
                step: None,
                start: None,
                unit: None,
            },
        }];
        let number = |value: i64| Reply::Number {
            value: value.into(),
        };
        assert!(
            replies(vec![("slides", number(10))])
                .fit(&questions)
                .is_ok()
        );
        assert_eq!(
            replies(vec![("slides", number(11))]).fit(&questions),
            Err(ReplyRefused::OutOfRange(key("slides")))
        );
    }

    #[test]
    fn replies_read_as_their_clients_take_them() {
        assert_eq!(
            chose(&["time", "chrono"], Some(" jiff ")).text().as_deref(),
            Some("time, chrono, jiff")
        );
        assert_eq!(
            chose(&["time"], Some("jiff")).answers(),
            vec!["time".to_owned(), "jiff".to_owned()]
        );
        assert_eq!(
            Reply::Confirmed {
                answer: Confirmation::Yes
            }
            .text()
            .as_deref(),
            Some("yes")
        );
        assert_eq!(Reply::Skipped.text(), None);
        assert!(Reply::Skipped.answers().is_empty());
    }
}
