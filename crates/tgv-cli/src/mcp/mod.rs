//! `tgv mcp`'s pure core: the Model Context Protocol over newline-delimited
//! JSON-RPC, serving read-only tools over TGV's history. It performs no I/O:
//! the command reads each line, hands it to [`McpServer::handle`], runs the
//! tool calls it asks for through the management API, and writes the replies.
//!
//! It serves both protocol eras. A request whose `_meta` names a protocol
//! version is served statelessly by that modern revision; an `initialize`
//! request selects a legacy, handshake-based revision for the rest of the
//! process, and later requests without a version are served by it.

mod tools;

#[cfg(test)]
mod purity;

pub use tools::{ArgumentError, BodyBudget, BodyExcerpt, ToolCall, ToolName};

use serde_json::{Map, Value, json};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use tgv_protocol::label::{Labeled, Stored};

/// Modern protocol revisions: version, identity, and capabilities travel in
/// every request's `_meta`.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ModernVersion {
    #[strum(to_string = "2026-07-28")]
    Jul2026,
}

/// Legacy protocol revisions, negotiated once by `initialize`. Their tool
/// messages are the subset this server uses in every one of them.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum LegacyVersion {
    #[strum(to_string = "2025-11-25")]
    Nov2025,
    #[strum(to_string = "2025-06-18")]
    Jun2025,
    #[strum(to_string = "2025-03-26")]
    Mar2025,
    #[strum(to_string = "2024-11-05")]
    Nov2024,
}

impl LegacyVersion {
    /// Offered when a client asks for a legacy revision this server lacks.
    const LATEST: Self = Self::Nov2025;
}

/// The requests this server answers. Any other method is kept as text and
/// refused as not found.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum Method {
    #[strum(to_string = "server/discover")]
    Discover,
    Initialize,
    Ping,
    #[strum(to_string = "tools/list")]
    ToolsList,
    #[strum(to_string = "tools/call")]
    ToolsCall,
}

/// Which revision a request is served by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Era {
    Modern(ModernVersion),
    Legacy(LegacyVersion),
}

/// A JSON-RPC request ID: a string or an integer, echoed unchanged.
#[derive(Debug, Clone, PartialEq)]
struct RequestId(Value);

/// One parsed line.
enum Incoming {
    Request {
        id: RequestId,
        method: Stored<Method>,
        params: Map<String, Value>,
    },
    /// A notification, or a response to a request this server never sends.
    Unanswered,
}

/// JSON-RPC and MCP error codes this server emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorCode {
    Parse,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    UnsupportedProtocolVersion,
}

impl ErrorCode {
    fn code(self) -> i64 {
        match self {
            Self::Parse => -32700,
            Self::InvalidRequest => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::UnsupportedProtocolVersion => -32022,
        }
    }
}

/// A protocol error, answered as a JSON-RPC error.
#[derive(Debug, Clone, PartialEq)]
struct RpcError {
    code: ErrorCode,
    message: String,
    data: Option<Value>,
}

impl RpcError {
    fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    fn reply(self, id: Option<&RequestId>) -> Value {
        let mut error = json!({ "code": self.code.code(), "message": self.message });
        if let (Some(data), Some(object)) = (self.data, error.as_object_mut()) {
            object.insert("data".to_owned(), data);
        }
        json!({
            "jsonrpc": "2.0",
            "id": id.map_or(Value::Null, |id| id.0.clone()),
            "error": error,
        })
    }
}

/// What the command does with one line.
#[must_use]
pub enum Step {
    /// Write this message.
    Reply(Value),
    /// Run this tool call, then write [`PendingCall::answer`].
    Run(PendingCall),
    /// Write nothing: the line was a notification or a response.
    Silent,
}

/// A tool call to run, and how to answer it.
#[must_use]
pub struct PendingCall {
    pub tool: ToolCall,
    reply: Replier,
}

/// What a tool call produced.
pub enum ToolOutcome {
    /// The tool's data, given to the model as JSON.
    Data(Value),
    /// Why it failed, given to the model so it can adjust and retry.
    Failed(String),
}

