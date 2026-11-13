//! Every history read as one typed request, served by [`Reader::serve`]. The
//! HTTP API, the CLI on this host, and the MCP server all ask through it, so
//! they answer with the same values: a response serializes to the API's
//! JSON body.

use crate::activity::report::RecordingReport;
use crate::activity::{ActivityBudget, CallActivity, TurnActivity, TurnSummary};
use crate::query::{CallQuery, Cursor, PageLimit, Shelf, TimeRange};
use crate::search::{ExposureQuery, ExposureView, SearchPage, SearchQuery};
use crate::view::{CallDetail, CallPage, ConversationView, RecordingView, RunView, UsageView};
use crate::{Reader, StoreError};
use ren_recorder::id::{CallId, RecordingId, RunId, TurnId};
use serde::Serialize;

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
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ReadResponse {
    Calls(CallPage),
    Call(Box<CallDetail>),
    /// Serialized as the read API's `sessions`.
    Recordings {
        sessions: Vec<RecordingView>,
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

impl Reader {
    /// Entries a conversation read returns at most: long runs stay whole.
    pub const CONVERSATION: u32 = 20_000;

    /// Answer one read.
    pub async fn serve(&self, request: &ReadRequest) -> Result<ReadResponse, StoreError> {
        Ok(match request {
            ReadRequest::Calls(query) => ReadResponse::Calls(self.calls(query).await?),
            ReadRequest::Call(call) => ReadResponse::Call(Box::new(self.call(*call).await?)),
            ReadRequest::Recordings { limit, shelf } => ReadResponse::Recordings {
                sessions: self.recordings(*limit, *shelf).await?,
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
