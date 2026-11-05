//! Request transform steps and the paths they may never touch.
//!
//! A body step changes the request's JSON; the body is re-serialized only when
//! a step actually changed it, so an untouched request stays byte for byte.
//! Object key order is preserved. Protected headers and paths are refused when
//! the configuration loads, never at request time.

use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use tgv_protocol::route::Endpoint;

/// One change to a request.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "op", deny_unknown_fields)]
pub enum TransformStep {
    SetHeader {
        name: String,
        value: String,
    },
    RemoveHeader {
        name: String,
    },
    /// Set the value at a JSON Pointer (RFC 6901). The parent must exist.
    JsonSet {
        path: String,
        value: Value,
    },
    JsonRemove {
        path: String,
    },
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
            Endpoint::Models => &[],
        }
    }

    pub(super) fn check(&self, endpoints: &[Endpoint]) -> Result<(), StepError> {
        match self {
            Self::SetHeader { name, value } => {
                let name = Self::header(name)?;
                HeaderValue::from_str(value).map_err(|_| StepError::InvalidHeader)?;
                Self::unprotected_header(&name)
            }
            Self::RemoveHeader { name } => Self::unprotected_header(&Self::header(name)?),
            Self::JsonSet { path, .. } | Self::JsonRemove { path } => {
                if !path.starts_with('/') {
                    return Err(StepError::InvalidPath);
                }
                let protected = endpoints.iter().any(|endpoint| {
                    Self::protected_paths(*endpoint).iter().any(|protected| {
                        path == protected || path.starts_with(&format!("{protected}/"))
                    })
                });
                if protected {
                    Err(StepError::ProtectedPath)
                } else {
                    Ok(())
                }
            }
        }
    }

    fn header(name: &str) -> Result<HeaderName, StepError> {
        HeaderName::from_bytes(name.as_bytes()).map_err(|_| StepError::InvalidHeader)
    }

    fn unprotected_header(name: &HeaderName) -> Result<(), StepError> {
        if PROTECTED_HEADERS.contains(&name.as_str()) || name.as_str().starts_with("x-tgv-") {
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
            Self::SetHeader { name, .. } | Self::RemoveHeader { name } => name,
            Self::JsonSet { path, .. } | Self::JsonRemove { path } => path,
        }
    }

    /// Run the step. Checked steps never fail here; an unusable body is an
    /// outcome, not an error.
    pub fn apply(&self, headers: &mut HeaderMap, body: &mut RequestJson) -> StepOutcome {
        match self {
            Self::SetHeader { name, value } => {
                let (Ok(name), Ok(value)) = (Self::header(name), HeaderValue::from_str(value))
                else {
                    return StepOutcome::Unchanged;
                };
                if headers.get(&name) == Some(&value) && headers.get_all(&name).iter().count() == 1
                {
                    StepOutcome::Unchanged
                } else {
                    headers.insert(name, value);
                    StepOutcome::Applied
                }
            }
            Self::RemoveHeader { name } => match Self::header(name) {
                Ok(name) if headers.remove(&name).is_some() => StepOutcome::Applied,
                Ok(_) | Err(_) => StepOutcome::Unchanged,
            },
            Self::JsonSet { path, value } => body.edit(|document| {
                let (parent, key) = Self::split(path);
                match document.pointer_mut(parent) {
                    Some(Value::Object(map)) => {
                        if map.get(&key) == Some(value) {
                            StepOutcome::Unchanged
                        } else {
                            map.insert(key, value.clone());
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
                let (parent, key) = Self::split(path);
                match document.pointer_mut(parent) {
                    Some(Value::Object(map)) => {
                        if map.shift_remove(&key).is_some() {
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

    /// Split a checked JSON Pointer into its parent pointer and unescaped last key.
    fn split(path: &str) -> (&str, String) {
        let (parent, key) = path.rsplit_once('/').unwrap_or(("", path));
        (parent, key.replace("~1", "/").replace("~0", "~"))
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
