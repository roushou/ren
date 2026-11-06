//! Query views: the typed rows the read API and CLI return.
//!
//! Labels are read as [`Stored`], so a value written by a newer binary is shown
//! verbatim instead of failing the query.

use crate::{StoreError, query::Cursor};
use serde::{Deserialize, Serialize};
use sqlx::{Row, sqlite::SqliteRow};
use std::str::FromStr;
use tgv_harness::attribution::{HintIssue, HintSource};
use tgv_protocol::{
    direction::Direction,
    encoding::{ResponseEncoding, ResponseFormat},
    inspect::ModelOutcome,
    label::{Labeled, Stored},
    route::{Endpoint, Method, Provider, Transport},
    websocket::Payload,
};
use tgv_recorder::Observation;
use tgv_recorder::batch::Recorded;
use tgv_recorder::inspection::{InspectionRecord, UsageRecord};
use tgv_recorder::routing::OperatorRevision;
use tgv_recorder::{
    capture::{
        AttemptPhase, AttemptTerminal, BodyEnding, CallRejection, CallState, ChannelEnd,
        TerminalCause,
    },
    id::{AttemptId, CallId, ChannelId, SessionId, TurnId},
    time::UnixMillis,
};

/// Typed column access with corruption reported as [`StoreError::Corrupt`].
pub(super) struct Columns<'r>(pub(super) &'r SqliteRow);

impl Columns<'_> {
    /// The observation log's columns, for [`Columns::observation`].
    pub(super) const OBSERVATION_SELECT: &'static str =
        "SELECT seq, at_wall, kind, channel, call, attempt FROM observations";

    pub(super) fn observation(&self) -> Result<Observation, StoreError> {
        Ok(Observation {
            seq: self.count("seq")?,
            at: self.time("at_wall")?,
            kind: self.label("kind")?,
            channel: self.id("channel")?,
            call: self.optional_id("call")?,
            attempt: self.optional_id("attempt")?,
        })
    }

    pub(super) fn id<T: FromStr>(&self, column: &str) -> Result<T, StoreError> {
        let text: String = self.0.try_get(column)?;
        text.parse().map_err(|_| StoreError::Corrupt("identifier"))
    }

    pub(super) fn optional_id<T: FromStr>(&self, column: &str) -> Result<Option<T>, StoreError> {
        let text: Option<String> = self.0.try_get(column)?;
        text.map(|text| text.parse().map_err(|_| StoreError::Corrupt("identifier")))
            .transpose()
    }

    pub(super) fn label<T: Labeled>(&self, column: &str) -> Result<Stored<T>, StoreError> {
        let text: String = self.0.try_get(column)?;
        Ok(Stored::from_label(text.as_str()))
    }

    pub(super) fn optional_label<T: Labeled>(
        &self,
        column: &str,
    ) -> Result<Option<Stored<T>>, StoreError> {
        let text: Option<String> = self.0.try_get(column)?;
        Ok(text.as_deref().map(Stored::from_label))
    }

    pub(super) fn count(&self, column: &str) -> Result<u64, StoreError> {
        Ok(self.0.try_get(column)?)
    }

    pub(super) fn optional_count(&self, column: &str) -> Result<Option<u64>, StoreError> {
        Ok(self.0.try_get(column)?)
    }

    pub(super) fn time(&self, column: &str) -> Result<UnixMillis, StoreError> {
        Ok(UnixMillis::from_millis(self.0.try_get(column)?))
    }

    pub(super) fn optional_time(&self, column: &str) -> Result<Option<UnixMillis>, StoreError> {
        let value: Option<u64> = self.0.try_get(column)?;
        Ok(value.map(UnixMillis::from_millis))
    }

    pub(super) fn text(&self, column: &str) -> Result<Option<String>, StoreError> {
        Ok(self.0.try_get(column)?)
    }

    pub(super) fn json<T: for<'de> Deserialize<'de>>(
        &self,
        column: &str,
    ) -> Result<Option<T>, StoreError> {
        let text: Option<String> = self.0.try_get(column)?;
        text.map(|text| serde_json::from_str(&text).map_err(StoreError::from))
            .transpose()
    }

    pub(super) fn flag(&self, column: &str) -> Result<bool, StoreError> {
        Ok(self.0.try_get(column)?)
    }
}

/// A body's capture accounting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BodyView {
    /// Bytes the relay saw; unknown until the body's end is recorded.
    pub observed_bytes: Option<u64>,
    /// Bytes committed to storage. Less than observed means bytes were omitted.
    pub stored_bytes: u64,
    pub ending: Stored<BodyEnding>,
}

