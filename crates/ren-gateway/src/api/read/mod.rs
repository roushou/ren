//! The read API's bodies: what `/api/calls`, `/api/recordings`, `/api/turns`,
//! `/api/usage`, `/api/search`, `/api/exposures`, and `/api/runs` send.
//!
//! These types are the API's contract. Each is converted once from the store
//! or harness type it reports, with every field named, so a change there does
//! not reach the API until it is made here too. ts-rs writes them to the UI
//! (`ui/apps/web/src/lib/wire`); the CLI and the MCP server read and print the
//! same types. Leaf values keep their owner's text or number form (ids,
//! labels, times), and each field declares that form for TypeScript.

pub mod activity;
pub mod calls;
pub mod drive;
pub mod recordings;
pub mod report;
pub mod runs;
pub mod search;
#[cfg(test)]
mod tests;
pub mod usage;

use activity::{CallActivity, TurnActivity, TurnSummary};
use calls::{CallDetail, CallPage};
use recordings::RecordingView;
use ren_store::read::{ReadRequest, ReadResponse};
use report::RecordingReport;
use runs::{ConversationView, RunView};
use search::{ExposureView, SearchPage};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use usage::UsageView;

/// What a read answers, before the API adds its version.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum ReadBody {
    Calls(CallPage),
    Call(Box<CallDetail>),
    Recordings(Recordings),
    Turns(Turns),
    Turn(Box<TurnActivity>),
    CallActivity(Box<CallActivity>),
    Report(Box<RecordingReport>),
    Usage(UsageView),
    Search(SearchPage),
    Exposures(Exposures),
    Runs(Runs),
    Run(Box<RunView>),
    Conversation(ConversationView),
}

/// A recording listing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Recordings {
    pub recordings: Vec<RecordingView>,
}

/// A recording's turns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Turns {
    pub turns: Vec<TurnSummary>,
}

/// Exposed secrets, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Exposures {
    pub exposures: Vec<ExposureView>,
}

/// Agent runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Runs {
    pub runs: Vec<RunView>,
}

impl From<ReadResponse> for ReadBody {
    fn from(response: ReadResponse) -> Self {
        match response {
            ReadResponse::Calls(page) => Self::Calls(page.into()),
            ReadResponse::Call(detail) => Self::Call(Box::new((*detail).into())),
            ReadResponse::Recordings { recordings } => Self::Recordings(Recordings {
                recordings: recordings.into_iter().map(Into::into).collect(),
            }),
            ReadResponse::Turns { turns } => Self::Turns(Turns {
                turns: turns.into_iter().map(Into::into).collect(),
            }),
            ReadResponse::Turn(turn) => Self::Turn(Box::new((*turn).into())),
            ReadResponse::CallActivity(activity) => {
                Self::CallActivity(Box::new((*activity).into()))
            }
            ReadResponse::Report(report) => Self::Report(Box::new((*report).into())),
            ReadResponse::Usage(usage) => Self::Usage(usage.into()),
            ReadResponse::Search(page) => Self::Search(page.into()),
            ReadResponse::Exposures { exposures } => Self::Exposures(Exposures {
                exposures: exposures.into_iter().map(Into::into).collect(),
            }),
            ReadResponse::Runs { runs } => Self::Runs(Runs {
                runs: runs.into_iter().map(Into::into).collect(),
            }),
            ReadResponse::Run(run) => Self::Run(Box::new((*run).into())),
            ReadResponse::Conversation(conversation) => Self::Conversation(conversation.into()),
        }
    }
}

impl ReadBody {
    /// The API's JSON body for `request`, parsed into what that read answers.
    /// Fields the body has beyond the answer (its `api_version`) are ignored.
    pub fn decode(
        request: &ReadRequest,
        body: serde_json::Value,
    ) -> Result<Self, serde_json::Error> {
        Ok(match request {
            ReadRequest::Calls(_) | ReadRequest::RecordingCalls { .. } => {
                Self::Calls(serde_json::from_value(body)?)
            }
            ReadRequest::Call(_) => Self::Call(serde_json::from_value(body)?),
            ReadRequest::Recordings { .. } => Self::Recordings(serde_json::from_value(body)?),
            ReadRequest::Turns { .. } => Self::Turns(serde_json::from_value(body)?),
            ReadRequest::Turn(_) => Self::Turn(serde_json::from_value(body)?),
            ReadRequest::CallActivity(_) => Self::CallActivity(serde_json::from_value(body)?),
            ReadRequest::Report(_) => Self::Report(serde_json::from_value(body)?),
            ReadRequest::Usage(_) => Self::Usage(serde_json::from_value(body)?),
            ReadRequest::Search(_) => Self::Search(serde_json::from_value(body)?),
            ReadRequest::Exposures(_) => Self::Exposures(serde_json::from_value(body)?),
            ReadRequest::Runs { .. } => Self::Runs(serde_json::from_value(body)?),
            ReadRequest::Run(_) => Self::Run(serde_json::from_value(body)?),
            ReadRequest::Conversation(_) => Self::Conversation(serde_json::from_value(body)?),
        })
    }
}
