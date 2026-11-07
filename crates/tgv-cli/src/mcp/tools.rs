//! The tools `tgv mcp` serves: their definitions, their arguments parsed
//! once into the store's typed queries, and the excerpt of a body preview
//! that a model reads.

use serde::Deserialize;
use serde_json::{Map, Value, json};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt, str::FromStr};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use tgv_gateway::api::preview::{DecodeIntegrity, PreviewDecoding, PreviewExtent, RawFormat};
use tgv_harness::Harness;
use tgv_protocol::{
    direction::Direction,
    label::{Labeled, Stored},
    route::Provider,
};
use tgv_recorder::{id::CallId, time::UnixMillis};
use tgv_store::query::{
    CallFilter, CallQuery, Cursor, KindFilter, PageLimit, SearchText, StatusFilter, TimeRange,
    ToolFilter,
};
use tgv_store::read::ReadRequest;
use tgv_store::search::{ExposureQuery, SearchField, SearchQuery, SearchTerms};

/// The tools, in the order `tools/list` returns them.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum ToolName {
    SearchHistory,
    SearchCalls,
    ListSessions,
    SessionCalls,
    SessionReport,
    ListTurns,
    GetTurn,
    GetCall,
    GetCallActivity,
    GetBody,
    Usage,
    ListExposures,
}

/// A tool call with its arguments parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCall {
    /// A history read, as the read API serves it.
    Read(ReadRequest),
    GetBody {
        call: CallId,
        direction: Direction,
        budget: BodyBudget,
    },
}

/// Why a tool refused its arguments; the model reads it and can retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgumentError(String);

impl fmt::Display for ArgumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How many characters of a body an excerpt shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyBudget(usize);

impl BodyBudget {
    const DEFAULT: u32 = 20_000;
    const MAX: u32 = 200_000;

    fn new(chars: Option<u32>) -> Result<Self, ArgumentError> {
        let chars = chars.unwrap_or(Self::DEFAULT);
        if !(1..=Self::MAX).contains(&chars) {
            return Err(ArgumentError(format!(
                "max_chars must be between 1 and {}",
                Self::MAX
            )));
        }
        Ok(Self(usize::try_from(chars).unwrap_or(usize::MAX)))
    }
}

