//! Every history read as one typed request, served by [`Reader::serve`]. The
//! HTTP API, the CLI on this host, and the MCP server all ask through it; the
//! gateway converts each response into the read API's body
//! (`ren_gateway::api::read`).

use crate::activity::report::RecordingReport;
use crate::activity::{ActivityBudget, CallActivity, TurnActivity, TurnSummary};
use crate::query::{CallQuery, Cursor, ItemBodies, PageLimit, Shelf, TimeRange, UsageScope};
use crate::search::{ExposureQuery, ExposureView, SearchPage, SearchQuery};
use crate::view::{
    CallDetail, CallPage, ConversationView, PassageView, RecordingView, RunView, UsageView,
    WorkItemView,
};
use crate::{Reader, StoreError};
use ren_recorder::id::{CallId, RecordingId, RunId, SessionId, TurnId, WorkItemId};
use ren_recorder::project::ProjectName;

/// A read of recorded history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadRequest {
    /// Calls, newest first, narrowed by the query's filters.
    Calls(CallQuery),
    Call(CallId),
    /// Recordings, most recently active first; with a project, its
    /// sessions' runs' only.
    Recordings {
        limit: PageLimit,
        shelf: Shelf,
        project: Option<ProjectName>,
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
    /// Usage of the calls started in `range`, within `scope`.
    Usage {
        range: TimeRange,
        scope: UsageScope,
    },
    Search(SearchQuery),
    Exposures(ExposureQuery),
    /// Agent runs, newest first; with a project, its sessions' only.
    Runs {
        limit: PageLimit,
        shelf: Shelf,
        project: Option<ProjectName>,
    },
    Run(RunId),
    /// A run's conversation, whole, with the runs before it on its thread,
    /// its items' bodies kept or left out.
    Conversation {
        run: RunId,
        bodies: ItemBodies,
    },
    /// Where a relayed run reached through the relay, grouped by what it
    /// asked for and how that ended.
    Passages(SessionId),
    /// Work items, most recently changed first; with a project, its only.
    WorkItems {
        limit: PageLimit,
        shelf: Shelf,
        project: Option<ProjectName>,
    },
    WorkItem(WorkItemId),
}

/// What a [`ReadRequest`] returns.
#[derive(Debug, Clone, PartialEq)]
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
    Passages {
        passages: Vec<PassageView>,
    },
    /// Serialized as the read API's `work`.
    WorkItems {
        work: Vec<WorkItemView>,
    },
    WorkItem(Box<WorkItemView>),
}

impl Reader {
    /// Entries a conversation read returns at most: long runs stay whole.
    pub const CONVERSATION: u32 = 20_000;

    /// Answer one read.
    pub async fn serve(&self, request: &ReadRequest) -> Result<ReadResponse, StoreError> {
        Ok(match request {
            ReadRequest::Calls(query) => ReadResponse::Calls(self.calls(query).await?),
            ReadRequest::Call(call) => ReadResponse::Call(Box::new(self.call(*call).await?)),
            ReadRequest::Recordings {
                limit,
                shelf,
                project,
            } => ReadResponse::Recordings {
                recordings: self.recordings(*limit, *shelf, project.as_ref()).await?,
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
            ReadRequest::Usage { range, scope } => {
                ReadResponse::Usage(self.usage(*range, scope).await?)
            }
            ReadRequest::Search(query) => ReadResponse::Search(self.search(query).await?),
            ReadRequest::Exposures(query) => ReadResponse::Exposures {
                exposures: self.exposures(query).await?,
            },
            ReadRequest::Runs {
                limit,
                shelf,
                project,
            } => ReadResponse::Runs {
                runs: self.runs(*limit, *shelf, project.as_ref()).await?,
            },
            ReadRequest::Run(run) => {
                ReadResponse::Run(Box::new(self.run(*run).await?.ok_or(StoreError::NotFound)?))
            }
            ReadRequest::Conversation { run, bodies } => ReadResponse::Conversation(
                self.conversation(*run, Self::CONVERSATION, *bodies)
                    .await?
                    .ok_or(StoreError::NotFound)?,
            ),
            ReadRequest::Passages(session) => ReadResponse::Passages {
                passages: self.passages(*session).await?.ok_or(StoreError::NotFound)?,
            },
            ReadRequest::WorkItems {
                limit,
                shelf,
                project,
            } => ReadResponse::WorkItems {
                work: self.work_items(*limit, *shelf, project.as_ref()).await?,
            },
            ReadRequest::WorkItem(item) => ReadResponse::WorkItem(Box::new(
                self.work_item(*item).await?.ok_or(StoreError::NotFound)?,
            )),
        })
    }
}