/// Answers one request in its era.
struct Replier {
    id: RequestId,
    era: Era,
    server_version: String,
}

impl Replier {
    fn complete(&self, result: Value) -> Value {
        let result = match (self.era, result) {
            (Era::Modern(_), Value::Object(mut object)) => {
                object.insert("resultType".to_owned(), json!("complete"));
                object.insert(
                    "_meta".to_owned(),
                    json!({ "io.modelcontextprotocol/serverInfo": McpServer::server_info(&self.server_version) }),
                );
                Value::Object(object)
            }
            (Era::Modern(_) | Era::Legacy(_), result) => result,
        };
        json!({ "jsonrpc": "2.0", "id": self.id.0, "result": result })
    }
}

impl PendingCall {
    /// The reply carrying `outcome`: data as compact JSON text, a failure as
    /// a tool error the model can read.
    pub fn answer(self, outcome: ToolOutcome) -> Value {
        self.reply.complete(outcome.into_result())
    }
}

impl ToolOutcome {
    /// A `tools/call` result: one text block, and whether it is an error.
    fn into_result(self) -> Value {
        let (text, is_error) = match self {
            Self::Data(value) => (value.to_string(), false),
            Self::Failed(message) => (message, true),
        };
        json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error,
        })
    }
}

/// The server's state: only the legacy revision an `initialize` chose.
pub struct McpServer {
    version: String,
    legacy: Option<LegacyVersion>,
}

impl McpServer {
    const NAME: &str = "tgv";
    const PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
    const CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
    const INSTRUCTIONS: &str = "TGV records the traffic between coding agents (Claude Code, Codex, and Pi) and their model providers. These read-only tools look up recorded calls: search_history finds what prompts, replies, and tool input and output said; search_calls finds calls, newest first, by text, session, client, model, tool, or status; list_sessions and session_calls follow a session's calls in order; session_report shows where context and prompt-cache tokens went; list_turns and get_turn show what each user prompt led to (tool calls, their results, files written, commands run); get_call shows one call's attempts, inspection, and observation log; get_body reads a bounded excerpt of a request or response body; usage totals calls, errors, and tokens; list_exposures shows credentials calls carried (never the secrets). Times are Unix milliseconds (UTC). Pass a listing's `next` value as `after` to read the next page. Bodies may hold private prompts and tool output. Token counts are what providers reported, not subscription allowance or cost.";

