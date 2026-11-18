//! Calls as the read API sends them: a page of calls, one call's detail with
//! its attempts, channel, bodies, and observations, and what inspection read
//! from it. Each type converts once from the type it reports (`From`), naming
//! every field.

use ren_harness::attribution::HintIssue;
use ren_harness::attribution::HintSource;
use ren_harness::tools::CallPurpose;
use ren_protocol::direction::Direction;
use ren_protocol::encoding::ResponseEncoding;
use ren_protocol::encoding::ResponseFormat;
use ren_protocol::inspect::Coverage;
use ren_protocol::inspect::FormatSource;
use ren_protocol::inspect::InputAccounting;
use ren_protocol::inspect::KindFamily;
use ren_protocol::inspect::Limitation;
use ren_protocol::inspect::ModelOutcome;
use ren_protocol::inspect::TerminalEvent;
use ren_protocol::label::Stored;
use ren_protocol::route::Endpoint;
use ren_protocol::route::Method;
use ren_protocol::route::Provider;
use ren_protocol::route::Transport;
use ren_protocol::route::Wire;
use ren_protocol::websocket::Payload;
use ren_recorder::batch::ObservationKind;
use ren_recorder::caller::CallerKind;
use ren_recorder::capture::AttemptPhase;
use ren_recorder::capture::AttemptTerminal;
use ren_recorder::capture::BodyEnding;
use ren_recorder::capture::CallRejection;
use ren_recorder::capture::CallState;
use ren_recorder::capture::ChannelEnd;
use ren_recorder::capture::TerminalCause;
use ren_recorder::id::AttemptId;
use ren_recorder::id::CallId;
use ren_recorder::id::ChannelId;
use ren_recorder::id::RecordingId;
use ren_recorder::id::TurnId;
use ren_recorder::time::UnixMillis;
use ren_store::query::Cursor;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A body's capture accounting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BodyView {
    /// Bytes the relay saw; unknown until the body's end is recorded.
    pub observed_bytes: Option<u64>,
    /// Bytes committed to storage. Less than observed means bytes were omitted.
    pub stored_bytes: u64,
    #[ts(as = "String")]
    pub ending: Stored<BodyEnding>,
}

impl From<ren_store::view::BodyView> for BodyView {
    fn from(value: ren_store::view::BodyView) -> Self {
        let ren_store::view::BodyView {
            observed_bytes,
            stored_bytes,
            ending,
        } = value;
        Self {
            observed_bytes,
            stored_bytes,
            ending,
        }
    }
}

/// Attribution as stored, keeping unknown labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AttributionView {
    pub hint: Option<SessionHintView>,
    #[ts(as = "Vec<String>")]
    pub issues: Vec<Stored<HintIssue>>,
}

