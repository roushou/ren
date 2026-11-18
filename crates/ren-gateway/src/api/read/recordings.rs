//! Recordings as the read API sends them. Each type converts once from the type
//! it reports (`From`), naming every field.

use super::calls::TokenTotals;
use ren_harness::attribution::HintSource;
use ren_protocol::label::Stored;
use ren_protocol::route::Provider;
use ren_recorder::id::RecordingId;
use ren_recorder::time::UnixMillis;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A group of calls a client attributed to one recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RecordingView {
    #[ts(as = "String")]
    pub id: RecordingId,
    #[ts(as = "String")]
    pub provider: Stored<Provider>,
    #[ts(as = "String")]
    pub source: Stored<HintSource>,
    pub native_id: String,
    #[ts(as = "u64")]
    pub first_seen: UnixMillis,
    #[ts(as = "u64")]
    pub last_seen: UnixMillis,
    /// When the operator archived it, while it has no newer traffic.
    #[ts(as = "Option<u64>")]
    pub archived_at: Option<UnixMillis>,
    pub call_count: u64,
    pub turn_count: u64,
    /// Calls with an error status, a failed transport, or a failed model
    /// outcome: the same calls the `errors` status filter finds.
    pub error_count: u64,
    /// Provider-reported tokens summed over the recording's calls.
    pub tokens: TokenTotals,
}

impl From<ren_store::view::RecordingView> for RecordingView {
    fn from(value: ren_store::view::RecordingView) -> Self {
        let ren_store::view::RecordingView {
            id,
            provider,
            source,
            native_id,
            first_seen,
            last_seen,
            archived_at,
            call_count,
            turn_count,
            error_count,
            tokens,
        } = value;
        Self {
            id,
            provider,
            source,
            native_id,
            first_seen,
            last_seen,
            archived_at,
            call_count,
            turn_count,
            error_count,
            tokens: tokens.into(),
        }
    }
}