    /// A server reporting `version` (TGV's own) as its implementation version.
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            legacy: None,
        }
    }

    /// The step for one line of input, which holds one JSON-RPC message.
    pub fn handle(&mut self, line: &[u8]) -> Step {
        let (id, method, params) = match Incoming::parse(line) {
            Ok(Incoming::Request { id, method, params }) => (id, method, params),
            Ok(Incoming::Unanswered) => return Step::Silent,
            Err(error) => return Step::Reply(error.reply(None)),
        };
        match self.request(&id, &method, &params) {
            Ok(step) => step,
            Err(error) => Step::Reply(error.reply(Some(&id))),
        }
    }

    /// The step for one line that could not be read whole.
    pub fn oversized(&self) -> Value {
        RpcError::new(ErrorCode::Parse, "message exceeds the line limit").reply(None)
    }

    fn request(
        &mut self,
        id: &RequestId,
        method: &Stored<Method>,
        params: &Map<String, Value>,
    ) -> Result<Step, RpcError> {
        let era = self.era(method, params)?;
        let reply = Replier {
            id: id.clone(),
            era,
            server_version: self.version.clone(),
        };
        let method = method.known().ok_or_else(|| {
            RpcError::new(
                ErrorCode::MethodNotFound,
                format!("method not found: {}", method.label()),
            )
        })?;
        let result = match (method, era) {
            (Method::Discover, Era::Modern(_)) => json!({
                "supportedVersions": Self::supported(),
                "capabilities": { "tools": {} },
                "instructions": Self::INSTRUCTIONS,
            }),
            (Method::Initialize, Era::Legacy(version)) => {
                self.legacy = Some(version);
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": Self::server_info(&self.version),
                    "instructions": Self::INSTRUCTIONS,
                })
            }
            (Method::Ping, Era::Modern(_) | Era::Legacy(_)) => json!({}),
            (Method::ToolsList, Era::Modern(_) | Era::Legacy(_)) => {
                json!({ "tools": ToolName::definitions() })
            }
            (Method::ToolsCall, Era::Modern(_) | Era::Legacy(_)) => {
                return Ok(match Self::call(params)? {
                    Ok(tool) => Step::Run(PendingCall { tool, reply }),
                    Err(message) => {
                        Step::Reply(reply.complete(ToolOutcome::Failed(message).into_result()))
                    }
                });
            }
            (Method::Discover, Era::Legacy(_)) | (Method::Initialize, Era::Modern(_)) => {
                return Err(RpcError::new(
                    ErrorCode::MethodNotFound,
                    format!("method not found in this protocol version: {method}"),
                ));
            }
        };
        Ok(Step::Reply(reply.complete(result)))
    }

    /// The era serving a request: the modern version its `_meta` names, a
    /// legacy version `initialize` negotiates, or the one negotiated before.
    fn era(&self, method: &Stored<Method>, params: &Map<String, Value>) -> Result<Era, RpcError> {
        let meta = params.get("_meta").and_then(Value::as_object);
        if let Some(requested) = meta.and_then(|meta| meta.get(Self::PROTOCOL_VERSION)) {
            let version = requested
                .as_str()
                .and_then(ModernVersion::from_label)
                .ok_or_else(|| RpcError {
                    code: ErrorCode::UnsupportedProtocolVersion,
                    message: "Unsupported protocol version".to_owned(),
                    data: Some(json!({ "supported": Self::supported(), "requested": requested })),
                })?;
            if !meta
                .and_then(|meta| meta.get(Self::CLIENT_CAPABILITIES))
                .is_some_and(Value::is_object)
            {
                return Err(RpcError::new(
                    ErrorCode::InvalidParams,
                    format!("_meta must carry {}", Self::CLIENT_CAPABILITIES),
                ));
            }
            return Ok(Era::Modern(version));
        }
        if method.known() == Some(Method::Initialize) {
            let requested = params.get("protocolVersion").and_then(Value::as_str);
            return Ok(Era::Legacy(
                requested
                    .and_then(LegacyVersion::from_label)
                    .unwrap_or(LegacyVersion::LATEST),
            ));
        }
        self.legacy.map(Era::Legacy).ok_or_else(|| {
            RpcError::new(
                ErrorCode::InvalidParams,
                format!(
                    "_meta must carry {} (supported: {}), or the client must send initialize first",
                    Self::PROTOCOL_VERSION,
                    Self::supported().join(", ")
                ),
            )
        })
    }

    /// The tool call `params` name. An unknown tool or a malformed request is
    /// a protocol error; arguments the tool rejects are a tool error, which
    /// the model reads and can correct.
    fn call(params: &Map<String, Value>) -> Result<Result<ToolCall, String>, RpcError> {
        let name = params.get("name").and_then(Value::as_str).ok_or_else(|| {
            RpcError::new(ErrorCode::InvalidParams, "tools/call needs a tool name")
        })?;
        let tool = ToolName::from_label(name).ok_or_else(|| {
            RpcError::new(ErrorCode::InvalidParams, format!("Unknown tool: {name}"))
        })?;
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(arguments)) => arguments.clone(),
            Some(_) => {
                return Err(RpcError::new(
                    ErrorCode::InvalidParams,
                    "tool arguments must be an object",
                ));
            }
        };
        Ok(ToolCall::parse(tool, arguments).map_err(|error| format!("{tool}: {error}")))
    }

    fn supported() -> Vec<&'static str> {
        ModernVersion::ALL
            .iter()
            .map(|version| version.label())
            .chain(LegacyVersion::ALL.iter().map(|version| version.label()))
            .collect()
    }

    fn server_info(version: &str) -> Value {
        json!({ "name": Self::NAME, "title": "TGV", "version": version })
    }
}

