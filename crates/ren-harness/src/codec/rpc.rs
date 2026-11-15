//! A JSON-RPC message, told apart by its members: a request from the
//! client, a notification, or a response to Ren. The `"jsonrpc"` member is
//! not required: Codex leaves it out.

use serde::Deserialize;
use serde_json::Value;

/// A JSON-RPC message.
#[derive(Debug, PartialEq)]
pub(crate) enum JsonRpc {
    /// The client asks Ren for something and waits on the answer.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// The answer to a request Ren sent.
    Response {
        id: Value,
        outcome: Result<Value, RpcError>,
    },
    /// JSON that is none of these.
    Invalid,
}

/// The error a response carries.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct RpcError {
    /// What went wrong, when it says.
    pub(crate) message: Option<String>,
}

/// The members that tell a message apart. A `null` member is as absent.
#[derive(Deserialize)]
struct Members {
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
    result: Option<Value>,
    error: Option<RpcError>,
}

impl JsonRpc {
    /// `body` told apart: by whether it has a method and an id.
    pub(crate) fn classify(body: Value) -> Self {
        let Ok(members) = serde_json::from_value::<Members>(body) else {
            return Self::Invalid;
        };
        let params = members.params.unwrap_or(Value::Null);
        match (members.method, members.id) {
            (Some(method), Some(id)) => Self::Request { id, method, params },
            (Some(method), None) => Self::Notification { method, params },
            (None, Some(id)) => Self::Response {
                id,
                outcome: match members.error {
                    Some(error) => Err(error),
                    None => Ok(members.result.unwrap_or(Value::Null)),
                },
            },
            (None, None) => Self::Invalid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{JsonRpc, RpcError};
    use serde_json::{Value, json};

    #[test]
    fn messages_are_told_apart_by_method_and_id() {
        assert_eq!(
            JsonRpc::classify(json!({ "id": 1, "method": "ask", "params": { "a": 1 } })),
            JsonRpc::Request {
                id: json!(1),
                method: "ask".into(),
                params: json!({ "a": 1 })
            }
        );
        assert_eq!(
            JsonRpc::classify(json!({ "jsonrpc": "2.0", "method": "news", "id": null })),
            JsonRpc::Notification {
                method: "news".into(),
                params: Value::Null
            }
        );
        assert_eq!(
            JsonRpc::classify(json!({ "id": "a", "result": { "ok": true } })),
            JsonRpc::Response {
                id: json!("a"),
                outcome: Ok(json!({ "ok": true }))
            }
        );
        assert_eq!(
            JsonRpc::classify(json!({ "id": 2, "error": { "code": 1 } })),
            JsonRpc::Response {
                id: json!(2),
                outcome: Err(RpcError { message: None })
            }
        );
        assert_eq!(JsonRpc::classify(json!({ "result": 1 })), JsonRpc::Invalid);
        assert_eq!(JsonRpc::classify(json!([1])), JsonRpc::Invalid);
        assert_eq!(
            JsonRpc::classify(json!({ "id": 1, "method": 5 })),
            JsonRpc::Invalid
        );
    }
}
