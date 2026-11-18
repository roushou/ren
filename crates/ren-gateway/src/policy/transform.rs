//! Request transform steps and the paths they may never touch.
//!
//! A body step changes the request's JSON; the body is re-serialized only when
//! a step actually changed it, so an untouched request stays byte for byte.
//! Object key order is preserved. Protected headers and paths are refused when
//! the configuration loads, never at request time.

use http::{HeaderMap, HeaderName, HeaderValue};
use ren_protocol::route::Endpoint;
use serde_json::Value;

/// One change to a request, parsed once when the configuration loads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "StepConfig", into = "StepConfig")]
pub enum TransformStep {
    SetHeader {
        name: HeaderName,
        value: HeaderValue,
    },
    RemoveHeader {
        name: HeaderName,
    },
    /// Set the value at a JSON Pointer. The parent must exist.
    JsonSet {
        path: JsonPointer,
        value: Value,
    },
    JsonRemove {
        path: JsonPointer,
    },
}

/// A step as the configuration writes it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "op", deny_unknown_fields)]
enum StepConfig {
    SetHeader { name: String, value: String },
    RemoveHeader { name: String },
    JsonSet { path: String, value: Value },
    JsonRemove { path: String },
}

impl TryFrom<StepConfig> for TransformStep {
    type Error = StepError;

    fn try_from(step: StepConfig) -> Result<Self, StepError> {
        let header = |name: &str| {
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| StepError::InvalidHeader)
        };
        Ok(match step {
            StepConfig::SetHeader { name, value } => Self::SetHeader {
                name: header(&name)?,
                value: HeaderValue::from_str(&value).map_err(|_| StepError::InvalidHeader)?,
            },
            StepConfig::RemoveHeader { name } => Self::RemoveHeader {
                name: header(&name)?,
            },
            StepConfig::JsonSet { path, value } => Self::JsonSet {
                path: path.parse()?,
                value,
            },
            StepConfig::JsonRemove { path } => Self::JsonRemove {
                path: path.parse()?,
            },
        })
    }
}

impl From<TransformStep> for StepConfig {
    fn from(step: TransformStep) -> Self {
        match step {
            TransformStep::SetHeader { name, value } => Self::SetHeader {
                name: name.as_str().to_owned(),
                // Parsed from a `str`, so its bytes are UTF-8.
                value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
            },
            TransformStep::RemoveHeader { name } => Self::RemoveHeader {
                name: name.as_str().to_owned(),
            },
            TransformStep::JsonSet { path, value } => Self::JsonSet {
                path: path.text,
                value,
            },
            TransformStep::JsonRemove { path } => Self::JsonRemove { path: path.text },
        }
    }
}

/// A JSON Pointer (RFC 6901) a body step targets, split once into its
/// parent pointer and its last key, unescaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonPointer {
    /// As written.
    text: String,
    /// The parent, still escaped, as `Value::pointer_mut` takes it.
    parent_end: usize,
    key: String,
}

impl std::str::FromStr for JsonPointer {
    type Err = StepError;

    fn from_str(text: &str) -> Result<Self, StepError> {
        let Some(parent_end) = text.rfind('/') else {
            return Err(StepError::InvalidPath);
        };
        if !text.starts_with('/') {
            return Err(StepError::InvalidPath);
        }
        Ok(Self {
            key: text[parent_end + 1..].replace("~1", "/").replace("~0", "~"),
            parent_end,
            text: text.to_owned(),
        })
    }
}

impl JsonPointer {
    pub fn as_str(&self) -> &str {
        &self.text
    }

    fn parent(&self) -> &str {
        &self.text[..self.parent_end]
    }

    /// Whether this pointer is `protected` or below it.
    fn within(&self, protected: &str) -> bool {
        self.text
            .strip_prefix(protected)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }
}

/// Why a step cannot be configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StepError {
    #[error("a header name or value is not valid HTTP")]
    InvalidHeader,
    #[error(
        "a protected header (credentials, client identity, framing, or a beta pair) cannot be changed"
    )]
    ProtectedHeader,
    #[error("a path must be a non-empty JSON Pointer starting with '/'")]
    InvalidPath,
    #[error("a protected body path cannot be changed")]
    ProtectedPath,
}