/// One named argument, for error messages that say which one is wrong.
#[derive(Clone, Copy)]
struct Field(&'static str);

impl Field {
    /// A value, unless it is absent or blank (models often send "").
    fn present(value: Option<String>) -> Option<String> {
        value.filter(|value| !value.trim().is_empty())
    }

    fn label<T: Labeled>(self, value: Option<String>) -> Result<Option<T>, ArgumentError> {
        Self::present(value)
            .map(|value| {
                T::from_label(value.trim()).ok_or_else(|| {
                    ArgumentError(format!(
                        "{} must be one of: {}",
                        self.0,
                        ToolName::labels::<T>().join(", ")
                    ))
                })
            })
            .transpose()
    }

    fn text(self, value: Option<String>) -> Result<Option<SearchText>, ArgumentError> {
        Self::present(value)
            .map(|value| {
                SearchText::new(&value).map_err(|_| {
                    ArgumentError(format!(
                        "{} must be 1 to {} bytes",
                        self.0,
                        SearchText::MAX_BYTES
                    ))
                })
            })
            .transpose()
    }

    fn id<T: FromStr>(self, value: &str) -> Result<T, ArgumentError> {
        value
            .trim()
            .parse()
            .map_err(|_| ArgumentError(format!("{} is not a TGV ID", self.0)))
    }

    fn optional_id<T: FromStr>(self, value: Option<String>) -> Result<Option<T>, ArgumentError> {
        Self::present(value).map(|id| self.id(&id)).transpose()
    }

    fn cursor(self, value: Option<String>) -> Result<Option<Cursor>, ArgumentError> {
        Self::present(value)
            .map(|value| {
                value.trim().parse().map_err(|_| {
                    ArgumentError(format!(
                        "{} must be a listing's `next` value, unchanged",
                        self.0
                    ))
                })
            })
            .transpose()
    }

    fn limit(self, value: Option<u32>, default: u32) -> Result<PageLimit, ArgumentError> {
        let limit = value.unwrap_or(default);
        PageLimit::new(limit)
            .ok()
            .filter(|_| limit <= ToolCall::MAX_ROWS)
            .ok_or_else(|| {
                ArgumentError(format!(
                    "{} must be between 1 and {}",
                    self.0,
                    ToolCall::MAX_ROWS
                ))
            })
    }

    fn range(from: Option<u64>, until: Option<u64>) -> Result<TimeRange, ArgumentError> {
        TimeRange::new(
            from.map(UnixMillis::from_millis),
            until.map(UnixMillis::from_millis),
        )
        .map_err(|_| ArgumentError("until_ms must not be before from_ms".to_owned()))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    q: Option<String>,
    session_id: Option<String>,
    client: Option<String>,
    kind: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    tool: Option<String>,
    status: Option<String>,
    has_tools: Option<bool>,
    from_ms: Option<u64>,
    until_ms: Option<u64>,
    limit: Option<u32>,
    after: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryArgs {
    query: String,
    session_id: Option<String>,
    client: Option<String>,
    field: Option<String>,
    from_ms: Option<u64>,
    until_ms: Option<u64>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageArgs {
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionArgs {
    session_id: String,
    limit: Option<u32>,
    after: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TurnsArgs {
    session_id: String,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportArgs {
    session_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TurnArgs {
    turn_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallArgs {
    call_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BodyArgs {
    call_id: String,
    direction: String,
    max_chars: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExposureArgs {
    session_id: Option<String>,
    from_ms: Option<u64>,
    until_ms: Option<u64>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RangeArgs {
    from_ms: Option<u64>,
    until_ms: Option<u64>,
}

impl ToolCall {
    /// The most rows one listing returns, to keep answers within a model's
    /// context.
    const MAX_ROWS: u32 = 100;
    const DEFAULT_ROWS: u32 = 20;

    /// The call `tool` names, with `arguments` parsed and checked.
    pub fn parse(tool: ToolName, arguments: Map<String, Value>) -> Result<Self, ArgumentError> {
        let arguments = Value::Object(arguments);
        let read = match tool {
            ToolName::SearchHistory => {
                let args: HistoryArgs = Self::arguments(arguments)?;
                ReadRequest::Search(SearchQuery {
                    terms: SearchTerms::parse(&args.query).map_err(|_| {
                        ArgumentError(format!(
                            "query must have 1 to {} bytes and at least one word",
                            SearchTerms::MAX_BYTES
                        ))
                    })?,
                    session: Field("session_id").optional_id(args.session_id)?,
                    client: Field("client").label::<Harness>(args.client)?,
                    field: Field("field").label::<SearchField>(args.field)?,
                    time: Field::range(args.from_ms, args.until_ms)?,
                    limit: Field("limit").limit(args.limit, Self::DEFAULT_ROWS)?,
                })
            }
            ToolName::SearchCalls => {
                let args: SearchArgs = Self::arguments(arguments)?;
                ReadRequest::Calls(CallQuery {
                    limit: Field("limit").limit(args.limit, Self::DEFAULT_ROWS)?,
                    after: Field("after").cursor(args.after)?,
                    filter: CallFilter {
                        session: Field("session_id").optional_id(args.session_id)?,
                        client: Field("client").label::<Harness>(args.client)?,
                        kind: Field("kind").label::<KindFilter>(args.kind)?,
                        provider: Field("provider").label::<Provider>(args.provider)?,
                        model: Field("model").text(args.model)?,
                        tool: Field("tool").text(args.tool)?,
                        text: Field("q").text(args.q)?,
                        status: Field("status").label::<StatusFilter>(args.status)?,
                        tools: args.has_tools.map(|has| {
                            if has {
                                ToolFilter::WithTools
                            } else {
                                ToolFilter::WithoutTools
                            }
                        }),
                        time: Field::range(args.from_ms, args.until_ms)?,
                    },
                })
            }
            ToolName::ListSessions => {
                let args: PageArgs = Self::arguments(arguments)?;
                ReadRequest::Sessions(Field("limit").limit(args.limit, Self::DEFAULT_ROWS)?)
            }
            ToolName::SessionCalls => {
                let args: SessionArgs = Self::arguments(arguments)?;
                ReadRequest::SessionCalls {
                    session: Field("session_id").id(&args.session_id)?,
                    limit: Field("limit").limit(args.limit, Self::MAX_ROWS)?,
                    after: Field("after").cursor(args.after)?,
                }
            }
            ToolName::SessionReport => {
                let args: ReportArgs = Self::arguments(arguments)?;
                ReadRequest::Report(Field("session_id").id(&args.session_id)?)
            }
            ToolName::ListTurns => {
                let args: TurnsArgs = Self::arguments(arguments)?;
                ReadRequest::Turns {
                    session: Field("session_id").id(&args.session_id)?,
                    limit: Field("limit").limit(args.limit, Self::DEFAULT_ROWS)?,
                }
            }
            ToolName::GetTurn => {
                let args: TurnArgs = Self::arguments(arguments)?;
                ReadRequest::Turn(Field("turn_id").id(&args.turn_id)?)
            }
            ToolName::GetCall => {
                let args: CallArgs = Self::arguments(arguments)?;
                ReadRequest::Call(Field("call_id").id(&args.call_id)?)
            }
            ToolName::GetCallActivity => {
                let args: CallArgs = Self::arguments(arguments)?;
                ReadRequest::CallActivity(Field("call_id").id(&args.call_id)?)
            }
            ToolName::GetBody => {
                let args: BodyArgs = Self::arguments(arguments)?;
                return Ok(Self::GetBody {
                    call: Field("call_id").id(&args.call_id)?,
                    direction: Field("direction")
                        .label::<Direction>(Some(args.direction))?
                        .ok_or_else(|| {
                            ArgumentError("direction must be request or response".to_owned())
                        })?,
                    budget: BodyBudget::new(args.max_chars)?,
                });
            }
            ToolName::Usage => {
                let args: RangeArgs = Self::arguments(arguments)?;
                ReadRequest::Usage(Field::range(args.from_ms, args.until_ms)?)
            }
            ToolName::ListExposures => {
                let args: ExposureArgs = Self::arguments(arguments)?;
                ReadRequest::Exposures(ExposureQuery {
                    session: Field("session_id").optional_id(args.session_id)?,
                    time: Field::range(args.from_ms, args.until_ms)?,
                    limit: Field("limit").limit(args.limit, Self::DEFAULT_ROWS)?,
                })
            }
        };
        Ok(Self::Read(read))
    }

    fn arguments<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T, ArgumentError> {
        serde_json::from_value(arguments).map_err(|error| ArgumentError(error.to_string()))
    }
}

impl ToolName {
    /// Every tool's definition, in a fixed order.
    pub fn definitions() -> Vec<Value> {
        Self::ALL.iter().map(|tool| tool.definition()).collect()
    }

    fn definition(self) -> Value {
        let (title, description, properties, required) = match self {
            Self::SearchHistory => (
                "Search history",
                "Full-text search over what recorded calls contain: the user's prompts, the model's replies, and tool inputs (commands, file paths, patches) and outputs. Words must all match; \"quoted text\" matches as a phrase; punctuation separates words, so search a hyphenated name as a phrase. Returns {hits, indexed_calls}, newest call first; each hit has the call, session, turn, field, and a snippet with matches marked «like this». Use get_turn or get_call_activity on a hit for the rest. Calls are indexed shortly after they finish.",
                json!({
                    "query": { "type": "string", "maxLength": SearchTerms::MAX_BYTES, "description": "Words and \"quoted phrases\" to find." },
                    "session_id": Self::string("Only this TGV session, as list_sessions returns it."),
                    "client": Self::labels_property::<Harness>("Only calls from this coding agent."),
                    "field": Self::labels_property::<SearchField>("Only this part of a call."),
                    "from_ms": Self::time("Calls started at or after this time."),
                    "until_ms": Self::time("Calls started at or before this time."),
                    "limit": Self::limit(ToolCall::DEFAULT_ROWS),
                }),
                json!(["query"]),
            ),
            Self::SearchCalls => (
                "Search calls",
                "Find recorded calls, newest first. Filters combine with AND. `q` matches call IDs, reported models, and tool names (never bodies). Model and tool filters use the inspection saved when a call finishes. Returns {calls, next}; pass `next` as `after` for the next page.",
                json!({
                    "q": Self::text("Text in a call ID, model, or tool name."),
                    "session_id": Self::string("A TGV session ID, as list_sessions returns it."),
                    "client": Self::labels_property::<Harness>("The coding agent the call came from."),
                    "kind": Self::labels_property::<KindFilter>("`model` for calls that ask a model for output; `other` for token counts, model listings, and WebSocket handshakes."),
                    "provider": Self::labels_property::<Provider>("The upstream provider."),
                    "model": Self::text("Text in the reported model name."),
                    "tool": Self::text("Text in a tool name the model called."),
                    "status": Self::labels_property::<StatusFilter>("`errors` covers HTTP errors, failed streams, and failed model outcomes; `interrupted` covers cancelled and interrupted calls."),
                    "has_tools": { "type": "boolean", "description": "Whether the model called tools." },
                    "from_ms": Self::time("Calls started at or after this time."),
                    "until_ms": Self::time("Calls started at or before this time."),
                    "limit": Self::limit(ToolCall::DEFAULT_ROWS),
                    "after": Self::string("A previous answer's `next` value, unchanged."),
                }),
                json!([]),
            ),
            Self::ListSessions => (
                "List sessions",
                "Recent sessions, newest first, with their call, turn, and error counts and token totals.",
                json!({ "limit": Self::limit(ToolCall::DEFAULT_ROWS) }),
                json!([]),
            ),
            Self::SessionCalls => (
                "Session calls",
                "One session's calls, oldest first. Returns {calls, next}; pass `next` as `after` for the next page.",
                json!({
                    "session_id": Self::string("A TGV session ID, as list_sessions returns it."),
                    "limit": Self::limit(ToolCall::MAX_ROWS),
                    "after": Self::string("A previous answer's `next` value, unchanged."),
                }),
                json!(["session_id"]),
            ),
            Self::SessionReport => (
                "Session report",
                "Why a session was slow or used many tokens, from its call records: per turn, context size at start and end, prompt-cache hit percent, tokens written to the cache, output tokens, and idle time before it; session totals; and findings: `cache_rebuild` (a call wrote most of the context to the cache again, with the idle time before it) and `context_growth` (the largest steps past the earlier peak). Token counts are provider-reported, not cost or subscription allowance. get_turn adds `large_tool_output` and `repeated_failure` findings.",
                json!({ "session_id": Self::string("A TGV session ID, as list_sessions returns it.") }),
                json!(["session_id"]),
            ),
            Self::ListTurns => (
                "List turns",
                "A session's turns (one user prompt and everything the agent did for it), newest first: each turn's ID, start and end, call and error counts, tokens, and the start of its prompt. Turns are the client's own grouping; calls without one (Pi) have none.",
                json!({
                    "session_id": Self::string("A TGV session ID, as list_sessions returns it."),
                    "limit": Self::limit(ToolCall::DEFAULT_ROWS),
                }),
                json!(["session_id"]),
            ),
            Self::GetTurn => (
                "Get turn",
                "What one turn did, read from its recorded bodies: the user's prompt; each call's tool calls with their inputs, effects (files read or written, commands run, searches, subagents), and the result status the client reported (`result` is null when no later request answered); reply excerpts; and the turn's files, commands, and failed or unanswered tool calls. A tool call is what the model requested, not proof it ran.",
                json!({ "turn_id": Self::string("A TGV turn ID, as list_turns returns it.") }),
                json!(["turn_id"]),
            ),
            Self::GetCall => (
                "Get call",
                "One call in detail: its attempts and upstream decisions, the inspection of its request and response (model, tool calls, stop reason, token usage), and its observation log.",
                json!({ "call_id": Self::string("A TGV call ID.") }),
                json!(["call_id"]),
            ),
            Self::GetCallActivity => (
                "Get call activity",
                "One call's activity, as get_turn shows each call: user input, tool calls with effects, and the reply. Use it for calls that belong to no turn.",
                json!({ "call_id": Self::string("A TGV call ID.") }),
                json!(["call_id"]),
            ),
            Self::GetBody => (
                "Get body",
                "An excerpt of a call's request or response body, decoded to text when possible. The response is the final attempt's. `extent` says whether the excerpt is the complete body; streamed responses are server-sent events.",
                json!({
                    "call_id": Self::string("A TGV call ID."),
                    "direction": Self::labels_property::<Direction>("Which body."),
                    "max_chars": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": BodyBudget::MAX,
                        "default": BodyBudget::DEFAULT,
                        "description": "The most characters to return."
                    },
                }),
                json!(["call_id", "direction"]),
            ),
            Self::Usage => (
                "Usage",
                "Calls, errors, and tokens (input, output, cache reads) in total, by UTC hour, and by model, for calls started in an optional range. Token counts are provider-reported, not subscription allowance or cost.",
                json!({
                    "from_ms": Self::time("Calls started at or after this time."),
                    "until_ms": Self::time("Calls started at or before this time."),
                }),
                json!([]),
            ),
            Self::ListExposures => (
                "List exposures",
                "Credentials that recorded calls carried, newest first: API keys and tokens in well-known formats (Anthropic, OpenAI, GitHub, AWS, Google, Slack, Stripe), private key blocks, and JWTs, found in the user's input, tool input or output (all sent to the provider), or the model's reply. Each has the call, session, turn, field, the secret's format, a masked hint, a fingerprint (equal for the same secret), and how often it appeared. The secrets themselves are never stored or returned. A secret sent to a provider should be rotated: TGV cannot recall it.",
                json!({
                    "session_id": Self::string("Only this TGV session, as list_sessions returns it."),
                    "from_ms": Self::time("Calls started at or after this time."),
                    "until_ms": Self::time("Calls started at or before this time."),
                    "limit": Self::limit(ToolCall::DEFAULT_ROWS),
                }),
                json!([]),
            ),
        };
        json!({
            "name": self,
            "title": title,
            "description": description,
            "inputSchema": {
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false,
            },
            "annotations": {
                "title": title,
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false,
            },
        })
    }

    fn labels<T: Labeled>() -> Vec<&'static str> {
        T::ALL.iter().map(|variant| variant.label()).collect()
    }

    fn labels_property<T: Labeled>(description: &str) -> Value {
        json!({ "type": "string", "enum": Self::labels::<T>(), "description": description })
    }

    fn string(description: &str) -> Value {
        json!({ "type": "string", "description": description })
    }

    fn text(description: &str) -> Value {
        json!({ "type": "string", "maxLength": SearchText::MAX_BYTES, "description": description })
    }

    fn time(description: &str) -> Value {
        json!({ "type": "integer", "minimum": 0, "description": format!("{description} Unix milliseconds, UTC.") })
    }

    fn limit(default: u32) -> Value {
        json!({ "type": "integer", "minimum": 1, "maximum": ToolCall::MAX_ROWS, "default": default })
    }
}

/// A body preview as the management API sends it.
#[derive(Deserialize)]
struct Preview {
    observed_bytes: Option<u64>,
    stored_bytes: u64,
    extent: Stored<PreviewExtent>,
    text: Option<String>,
    raw: RawPart,
    decoding: DecodingPart,
}

#[derive(Deserialize)]
struct RawPart {
    text: String,
    format: Stored<RawFormat>,
}

#[derive(Deserialize)]
struct DecodingPart {
    applied: Stored<PreviewDecoding>,
    integrity: Stored<DecodeIntegrity>,
}

/// What `get_body` returns: a preview's decoded text when it has any, else
/// its raw bytes, cut to the budget, with the preview's own extent.
pub struct BodyExcerpt {
    preview: Preview,
    budget: BodyBudget,
}

impl BodyExcerpt {
    /// The excerpt of `preview`, the management API's answer.
    pub fn new(preview: Value, budget: BodyBudget) -> Result<Self, serde_json::Error> {
        Ok(Self {
            preview: serde_json::from_value(preview)?,
            budget,
        })
    }

    pub fn into_value(self) -> Value {
        let Self { preview, budget } = self;
        let (field, format, text) = match preview.text {
            Some(text) => ("text", None, text),
            None => ("raw", Some(preview.raw.format), preview.raw.text),
        };
        let total = text.chars().count();
        let shown: String = text.chars().take(budget.0).collect();
        let mut excerpt = json!({
            "extent": preview.extent,
            "observed_bytes": preview.observed_bytes,
            "stored_bytes": preview.stored_bytes,
            "decoding": { "applied": preview.decoding.applied, "integrity": preview.decoding.integrity },
            "omitted_chars": total.saturating_sub(budget.0),
        });
        if let Some(object) = excerpt.as_object_mut() {
            object.insert(field.to_owned(), Value::String(shown));
            if let Some(format) = format {
                object.insert("raw_format".to_owned(), json!(format));
            }
        }
        excerpt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn search_arguments_parse_into_the_store_query() {
        let call = ToolCall::parse(
            ToolName::SearchCalls,
            args(json!({
                "q": " opus ",
                "client": "claude_code",
                "status": "errors",
                "has_tools": true,
                "from_ms": 10,
                "until_ms": 20,
                "provider": "",
            })),
        )
        .unwrap();
        let ToolCall::Read(ReadRequest::Calls(query)) = call else {
            panic!("not a search: {call:?}");
        };
        assert_eq!(query.limit.get(), 20);
        assert_eq!(query.filter.client, Some(Harness::ClaudeCode));
        assert_eq!(query.filter.status, Some(StatusFilter::Errors));
        assert_eq!(query.filter.tools, Some(ToolFilter::WithTools));
        assert_eq!(query.filter.provider, None);
        assert_eq!(
            query.filter.text.as_ref().map(SearchText::as_str),
            Some("opus")
        );
        assert_eq!(
            query.filter.time.from().map(UnixMillis::as_millis),
            Some(10)
        );
    }

    #[test]
    fn invalid_arguments_say_what_is_accepted() {
        let error = |tool, value| ToolCall::parse(tool, args(value)).unwrap_err().to_string();
        assert_eq!(
            error(ToolName::SearchCalls, json!({ "client": "vim" })),
            "client must be one of: claude_code, codex, pi"
        );
        assert!(
            error(ToolName::SearchCalls, json!({ "limit": 101 })).contains("between 1 and 100")
        );
        assert!(
            error(
                ToolName::SearchCalls,
                json!({ "from_ms": 2, "until_ms": 1 })
            )
            .contains("until_ms")
        );
        assert!(error(ToolName::SearchCalls, json!({ "sql": "x" })).contains("unknown field"));
        assert_eq!(
            error(ToolName::GetCall, json!({ "call_id": "nope" })),
            "call_id is not a TGV ID"
        );
        assert!(error(ToolName::GetCall, json!({})).contains("call_id"));
        assert!(
            error(
                ToolName::GetBody,
                json!({ "call_id": "00000000-0000-0000-0000-000000000001", "direction": "both" })
            )
            .contains("request, response")
        );
        assert!(
            error(
                ToolName::SessionCalls,
                json!({ "session_id": "s", "after": "x" })
            )
            .contains("TGV ID")
        );
    }

    #[test]
    fn definitions_list_the_vocabularies_they_accept() {
        let definitions = ToolName::definitions();
        let search = &definitions[1];
        assert_eq!(definitions[0]["name"], "search_history");
        assert_eq!(search["name"], "search_calls");
        assert_eq!(search["annotations"]["readOnlyHint"], true);
        assert_eq!(search["inputSchema"]["additionalProperties"], false);
        assert_eq!(
            search["inputSchema"]["properties"]["status"]["enum"],
            json!(["active", "finished", "errors", "interrupted"])
        );
        assert_eq!(
            definitions[9]["inputSchema"]["required"],
            json!(["call_id", "direction"])
        );
    }

    #[test]
    fn excerpts_cut_text_to_the_budget_and_keep_the_extent() {
        let preview = json!({
            "observed_bytes": 9, "stored_bytes": 9, "ending": "complete", "captured_bytes": 9,
            "extent": "complete",
            "raw": { "text": "68 c3 a9", "format": "hex", "truncated": false },
            "text": "héllo wörld", "text_end": "intact",
            "decoding": { "applied": "none", "integrity": "not_decoded", "truncated": false },
        });
        let excerpt = BodyExcerpt::new(preview.clone(), BodyBudget::new(Some(4)).unwrap())
            .unwrap()
            .into_value();
        assert_eq!(excerpt["text"], "héll");
        assert_eq!(excerpt["omitted_chars"], 7);
        assert_eq!(excerpt["extent"], "complete");
        assert!(excerpt.get("raw").is_none());

        let mut binary = preview;
        binary["text"] = Value::Null;
        binary["extent"] = json!("some_future_extent");
        let excerpt = BodyExcerpt::new(binary, BodyBudget::new(None).unwrap())
            .unwrap()
            .into_value();
        assert_eq!(excerpt["raw"], "68 c3 a9");
        assert_eq!(excerpt["raw_format"], "hex");
        assert_eq!(excerpt["extent"], "some_future_extent");
        assert!(BodyBudget::new(Some(0)).is_err());
        assert!(BodyExcerpt::new(json!({ "text": 1 }), BodyBudget::new(None).unwrap()).is_err());
    }
}