impl BodyView {
    pub(super) fn from_columns(
        row: &Columns<'_>,
        prefix: &str,
    ) -> Result<Option<Self>, StoreError> {
        let ending: Option<String> = row.0.try_get(format!("{prefix}_ending").as_str())?;
        let Some(ending) = ending else {
            return Ok(None);
        };
        Ok(Some(Self {
            observed_bytes: row.0.try_get(format!("{prefix}_observed").as_str())?,
            stored_bytes: row.count(&format!("{prefix}_stored"))?,
            ending: Stored::from_label(ending.as_str()),
        }))
    }

    /// Every byte observed is stored and the end was seen.
    pub fn complete(&self) -> bool {
        self.ending.known() == Some(BodyEnding::Ended)
            && self.observed_bytes == Some(self.stored_bytes)
    }
}

/// Attribution as stored, keeping unknown labels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AttributionView {
    pub hint: Option<SessionHintView>,
    pub issues: Vec<Stored<HintIssue>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHintView {
    pub source: Stored<HintSource>,
    pub session: String,
    pub turn: Option<String>,
    pub thread: Option<String>,
}

/// One call, as listed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallView {
    pub id: CallId,
    pub channel: ChannelId,
    pub started_at: UnixMillis,
    pub ended_at: Option<UnixMillis>,
    pub provider: Stored<Provider>,
    pub endpoint: Stored<Endpoint>,
    pub method: Stored<Method>,
    pub config_revision: String,
    pub session: Option<SessionId>,
    pub turn: Option<TurnId>,
    pub attribution: AttributionView,
    pub state: Stored<CallState>,
    pub terminal: Option<Stored<AttemptTerminal>>,
    pub status: Option<u16>,
    pub model: Option<String>,
    pub outcome: Option<Stored<ModelOutcome>>,
    pub stop_reason: Option<String>,
    pub tool_names: Vec<String>,
    pub usage: Option<UsageRecord>,
    /// Both bodies ended and were stored without omissions.
    pub capture_complete: bool,
    pub request: Option<BodyView>,
    pub response: Option<BodyView>,
    /// Why TGV answered without forwarding; absent for a forwarded call.
    pub rejection: Option<Stored<CallRejection>>,
    /// The operator state revision the call was planned against.
    pub operator_revision: Option<u64>,
    /// The recorded routing plan, as written.
    pub plan: Option<serde_json::Value>,
}

impl CallView {
    /// Columns every call query selects, joined to the request body (`rq`) and
    /// the final attempt's response body (`rs`).
    pub(super) const SELECT: &'static str = "SELECT c.id, c.channel, c.started_at, c.ended_at, c.provider, c.endpoint, c.method, c.config_revision, c.session, c.turn, c.attribution, c.state, c.terminal, c.status, c.model, c.outcome, c.stop_reason, c.tool_names, c.usage, c.capture_complete, c.rejection, c.operator_revision, c.plan, rq.observed_bytes AS rq_observed, rq.stored_bytes AS rq_stored, rq.ending AS rq_ending, rs.observed_bytes AS rs_observed, rs.stored_bytes AS rs_stored, rs.ending AS rs_ending FROM calls c LEFT JOIN bodies rq ON rq.owner = c.id AND rq.direction = 'request' LEFT JOIN bodies rs ON rs.owner = c.final_attempt AND rs.direction = 'response'";

    pub(super) fn from_row(row: &SqliteRow) -> Result<Self, StoreError> {
        let row = Columns(row);
        Ok(Self {
            id: row.id("id")?,
            channel: row.id("channel")?,
            started_at: row.time("started_at")?,
            ended_at: row.optional_time("ended_at")?,
            provider: row.label("provider")?,
            endpoint: row.label("endpoint")?,
            method: row.label("method")?,
            config_revision: row.text("config_revision")?.unwrap_or_default(),
            session: row.optional_id("session")?,
            turn: row.optional_id("turn")?,
            attribution: row.json("attribution")?.unwrap_or_default(),
            state: row.label("state")?,
            terminal: row.optional_label("terminal")?,
            status: row.0.try_get("status")?,
            model: row.text("model")?,
            outcome: row.optional_label("outcome")?,
            stop_reason: row.text("stop_reason")?,
            tool_names: row.json("tool_names")?.unwrap_or_default(),
            usage: row.json("usage")?,
            capture_complete: row.flag("capture_complete")?,
            request: BodyView::from_columns(&row, "rq")?,
            response: BodyView::from_columns(&row, "rs")?,
            rejection: row.optional_label("rejection")?,
            operator_revision: row.0.try_get("operator_revision")?,
            plan: row.json("plan")?,
        })
    }

    pub fn cursor(&self) -> Cursor {
        Cursor {
            started_at: self.started_at,
            call: self.id,
        }
    }
}