impl From<ren_store::view::AttributionView> for AttributionView {
    fn from(value: ren_store::view::AttributionView) -> Self {
        let ren_store::view::AttributionView { hint, issues } = value;
        Self {
            hint: hint.map(Into::into),
            issues,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionHintView {
    #[ts(as = "String")]
    pub source: Stored<HintSource>,
    pub session: String,
    pub turn: Option<String>,
    pub thread: Option<String>,
}

impl From<ren_store::view::SessionHintView> for SessionHintView {
    fn from(value: ren_store::view::SessionHintView) -> Self {
        let ren_store::view::SessionHintView {
            source,
            session,
            turn,
            thread,
        } = value;
        Self {
            source,
            session,
            turn,
            thread,
        }
    }
}

/// Who made a call, as recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CallerView {
    #[ts(as = "String")]
    pub kind: Stored<CallerKind>,
    /// The named user it proved to be.
    pub user: Option<String>,
    /// The Tailscale login the request carried.
    pub login: Option<String>,
}

impl From<ren_store::view::CallerView> for CallerView {
    fn from(value: ren_store::view::CallerView) -> Self {
        let ren_store::view::CallerView { kind, user, login } = value;
        Self { kind, user, login }
    }
}

/// One call, as listed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CallView {
    #[ts(as = "String")]
    pub id: CallId,
    #[ts(as = "String")]
    pub channel: ChannelId,
    #[ts(as = "u64")]
    pub started_at: UnixMillis,
    #[ts(as = "Option<u64>")]
    pub ended_at: Option<UnixMillis>,
    #[ts(as = "String")]
    pub provider: Stored<Provider>,
    /// The wire format.
    #[ts(as = "String")]
    pub wire: Stored<Wire>,
    #[ts(as = "String")]
    pub endpoint: Stored<Endpoint>,
    #[ts(as = "String")]
    pub method: Stored<Method>,
    pub config_revision: String,
    #[ts(as = "Option<String>")]
    pub recording: Option<RecordingId>,
    #[ts(as = "Option<String>")]
    pub turn: Option<TurnId>,
    pub attribution: AttributionView,
    /// Who made the call.
    pub caller: CallerView,
    #[ts(as = "String")]
    pub state: Stored<CallState>,
    #[ts(as = "Option<String>")]
    pub terminal: Option<Stored<AttemptTerminal>>,
    pub status: Option<u16>,
    pub model: Option<String>,
    #[ts(as = "Option<String>")]
    pub outcome: Option<Stored<ModelOutcome>>,
    pub stop_reason: Option<String>,
    pub tool_names: Vec<String>,
    pub usage: Option<UsageRecord>,
    /// Both bodies ended and were stored without omissions.
    pub capture_complete: bool,
    pub request: Option<BodyView>,
    pub response: Option<BodyView>,
    /// Why Ren answered without forwarding; absent for a forwarded call.
    #[ts(as = "Option<String>")]
    pub rejection: Option<Stored<CallRejection>>,
    /// The operator state revision the call was planned against.
    pub operator_revision: Option<u64>,
    /// The recorded routing plan, as written.
    pub plan: Option<serde_json::Value>,
    /// Why the client sent it (work, or a check of its own such as Claude
    /// Code's quota probe), from the call's digest; absent until derived.
    #[ts(as = "Option<String>")]
    pub purpose: Option<Stored<CallPurpose>>,
}

impl From<ren_store::view::CallView> for CallView {
    fn from(value: ren_store::view::CallView) -> Self {
        let ren_store::view::CallView {
            id,
            channel,
            started_at,
            ended_at,
            provider,
            wire,
            endpoint,
            method,
            config_revision,
            recording,
            turn,
            attribution,
            caller,
            state,
            terminal,
            status,
            model,
            outcome,
            stop_reason,
            tool_names,
            usage,
            capture_complete,
            request,
            response,
            rejection,
            operator_revision,
            plan,
            purpose,
        } = value;
        Self {
            id,
            channel,
            started_at,
            ended_at,
            provider,
            wire,
            endpoint,
            method,
            config_revision,
            recording,
            turn,
            attribution: attribution.into(),
            caller: caller.into(),
            state,
            terminal,
            status,
            model,
            outcome,
            stop_reason,
            tool_names,
            usage: usage.map(Into::into),
            capture_complete,
            request: request.map(Into::into),
            response: response.map(Into::into),
            rejection,
            operator_revision,
            plan,
            purpose,
        }
    }
}

/// One upstream attempt of a call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AttemptView {
    #[ts(as = "String")]
    pub id: AttemptId,
    pub ordinal: u32,
    #[ts(as = "u64")]
    pub started_at: UnixMillis,
    #[ts(as = "Option<u64>")]
    pub ended_at: Option<UnixMillis>,
    /// The upstream entry's id.
    pub upstream: String,
    /// The upstream entry's provider.
    #[ts(as = "String")]
    pub provider: Stored<Provider>,
    pub status: Option<u16>,
    #[ts(as = "Option<String>")]
    pub response_format: Option<Stored<ResponseFormat>>,
    #[ts(as = "Option<String>")]
    pub response_encoding: Option<Stored<ResponseEncoding>>,
    #[ts(as = "Option<String>")]
    pub terminal: Option<Stored<AttemptTerminal>>,
    /// Why it ended, where the terminal alone does not say.
    #[ts(as = "Option<String>")]
    pub terminal_cause: Option<Stored<TerminalCause>>,
    /// How far it had got when it ended.
    #[ts(as = "Option<String>")]
    pub terminal_phase: Option<Stored<AttemptPhase>>,
    pub response: Option<BodyView>,
    pub inspection_version: Option<u32>,
    pub inspection: Option<InspectionRecord>,
    /// What Ren changed in this attempt's request, as recorded.
    pub changes: Option<serde_json::Value>,
    /// The changed request body this attempt sent, if a transform changed it.
    pub request: Option<BodyView>,
}

