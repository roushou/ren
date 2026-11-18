//! Search results and exposed secrets, as the read API sends them. Each type
//! converts once from the type it reports (`From`), naming every field.

use ren_harness::Harness;
use ren_protocol::label::Stored;
use ren_recorder::id::CallId;
use ren_recorder::id::RecordingId;
use ren_recorder::id::TurnId;
use ren_recorder::secrets::SecretKind;
use ren_recorder::time::UnixMillis;
use ren_store::search::SearchField;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A secret one call carried, as listed: never the secret itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExposureView {
    #[ts(as = "String")]
    pub call: CallId,
    #[ts(as = "Option<String>")]
    pub recording: Option<RecordingId>,
    #[ts(as = "Option<String>")]
    pub turn: Option<TurnId>,
    #[ts(as = "Option<String>")]
    pub client: Option<Stored<Harness>>,
    #[ts(as = "u64")]
    pub started_at: UnixMillis,
    #[ts(as = "String")]
    pub field: Stored<SearchField>,
    #[ts(as = "String")]
    pub kind: Stored<SecretKind>,
    pub hint: String,
    pub fingerprint: String,
    pub count: u64,
}

impl From<ren_store::search::ExposureView> for ExposureView {
    fn from(value: ren_store::search::ExposureView) -> Self {
        let ren_store::search::ExposureView {
            call,
            recording,
            turn,
            client,
            started_at,
            field,
            kind,
            hint,
            fingerprint,
            count,
        } = value;
        Self {
            call,
            recording,
            turn,
            client,
            started_at,
            field,
            kind,
            hint,
            fingerprint,
            count,
        }
    }
}

/// One matching document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchHit {
    #[ts(as = "String")]
    pub call: CallId,
    #[ts(as = "Option<String>")]
    pub recording: Option<RecordingId>,
    #[ts(as = "Option<String>")]
    pub turn: Option<TurnId>,
    #[ts(as = "Option<String>")]
    pub client: Option<Stored<Harness>>,
    #[ts(as = "u64")]
    pub started_at: UnixMillis,
    #[ts(as = "String")]
    pub field: Stored<SearchField>,
    /// The matching text around the match, terms marked «like this».
    pub snippet: String,
}

impl From<ren_store::search::SearchHit> for SearchHit {
    fn from(value: ren_store::search::SearchHit) -> Self {
        let ren_store::search::SearchHit {
            call,
            recording,
            turn,
            client,
            started_at,
            field,
            snippet,
        } = value;
        Self {
            call,
            recording,
            turn,
            client,
            started_at,
            field,
            snippet,
        }
    }
}

/// Search results, and how much history the index covers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    /// Calls indexed: every finished call whose digest is derived.
    pub indexed_calls: u64,
}

impl From<ren_store::search::SearchPage> for SearchPage {
    fn from(value: ren_store::search::SearchPage) -> Self {
        let ren_store::search::SearchPage {
            hits,
            indexed_calls,
        } = value;
        Self {
            hits: hits.into_iter().map(Into::into).collect(),
            indexed_calls,
        }
    }
}