/// What happened when a step ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Applied,
    /// The request already had the step's result.
    Unchanged,
    /// The body is not JSON, so a body step could not run.
    NotJson,
    /// The path's parent does not exist.
    NoParent,
    /// The body is compressed; body steps do not run on it.
    Encoded,
}

/// One step's record: which rule, which step, and what happened. Header
/// values are never recorded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StepRecord {
    pub rule: String,
    pub step: usize,
    pub op: &'static str,
    /// The header name or JSON Pointer the step targets.
    pub target: String,
    pub outcome: StepOutcome,
    pub reason: String,
}

/// Headers no step may set or remove, for any endpoint.
const PROTECTED_HEADERS: &[&str] = &[
    // Credentials: a transform must never add, change, or drop them.
    "authorization",
    "x-api-key",
    "cookie",
    "chatgpt-account-id",
    // Beta features pair a header with body fields; changing one half
    // breaks the pair, and the OAuth beta is part of subscription access.
    "anthropic-beta",
    "openai-beta",
    // Client identity is never spoofed.
    "user-agent",
    "originator",
    "x-app",
    // Framing is the gateway's.
    "host",
    "content-length",
    "content-encoding",
    "transfer-encoding",
];

impl TransformStep {
    /// Body paths no step may touch on `endpoint`, with everything below them.
    fn protected_paths(endpoint: Endpoint) -> &'static [&'static str] {
        match endpoint {
            // The system blocks are position-sensitive; metadata carries the
            // client's identity.
            Endpoint::Messages | Endpoint::CountTokens => &["/system", "/metadata", "/betas"],
            // Continuation state lives on the provider; client metadata is identity.
            Endpoint::Responses | Endpoint::ResponsesCompact => {
                &["/previous_response_id", "/client_metadata"]
            }
            // The end-user identifiers a provider attributes abuse to, and
            // the client's stored-completion metadata.
            Endpoint::ChatCompletions => &["/user", "/safety_identifier", "/metadata"],
            Endpoint::Models => &[],
        }
    }

    /// Refuse a step that would touch a protected header, or a body path
    /// protected on any of `endpoints`.
    pub(super) fn check(&self, endpoints: &[Endpoint]) -> Result<(), StepError> {
        match self {
            Self::SetHeader { name, .. } | Self::RemoveHeader { name } => {
                Self::unprotected_header(name)
            }
            Self::JsonSet { path, .. } | Self::JsonRemove { path } => {
                let protected = endpoints.iter().any(|endpoint| {
                    Self::protected_paths(*endpoint)
                        .iter()
                        .any(|protected| path.within(protected))
                });
                if protected {
                    Err(StepError::ProtectedPath)
                } else {
                    Ok(())
                }
            }
        }
    }

    fn unprotected_header(name: &HeaderName) -> Result<(), StepError> {
        if PROTECTED_HEADERS.contains(&name.as_str()) || name.as_str().starts_with("x-ren-") {
            Err(StepError::ProtectedHeader)
        } else {
            Ok(())
        }
    }

    fn op(&self) -> &'static str {
        match self {
            Self::SetHeader { .. } => "set_header",
            Self::RemoveHeader { .. } => "remove_header",
            Self::JsonSet { .. } => "json_set",
            Self::JsonRemove { .. } => "json_remove",
        }
    }

    fn target(&self) -> &str {
        match self {
            Self::SetHeader { name, .. } | Self::RemoveHeader { name } => name.as_str(),
            Self::JsonSet { path, .. } | Self::JsonRemove { path } => path.as_str(),
        }
    }

    /// Run the step. A parsed step cannot fail; an unusable body is an
    /// outcome, not an error.
    pub fn apply(&self, headers: &mut HeaderMap, body: &mut RequestJson) -> StepOutcome {
        match self {
            Self::SetHeader { name, value } => {
                if headers.get(name) == Some(value) && headers.get_all(name).iter().count() == 1 {
                    StepOutcome::Unchanged
                } else {
                    headers.insert(name.clone(), value.clone());
                    StepOutcome::Applied
                }
            }
            Self::RemoveHeader { name } => {
                if headers.remove(name).is_some() {
                    StepOutcome::Applied
                } else {
                    StepOutcome::Unchanged
                }
            }
            Self::JsonSet { path, value } => body.edit(|document| {
                let key = &path.key;
                match document.pointer_mut(path.parent()) {
                    Some(Value::Object(map)) => {
                        if map.get(key) == Some(value) {
                            StepOutcome::Unchanged
                        } else {
                            map.insert(key.clone(), value.clone());
                            StepOutcome::Applied
                        }
                    }
                    Some(Value::Array(items)) => match key.parse::<usize>() {
                        Ok(index) if index < items.len() => {
                            if items.get(index) == Some(value) {
                                StepOutcome::Unchanged
                            } else if let Some(slot) = items.get_mut(index) {
                                *slot = value.clone();
                                StepOutcome::Applied
                            } else {
                                StepOutcome::NoParent
                            }
                        }
                        Ok(_) | Err(_) => StepOutcome::NoParent,
                    },
                    Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))
                    | None => StepOutcome::NoParent,
                }
            }),
            Self::JsonRemove { path } => body.edit(|document| {
                let key = &path.key;
                match document.pointer_mut(path.parent()) {
                    Some(Value::Object(map)) => {
                        if map.shift_remove(key).is_some() {
                            StepOutcome::Applied
                        } else {
                            StepOutcome::Unchanged
                        }
                    }
                    Some(Value::Array(items)) => match key.parse::<usize>() {
                        Ok(index) if index < items.len() => {
                            items.remove(index);
                            StepOutcome::Applied
                        }
                        Ok(_) | Err(_) => StepOutcome::Unchanged,
                    },
                    Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => {
                        StepOutcome::Unchanged
                    }
                    None => StepOutcome::NoParent,
                }
            }),
        }
    }

    pub fn record(
        &self,
        rule: &super::TransformRule,
        step: usize,
        outcome: StepOutcome,
    ) -> StepRecord {
        StepRecord {
            rule: rule.id.as_str().to_owned(),
            step,
            op: self.op(),
            target: self.target().to_owned(),
            outcome,
            reason: rule.reason.clone(),
        }
    }
}