/// One upstream attempt of a call.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AttemptView {
    pub id: AttemptId,
    pub ordinal: u32,
    pub started_at: UnixMillis,
    pub ended_at: Option<UnixMillis>,
    /// The upstream entry's id.
    pub upstream: String,
    pub status: Option<u16>,
    pub response_format: Option<Stored<ResponseFormat>>,
    pub response_encoding: Option<Stored<ResponseEncoding>>,
    pub terminal: Option<Stored<AttemptTerminal>>,
    /// Why it ended, where the terminal alone does not say.
    pub terminal_cause: Option<Stored<TerminalCause>>,
    /// How far it had got when it ended.
    pub terminal_phase: Option<Stored<AttemptPhase>>,
    pub response: Option<BodyView>,
    pub inspection_version: Option<u32>,
    pub inspection: Option<InspectionRecord>,
    /// What TGV changed in this attempt's request, as recorded.
    pub changes: Option<serde_json::Value>,
    /// The changed request body this attempt sent, if a transform changed it.
    pub request: Option<BodyView>,
}

impl AttemptView {
    pub(super) const SELECT: &'static str = "SELECT a.id, a.ordinal, a.started_at, a.ended_at, a.upstream, a.status, a.response_format, a.response_encoding, a.terminal, a.terminal_cause, a.terminal_phase, a.inspection, a.inspection_version, a.changes, rs.observed_bytes AS rs_observed, rs.stored_bytes AS rs_stored, rs.ending AS rs_ending, rq.observed_bytes AS rq_observed, rq.stored_bytes AS rq_stored, rq.ending AS rq_ending FROM attempts a LEFT JOIN bodies rs ON rs.owner = a.id AND rs.direction = 'response' LEFT JOIN bodies rq ON rq.owner = a.id AND rq.direction = 'request'";

    pub(super) fn from_row(row: &SqliteRow) -> Result<Self, StoreError> {
        let row = Columns(row);
        Ok(Self {
            id: row.id("id")?,
            ordinal: row.0.try_get("ordinal")?,
            started_at: row.time("started_at")?,
            ended_at: row.optional_time("ended_at")?,
            upstream: row.text("upstream")?.unwrap_or_default(),
            status: row.0.try_get("status")?,
            response_format: row.optional_label("response_format")?,
            response_encoding: row.optional_label("response_encoding")?,
            terminal: row.optional_label("terminal")?,
            terminal_cause: row.optional_label("terminal_cause")?,
            terminal_phase: row.optional_label("terminal_phase")?,
            response: BodyView::from_columns(&row, "rs")?,
            inspection_version: row.0.try_get("inspection_version")?,
            inspection: row.json("inspection")?,
            changes: row.json("changes")?,
            request: BodyView::from_columns(&row, "rq")?,
        })
    }
}

/// A call with its attempts and log.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallDetail {
    pub call: CallView,
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

/// One client connection or exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelView {
    pub id: ChannelId,
    pub transport: Stored<Transport>,
    pub opened_at: UnixMillis,
    /// When a WebSocket channel ended; absent while open, and for HTTP.
    pub closed_at: Option<UnixMillis>,
    pub ending: Option<Stored<ChannelEnd>>,
    pub close_code: Option<u16>,
    /// Messages forwarded while no call was open: counted, not stored.
    pub unattributed_messages: u64,
    pub unattributed_bytes: u64,
}

impl ChannelView {
    pub(super) const SELECT: &'static str = "SELECT id, transport, opened_at, closed_at, ending, close_code, unattributed_messages, unattributed_bytes FROM channels";

    pub(super) fn from_row(row: &SqliteRow) -> Result<Self, StoreError> {
        let columns = Columns(row);
        Ok(Self {
            id: columns.id("id")?,
            transport: columns.label("transport")?,
            opened_at: columns.time("opened_at")?,
            closed_at: columns.optional_time("closed_at")?,
            ending: columns.optional_label("ending")?,
            close_code: row.try_get("close_code")?,
            unattributed_messages: columns.count("unattributed_messages")?,
            unattributed_bytes: columns.count("unattributed_bytes")?,
        })
    }
}

/// Where one WebSocket message sits inside a body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageView {
    pub direction: Stored<Direction>,
    pub offset: u64,
    pub len: u64,
    pub payload: Stored<Payload>,
    pub at: UnixMillis,
}

impl MessageView {
    /// Rows for a call: `?1` is the call, `?2` the request direction, `?3`
    /// the response direction, `?4` the final attempt.
    pub(super) const SELECT_FOR_CALL: &'static str = "SELECT direction, byte_offset, len, payload, at_wall FROM messages WHERE (owner = ?1 AND direction = ?2) OR (owner = ?4 AND direction = ?3) ORDER BY direction, byte_offset LIMIT ?5";

