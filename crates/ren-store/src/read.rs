//! Every history read as one typed request, served by [`Reader::serve`]. The
//! HTTP API, the CLI on this host, and the MCP server all ask through it, so
//! they answer with the same values: a response serializes to the API's
//! JSON body.

use crate::activity::report::SessionReport;
use crate::activity::{ActivityBudget, CallActivity, TurnActivity, TurnSummary};
use crate::query::{CallQuery, Cursor, PageLimit, TimeRange};
use crate::search::{ExposureQuery, ExposureView, SearchPage, SearchQuery};
use crate::view::{CallDetail, CallPage, RunView, SessionView, UsageView};
use crate::{Reader, StoreError};
use ren_recorder::id::{CallId, RunId, SessionId, TurnId};
use serde::Serialize;

/// A read of recorded history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadRequest {
    /// Calls, newest first, narrowed by the query's filters.
    Calls(CallQuery),
    Call(CallId),
    Sessions(PageLimit),
    /// A session's calls, oldest first, a page at a time.
    SessionCalls {
        session: SessionId,
        limit: PageLimit,
        after: Option<Cursor>,
    },
    Turns {
        session: SessionId,
        limit: PageLimit,
    },
    Turn(TurnId),
    CallActivity(CallId),
    Report(SessionId),
    Usage(TimeRange),
    Search(SearchQuery),
    Exposures(ExposureQuery),
    /// Agent runs, newest first.
    Runs(PageLimit),
    Run(RunId),
}

/// What a [`ReadRequest`] returns. Serialized, each is the API's JSON body
/// for that read (before the API adds its version).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ReadResponse {
    Calls(CallPage),
    Call(Box<CallDetail>),
    Sessions { sessions: Vec<SessionView> },
    Turns { turns: Vec<TurnSummary> },
    Turn(Box<TurnActivity>),
    CallActivity(Box<CallActivity>),
    Report(Box<SessionReport>),
    Usage(UsageView),
    Search(SearchPage),
    Exposures { exposures: Vec<ExposureView> },
    Runs { runs: Vec<RunView> },
    Run(Box<RunView>),
}

impl Reader {
    /// Answer one read.
    pub async fn serve(&self, request: &ReadRequest) -> Result<ReadResponse, StoreError> {
        Ok(match request {
            ReadRequest::Calls(query) => ReadResponse::Calls(self.calls(query).await?),
            ReadRequest::Call(call) => ReadResponse::Call(Box::new(self.call(*call).await?)),
            ReadRequest::Sessions(limit) => ReadResponse::Sessions {
                sessions: self.sessions(*limit).await?,
            },
            ReadRequest::SessionCalls {
                session,
                limit,
                after,
            } => ReadResponse::Calls(self.session_calls(*session, *limit, *after).await?),
            ReadRequest::Turns { session, limit } => ReadResponse::Turns {
                turns: self.session_turns(*session, *limit).await?,
            },
            ReadRequest::Turn(turn) => ReadResponse::Turn(Box::new(
                self.turn_activity(*turn, ActivityBudget::DEFAULT).await?,
            )),
            ReadRequest::CallActivity(call) => ReadResponse::CallActivity(Box::new(
                self.call_activity(*call, ActivityBudget::DEFAULT).await?,
            )),
            ReadRequest::Report(session) => {
                ReadResponse::Report(Box::new(self.session_report(*session).await?))
            }
            ReadRequest::Usage(range) => ReadResponse::Usage(self.usage(*range).await?),
            ReadRequest::Search(query) => ReadResponse::Search(self.search(query).await?),
            ReadRequest::Exposures(query) => ReadResponse::Exposures {
                exposures: self.exposures(query).await?,
            },
            ReadRequest::Runs(limit) => ReadResponse::Runs {
                runs: self.runs(*limit).await?,
            },
            ReadRequest::Run(run) => {
                ReadResponse::Run(Box::new(self.run(*run).await?.ok_or(StoreError::NotFound)?))
            }
        })
    }
}