impl From<ren_store::view::AttemptView> for AttemptView {
    fn from(value: ren_store::view::AttemptView) -> Self {
        let ren_store::view::AttemptView {
            id,
            ordinal,
            started_at,
            ended_at,
            upstream,
            provider,
            status,
            response_format,
            response_encoding,
            terminal,
            terminal_cause,
            terminal_phase,
            response,
            inspection_version,
            inspection,
            changes,
            request,
        } = value;
        Self {
            id,
            ordinal,
            started_at,
            ended_at,
            upstream,
            provider,
            status,
            response_format,
            response_encoding,
            terminal,
            terminal_cause,
            terminal_phase,
            response: response.map(Into::into),
            inspection_version,
            inspection: inspection.map(Into::into),
            changes,
            request: request.map(Into::into),
        }
    }
}

/// A call with its attempts and log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CallDetail {
    pub call: CallView,
    #[ts(as = "String")]
    pub transport: Stored<Transport>,
    /// The channel that carried the call: for WebSocket, the upgraded socket
    /// shared with the channel's other calls.
    pub channel: ChannelView,
    pub attempts: Vec<AttemptView>,
    pub observations: Vec<Observation>,
    /// WebSocket message boundaries in the call's request body and its final
    /// attempt's response body, in body order. Empty for HTTP.
    pub messages: Vec<MessageView>,
}

impl From<ren_store::view::CallDetail> for CallDetail {
    fn from(value: ren_store::view::CallDetail) -> Self {
        let ren_store::view::CallDetail {
            call,
            transport,
            channel,
            attempts,
            observations,
            messages,
        } = value;
        Self {
            call: call.into(),
            transport,
            channel: channel.into(),
            attempts: attempts.into_iter().map(Into::into).collect(),
            observations: observations.into_iter().map(Into::into).collect(),
            messages: messages.into_iter().map(Into::into).collect(),
        }
    }
}

/// One client connection or exchange.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChannelView {
    #[ts(as = "String")]
    pub id: ChannelId,
    #[ts(as = "String")]
    pub transport: Stored<Transport>,
    #[ts(as = "u64")]
    pub opened_at: UnixMillis,
    /// When a WebSocket channel ended; absent while open, and for HTTP.
    #[ts(as = "Option<u64>")]
    pub closed_at: Option<UnixMillis>,
    #[ts(as = "Option<String>")]
    pub ending: Option<Stored<ChannelEnd>>,
    pub close_code: Option<u16>,
    /// Messages forwarded while no call was open: counted, not stored.
    pub unattributed_messages: u64,
    pub unattributed_bytes: u64,
}

impl From<ren_store::view::ChannelView> for ChannelView {
    fn from(value: ren_store::view::ChannelView) -> Self {
        let ren_store::view::ChannelView {
            id,
            transport,
            opened_at,
            closed_at,
            ending,
            close_code,
            unattributed_messages,
            unattributed_bytes,
        } = value;
        Self {
            id,
            transport,
            opened_at,
            closed_at,
            ending,
            close_code,
            unattributed_messages,
            unattributed_bytes,
        }
    }
}

/// Where one WebSocket message sits inside a body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MessageView {
    #[ts(as = "String")]
    pub direction: Stored<Direction>,
    pub offset: u64,
    pub len: u64,
    #[ts(as = "String")]
    pub payload: Stored<Payload>,
    #[ts(as = "u64")]
    pub at: UnixMillis,
}

impl From<ren_store::view::MessageView> for MessageView {
    fn from(value: ren_store::view::MessageView) -> Self {
        let ren_store::view::MessageView {
            direction,
            offset,
            len,
            payload,
            at,
        } = value;
        Self {
            direction,
            offset,
            len,
            payload,
            at,
        }
    }
}

/// One page of calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CallPage {
    pub calls: Vec<CallView>,
    /// Pass as `after` for the next page; absent on the last page.
    #[ts(as = "Option<String>")]
    pub next: Option<Cursor>,
}

impl From<ren_store::view::CallPage> for CallPage {
    fn from(value: ren_store::view::CallPage) -> Self {
        let ren_store::view::CallPage { calls, next } = value;
        Self {
            calls: calls.into_iter().map(Into::into).collect(),
            next,
        }
    }
}

/// Provider-reported tokens summed over calls; each is `None` when no call
/// reported it. Input counts cached context too, whichever way each call's
/// provider accounts it, because cached context is most of a coding agent's
/// input; `cache_read` is the part of it read from the provider's cache.
/// Token counts are not subscription allowance or charges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TokenTotals {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
}

