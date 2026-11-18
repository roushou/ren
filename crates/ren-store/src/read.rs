//! Every history read as one typed request, served by [`Reader::serve`]. The
//! HTTP API, the CLI on this host, and the MCP server all ask through it, so
//! they answer with the same values: a response serializes to the API's
//! JSON body, and [`ReadRequest::decode`] reads that body back, so a client
//! of the API gets the same typed values the reader returns.

use crate::activity::report::RecordingReport;
use crate::activity::{ActivityBudget, CallActivity, TurnActivity, TurnSummary};
use crate::query::{CallQuery, Cursor, PageLimit, Shelf, TimeRange};
use crate::search::{ExposureQuery, ExposureView, SearchPage, SearchQuery};
use crate::view::{CallDetail, CallPage, ConversationView, RecordingView, RunView, UsageView};
use crate::{Reader, StoreError};
use ren_recorder::id::{CallId, RecordingId, RunId, TurnId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A read of recorded history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadRequest {
    /// Calls, newest first, narrowed by the query's filters.
    Calls(CallQuery),
    Call(CallId),
    /// Recordings, most recently active first.
    Recordings {
        limit: PageLimit,
        shelf: Shelf,
    },
    /// A recording's calls, oldest first, a page at a time.
    RecordingCalls {
        recording: RecordingId,
        limit: PageLimit,
        after: Option<Cursor>,
    },
    Turns {
        recording: RecordingId,
        limit: PageLimit,
    },
    Turn(TurnId),
    CallActivity(CallId),
    Report(RecordingId),
    Usage(TimeRange),
    Search(SearchQuery),
    Exposures(ExposureQuery),
    /// Agent runs, newest first.
    Runs {
        limit: PageLimit,
        shelf: Shelf,
    },
    Run(RunId),
    /// A run's conversation, whole, with the runs before it on its thread.
    Conversation(RunId),
}

/// What a [`ReadRequest`] returns. Serialized, each is the API's JSON body
/// for that read (before the API adds its version).
///
/// ts-rs writes it and every type it holds to the UI
/// (`ui/apps/web/src/lib/wire`). Values from other crates are written as
/// their wire form at each field (`#[ts(as = …)]`): ids, labels, and cursors
/// as text, times as numbers, and the recorder's and harness's structured
/// values as JSON the UI decodes itself.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum ReadResponse {
    Calls(CallPage),
    Call(Box<CallDetail>),
    /// Serialized as the read API's `recordings`.
    Recordings {
        recordings: Vec<RecordingView>,
    },
    Turns {
        turns: Vec<TurnSummary>,
    },
    Turn(Box<TurnActivity>),
    CallActivity(Box<CallActivity>),
    Report(Box<RecordingReport>),
    Usage(UsageView),
    Search(SearchPage),
    Exposures {
        exposures: Vec<ExposureView>,
    },
    Runs {
        runs: Vec<RunView>,
    },
    Run(Box<RunView>),
    Conversation(ConversationView),
}

/// The read API's body for a recording listing.
#[derive(Deserialize)]
struct Recordings {
    recordings: Vec<RecordingView>,
}

#[derive(Deserialize)]
struct Turns {
    turns: Vec<TurnSummary>,
}

#[derive(Deserialize)]
struct Exposures {
    exposures: Vec<ExposureView>,
}

#[derive(Deserialize)]
struct Runs {
    runs: Vec<RunView>,
}

impl ReadRequest {
    /// The API's JSON body for this read, parsed into what the read returns.
    /// Fields the body has beyond the response (its `api_version`) are
    /// ignored.
    pub fn decode(&self, body: serde_json::Value) -> Result<ReadResponse, serde_json::Error> {
        Ok(match self {
            Self::Calls(_) | Self::RecordingCalls { .. } => {
                ReadResponse::Calls(serde_json::from_value(body)?)
            }
            Self::Call(_) => ReadResponse::Call(serde_json::from_value(body)?),
            Self::Recordings { .. } => ReadResponse::Recordings {
                recordings: serde_json::from_value::<Recordings>(body)?.recordings,
            },
            Self::Turns { .. } => ReadResponse::Turns {
                turns: serde_json::from_value::<Turns>(body)?.turns,
            },
            Self::Turn(_) => ReadResponse::Turn(serde_json::from_value(body)?),
            Self::CallActivity(_) => ReadResponse::CallActivity(serde_json::from_value(body)?),
            Self::Report(_) => ReadResponse::Report(serde_json::from_value(body)?),
            Self::Usage(_) => ReadResponse::Usage(serde_json::from_value(body)?),
            Self::Search(_) => ReadResponse::Search(serde_json::from_value(body)?),
            Self::Exposures(_) => ReadResponse::Exposures {
                exposures: serde_json::from_value::<Exposures>(body)?.exposures,
            },
            Self::Runs { .. } => ReadResponse::Runs {
                runs: serde_json::from_value::<Runs>(body)?.runs,
            },
            Self::Run(_) => ReadResponse::Run(serde_json::from_value(body)?),
            Self::Conversation(_) => ReadResponse::Conversation(serde_json::from_value(body)?),
        })
    }
}

impl Reader {
    /// Entries a conversation read returns at most: long runs stay whole.
    pub const CONVERSATION: u32 = 20_000;

    /// Answer one read.
    pub async fn serve(&self, request: &ReadRequest) -> Result<ReadResponse, StoreError> {
        Ok(match request {
            ReadRequest::Calls(query) => ReadResponse::Calls(self.calls(query).await?),
            ReadRequest::Call(call) => ReadResponse::Call(Box::new(self.call(*call).await?)),
            ReadRequest::Recordings { limit, shelf } => ReadResponse::Recordings {
                recordings: self.recordings(*limit, *shelf).await?,
            },
            ReadRequest::RecordingCalls {
                recording,
                limit,
                after,
            } => ReadResponse::Calls(self.recording_calls(*recording, *limit, *after).await?),
            ReadRequest::Turns { recording, limit } => ReadResponse::Turns {
                turns: self.recording_turns(*recording, *limit).await?,
            },
            ReadRequest::Turn(turn) => ReadResponse::Turn(Box::new(
                self.turn_activity(*turn, ActivityBudget::DEFAULT).await?,
            )),
            ReadRequest::CallActivity(call) => ReadResponse::CallActivity(Box::new(
                self.call_activity(*call, ActivityBudget::DEFAULT).await?,
            )),
            ReadRequest::Report(recording) => {
                ReadResponse::Report(Box::new(self.recording_report(*recording).await?))
            }
            ReadRequest::Usage(range) => ReadResponse::Usage(self.usage(*range).await?),
            ReadRequest::Search(query) => ReadResponse::Search(self.search(query).await?),
            ReadRequest::Exposures(query) => ReadResponse::Exposures {
                exposures: self.exposures(query).await?,
            },
            ReadRequest::Runs { limit, shelf } => ReadResponse::Runs {
                runs: self.runs(*limit, *shelf).await?,
            },
            ReadRequest::Run(run) => {
                ReadResponse::Run(Box::new(self.run(*run).await?.ok_or(StoreError::NotFound)?))
            }
            ReadRequest::Conversation(run) => ReadResponse::Conversation(
                self.conversation(*run, Self::CONVERSATION)
                    .await?
                    .ok_or(StoreError::NotFound)?,
            ),
        })
    }
}