/// A request body as JSON, parsed only if a body step runs.
#[derive(Debug)]
pub struct RequestJson<'a> {
    original: &'a [u8],
    state: Parsed,
}

#[derive(Debug)]
enum Parsed {
    NotYet,
    Json { document: Value, changed: bool },
    NotJson,
    Encoded,
}

impl<'a> RequestJson<'a> {
    pub fn new(original: &'a [u8]) -> Self {
        Self {
            original,
            state: Parsed::NotYet,
        }
    }

    /// A compressed body: body steps report [`StepOutcome::Encoded`].
    pub fn encoded() -> Self {
        Self {
            original: &[],
            state: Parsed::Encoded,
        }
    }

    fn edit(&mut self, change: impl FnOnce(&mut Value) -> StepOutcome) -> StepOutcome {
        if let Parsed::NotYet = self.state {
            self.state = match serde_json::from_slice(self.original) {
                Ok(document) => Parsed::Json {
                    document,
                    changed: false,
                },
                Err(_) => Parsed::NotJson,
            };
        }
        match &mut self.state {
            Parsed::NotYet | Parsed::NotJson => StepOutcome::NotJson,
            Parsed::Encoded => StepOutcome::Encoded,
            Parsed::Json { document, changed } => {
                let outcome = change(document);
                *changed |= outcome == StepOutcome::Applied;
                outcome
            }
        }
    }

    /// The new body, if a step changed it.
    pub fn changed(self) -> Option<Vec<u8>> {
        match self.state {
            Parsed::Json {
                document,
                changed: true,
            } => serde_json::to_vec(&document).ok(),
            Parsed::Json { changed: false, .. }
            | Parsed::NotYet
            | Parsed::NotJson
            | Parsed::Encoded => None,
        }
    }
}