impl From<ren_store::view::TokenTotals> for TokenTotals {
    fn from(value: ren_store::view::TokenTotals) -> Self {
        let ren_store::view::TokenTotals {
            input,
            output,
            cache_read,
        } = value;
        Self {
            input,
            output,
            cache_read,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InspectionRecord {
    #[ts(as = "String")]
    pub coverage: Stored<Coverage>,
    #[ts(as = "String")]
    pub format: Stored<FormatSource>,
    #[ts(as = "Vec<String>")]
    pub limitations: Vec<Stored<Limitation>>,
    #[ts(as = "Option<String>")]
    pub outcome: Option<Stored<ModelOutcome>>,
    #[ts(as = "String")]
    pub terminal_event: Stored<TerminalEvent>,
    pub model: Option<String>,
    pub stop_reason: Option<String>,
    pub usage: Option<UsageRecord>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub unknown_kinds: Vec<UnknownKindRecord>,
    pub events: EventCountsRecord,
}

impl From<ren_recorder::inspection::InspectionRecord> for InspectionRecord {
    fn from(value: ren_recorder::inspection::InspectionRecord) -> Self {
        let ren_recorder::inspection::InspectionRecord {
            coverage,
            format,
            limitations,
            outcome,
            terminal_event,
            model,
            stop_reason,
            usage,
            tool_calls,
            unknown_kinds,
            events,
        } = value;
        Self {
            coverage,
            format,
            limitations,
            outcome,
            terminal_event,
            model,
            stop_reason,
            usage: usage.map(Into::into),
            tool_calls: tool_calls.into_iter().map(Into::into).collect(),
            unknown_kinds: unknown_kinds.into_iter().map(Into::into).collect(),
            events: events.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UsageRecord {
    #[ts(as = "String")]
    pub accounting: Stored<InputAccounting>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

impl From<ren_recorder::inspection::UsageRecord> for UsageRecord {
    fn from(value: ren_recorder::inspection::UsageRecord) -> Self {
        let ren_recorder::inspection::UsageRecord {
            accounting,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            reasoning_tokens,
        } = value;
        Self {
            accounting,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            reasoning_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolCallRecord {
    pub call_id: Option<String>,
    pub item_id: Option<String>,
    pub kind: String,
    pub name: Option<String>,
}

impl From<ren_recorder::inspection::ToolCallRecord> for ToolCallRecord {
    fn from(value: ren_recorder::inspection::ToolCallRecord) -> Self {
        let ren_recorder::inspection::ToolCallRecord {
            call_id,
            item_id,
            kind,
            name,
        } = value;
        Self {
            call_id,
            item_id,
            kind,
            name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UnknownKindRecord {
    #[ts(as = "String")]
    pub family: Stored<KindFamily>,
    pub kind: String,
    pub count: u64,
}

impl From<ren_recorder::inspection::UnknownKindRecord> for UnknownKindRecord {
    fn from(value: ren_recorder::inspection::UnknownKindRecord) -> Self {
        let ren_recorder::inspection::UnknownKindRecord {
            family,
            kind,
            count,
        } = value;
        Self {
            family,
            kind,
            count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EventCountsRecord {
    pub understood: u64,
    pub unknown: u64,
    pub malformed: u64,
}

impl From<ren_recorder::inspection::EventCountsRecord> for EventCountsRecord {
    fn from(value: ren_recorder::inspection::EventCountsRecord) -> Self {
        let ren_recorder::inspection::EventCountsRecord {
            understood,
            unknown,
            malformed,
        } = value;
        Self {
            understood,
            unknown,
            malformed,
        }
    }
}

/// One committed row of the observation log, as published to live
/// subscribers and read back by the store, and as `/api/live` clients read
/// it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Observation {
    pub seq: u64,
    #[ts(as = "u64")]
    pub at: UnixMillis,
    #[ts(as = "String")]
    pub kind: Stored<ObservationKind>,
    #[ts(as = "String")]
    pub channel: ChannelId,
    #[ts(as = "Option<String>")]
    pub call: Option<CallId>,
    #[ts(as = "Option<String>")]
    pub attempt: Option<AttemptId>,
}

impl From<ren_recorder::Observation> for Observation {
    fn from(value: ren_recorder::Observation) -> Self {
        let ren_recorder::Observation {
            seq,
            at,
            kind,
            channel,
            call,
            attempt,
        } = value;
        Self {
            seq,
            at,
            kind,
            channel,
            call,
            attempt,
        }
    }
}