impl Incoming {
    /// One JSON-RPC message. Batches are not part of MCP.
    fn parse(line: &[u8]) -> Result<Self, RpcError> {
        let message: Value = serde_json::from_slice(line)
            .map_err(|_| RpcError::new(ErrorCode::Parse, "not a JSON message"))?;
        let Value::Object(mut message) = message else {
            return Err(RpcError::new(
                ErrorCode::InvalidRequest,
                "a message must be a JSON object",
            ));
        };
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(RpcError::new(
                ErrorCode::InvalidRequest,
                "jsonrpc must be \"2.0\"",
            ));
        }
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // A response to a request this server never sent.
            return Ok(Self::Unanswered);
        };
        let method = Stored::from_label(method);
        let Some(id) = message.shift_remove("id") else {
            return Ok(Self::Unanswered);
        };
        let id = match id {
            Value::String(_) => RequestId(id),
            Value::Number(ref number) if number.is_i64() || number.is_u64() => RequestId(id),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_) => {
                return Err(RpcError::new(
                    ErrorCode::InvalidRequest,
                    "id must be a string or an integer",
                ));
            }
        };
        let params = match message.shift_remove("params") {
            None => Map::new(),
            Some(Value::Object(params)) => params,
            Some(_) => {
                return Err(RpcError::new(
                    ErrorCode::InvalidParams,
                    "params must be an object",
                ));
            }
        };
        Ok(Self::Request { id, method, params })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODERN_META: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}"#;

    fn reply(server: &mut McpServer, line: &str) -> Value {
        match server.handle(line.as_bytes()) {
            Step::Reply(value) => value,
            Step::Run(_) => panic!("expected a reply, got a tool call: {line}"),
            Step::Silent => panic!("expected a reply, got nothing: {line}"),
        }
    }

    fn run(server: &mut McpServer, line: &str) -> PendingCall {
        match server.handle(line.as_bytes()) {
            Step::Run(pending) => pending,
            Step::Reply(value) => panic!("expected a tool call, got {value}"),
            Step::Silent => panic!("expected a tool call, got nothing"),
        }
    }

    #[test]
    fn modern_requests_are_served_without_a_handshake() {
        let mut server = McpServer::new("0.1.0");
        let discover = reply(
            &mut server,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"d","method":"server/discover","params":{{{MODERN_META}}}}}"#
            ),
        );
        assert_eq!(discover["id"], "d");
        assert_eq!(discover["result"]["resultType"], "complete");
        assert_eq!(discover["result"]["supportedVersions"][0], "2026-07-28");
        assert_eq!(discover["result"]["capabilities"], json!({ "tools": {} }));
        assert_eq!(
            discover["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "tgv"
        );
        let list = reply(
            &mut server,
            &format!(
                r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{{MODERN_META}}}}}"#
            ),
        );
        let names: Vec<_> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "search_history",
                "search_calls",
                "list_sessions",
                "session_calls",
                "session_report",
                "list_turns",
                "get_turn",
                "get_call",
                "get_call_activity",
                "get_body",
                "usage",
                "list_exposures"
            ]
        );
        // Statelessness: a request without its version is not served by an
        // earlier one's.
        let bare = reply(
            &mut server,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
        );
        assert_eq!(bare["error"]["code"], -32602);
    }

    #[test]
    fn unsupported_and_incomplete_modern_metadata_is_refused() {
        let mut server = McpServer::new("0.1.0");
        let old = reply(
            &mut server,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"1900-01-01","io.modelcontextprotocol/clientCapabilities":{}}}}"#,
        );
        assert_eq!(old["error"]["code"], -32022);
        assert_eq!(old["error"]["data"]["requested"], "1900-01-01");
        assert_eq!(old["error"]["data"]["supported"][0], "2026-07-28");
        let missing = reply(
            &mut server,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}"#,
        );
        assert_eq!(missing["error"]["code"], -32602);
        let initialize = reply(
            &mut server,
            &format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{{MODERN_META}}}}}"#
            ),
        );
        assert_eq!(initialize["error"]["code"], -32601);
    }

    #[test]
    fn initialize_selects_a_legacy_revision_for_the_process() {
        let mut server = McpServer::new("0.1.0");
        let initialized = reply(
            &mut server,
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"c","version":"1"}}}"#,
        );
        assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(initialized["result"]["serverInfo"]["name"], "tgv");
        assert!(initialized["result"].get("resultType").is_none());
        assert!(matches!(
            server.handle(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            Step::Silent
        ));
        let list = reply(
            &mut server,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 12);
        assert!(list["result"].get("resultType").is_none());
        let ping = reply(&mut server, r#"{"jsonrpc":"2.0","id":"p","method":"ping"}"#);
        assert_eq!(ping["result"], json!({}));
        // An unknown legacy version is answered with the latest one.
        let mut other = McpServer::new("0.1.0");
        let newer = reply(
            &mut other,
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#,
        );
        assert_eq!(newer["result"]["protocolVersion"], "2025-11-25");
    }

    #[test]
    fn tool_calls_are_handed_to_the_shell_and_answered_in_their_era() {
        let mut server = McpServer::new("0.1.0");
        let pending = run(
            &mut server,
            &format!(
                r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"list_sessions","arguments":{{"limit":5}},{MODERN_META}}}}}"#
            ),
        );
        assert!(matches!(pending.tool, ToolCall::ListSessions(limit) if limit.get() == 5));
        let answer = pending.answer(ToolOutcome::Data(json!([{ "id": "s" }])));
        assert_eq!(answer["id"], 7);
        assert_eq!(answer["result"]["resultType"], "complete");
        assert_eq!(answer["result"]["isError"], false);
        assert_eq!(answer["result"]["content"][0]["text"], r#"[{"id":"s"}]"#);

        let mut legacy = McpServer::new("0.1.0");
        let _ = reply(
            &mut legacy,
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#,
        );
        let pending = run(
            &mut legacy,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"get_call","arguments":{"call_id":"00000000-0000-0000-0000-000000000001"}}}"#,
        );
        let failed = pending.answer(ToolOutcome::Failed("cannot reach the gateway".to_owned()));
        assert_eq!(failed["result"]["isError"], true);
        assert!(failed["result"].get("resultType").is_none());
    }

    #[test]
    fn bad_arguments_are_tool_errors_and_unknown_tools_protocol_errors() {
        let mut server = McpServer::new("0.1.0");
        let bad = reply(
            &mut server,
            &format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"search_calls","arguments":{{"client":"vim"}},{MODERN_META}}}}}"#
            ),
        );
        assert_eq!(bad["result"]["isError"], true);
        let text = bad["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("claude_code"), "{text}");
        let unknown = reply(
            &mut server,
            &format!(
                r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"drop_tables",{MODERN_META}}}}}"#
            ),
        );
        assert_eq!(unknown["error"]["code"], -32602);
        assert!(
            unknown["error"]["message"]
                .as_str()
                .unwrap()
                .contains("drop_tables")
        );
    }

    #[test]
    fn malformed_messages_get_json_rpc_errors() {
        let mut server = McpServer::new("0.1.0");
        assert_eq!(reply(&mut server, "{not json")["error"]["code"], -32700);
        assert_eq!(reply(&mut server, "[]")["error"]["code"], -32600);
        assert_eq!(
            reply(&mut server, r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#)["error"]["code"],
            -32600
        );
        assert_eq!(
            reply(
                &mut server,
                r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#
            )["error"]["code"],
            -32600
        );
        let unknown = reply(
            &mut server,
            &format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"resources/list","params":{{{MODERN_META}}}}}"#
            ),
        );
        assert_eq!(unknown["error"]["code"], -32601);
        assert_eq!(unknown["id"], 1);
        assert!(matches!(
            server.handle(br#"{"jsonrpc":"2.0","id":5,"result":{}}"#),
            Step::Silent
        ));
        assert_eq!(server.oversized()["error"]["code"], -32700);
    }
}