    pub(super) fn from_row(row: &SqliteRow) -> Result<Self, StoreError> {
        let columns = Columns(row);
        Ok(Self {
            direction: columns.label("direction")?,
            offset: columns.count("byte_offset")?,
            len: columns.count("len")?,
            payload: columns.label("payload")?,
            at: columns.time("at_wall")?,
        })
    }
}

/// One page of calls.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallPage {
    pub calls: Vec<CallView>,
    /// Pass as `after` for the next page; absent on the last page.
    pub next: Option<Cursor>,
}

/// A group of calls a client attributed to one session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionView {
    pub id: SessionId,
    pub provider: Stored<Provider>,
    pub source: Stored<HintSource>,
    pub native_id: String,
    pub first_seen: UnixMillis,
    pub last_seen: UnixMillis,
    pub call_count: u64,
    pub turn_count: u64,
    /// Calls with an error status, a failed transport, or a failed model
    /// outcome: the same calls the `errors` status filter finds.
    pub error_count: u64,
    /// Provider-reported tokens summed over the session's calls.
    pub tokens: SessionTokens,
}

/// Token totals for a session; `None` when no call reported usage. Input
/// counts cached context too, whichever way each call's provider accounts
/// it, because cached context is most of a coding agent's input. Token
/// counts are not subscription allowance or charges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SessionTokens {
    pub input: Option<u64>,
    pub output: Option<u64>,
}

impl SessionView {
    pub(super) fn from_row(row: &SqliteRow) -> Result<Self, StoreError> {
        let row = Columns(row);
        Ok(Self {
            id: row.id("id")?,
            provider: row.label("provider")?,
            source: row.label("source")?,
            native_id: row.text("native_id")?.unwrap_or_default(),
            first_seen: row.time("first_seen")?,
            last_seen: row.time("last_seen")?,
            call_count: row.count("call_count")?,
            turn_count: row.count("turn_count")?,
            error_count: row.count("error_count")?,
            tokens: SessionTokens {
                input: row.optional_count("input_tokens")?,
                output: row.optional_count("output_tokens")?,
            },
        })
    }
}

/// One recorded operator command, as the gateway serialized it.
#[derive(Debug, Clone, Serialize)]
pub struct OperatorActionView {
    pub seq: u64,
    pub at: UnixMillis,
    /// The operator state revision the command made.
    pub revision: OperatorRevision,
    /// Who issued it: the recorder's `Actor` document.
    pub actor: Recorded,
    /// The gateway's command document, never interpreted by the store.
    pub command: Recorded,
}

impl OperatorActionView {
    pub(super) fn from_row(row: &SqliteRow) -> Result<Self, StoreError> {
        let row = Columns(row);
        let document = |column| {
            row.text(column)?
                .ok_or(StoreError::Corrupt("operator action"))
                .and_then(|text| {
                    Recorded::restore(text).map_err(|_| StoreError::Corrupt("operator action"))
                })
        };
        Ok(Self {
            seq: row.count("seq")?,
            at: row.time("at_wall")?,
            revision: OperatorRevision::restore(row.count("revision")?),
            actor: document("actor")?,
            command: document("command")?,
        })
    }
}

/// A bounded, contiguous prefix of a body, for previews.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyPrefix {
    pub body: BodyView,
    pub bytes: Vec<u8>,
    /// Stored bytes are missing before the end of the prefix.
    pub gap: bool,
    /// The body continues past the prefix limit.
    pub truncated: bool,
    /// The `Content-Encoding` its attempt recorded; `None` for request bodies.
    pub encoding: Option<Stored<ResponseEncoding>>,
}

/// Real disk usage of one database file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FileUsage {
    pub page_size: u64,
    pub page_count: u64,
    pub freelist_count: u64,
}

impl FileUsage {
    pub fn live_bytes(self) -> u64 {
        self.page_count
            .saturating_sub(self.freelist_count)
            .saturating_mul(self.page_size)
    }

    pub fn free_bytes(self) -> u64 {
        self.freelist_count.saturating_mul(self.page_size)
    }
}

/// Disk usage of both files, as retention measures it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DiskUsage {
    pub metadata: FileUsage,
    pub blobs: FileUsage,
}

impl DiskUsage {
    pub fn live_bytes(self) -> u64 {
        self.metadata
            .live_bytes()
            .saturating_add(self.blobs.live_bytes())
    }
}

/// What is retained, and how well chunk deduplication works on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StorageView {
    pub disk: DiskUsage,
    pub calls: u64,
    pub open_calls: u64,
    pub chunks: u64,
    /// Raw bytes all stored segments reference.
    pub referenced_bytes: u64,
    /// Raw bytes of the distinct chunks.
    pub unique_bytes: u64,
    /// Bytes the chunks occupy after compression.
    pub compressed_bytes: u64,
}
