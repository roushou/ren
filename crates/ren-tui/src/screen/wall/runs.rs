//! Agent runs on the wall: what a live run's pane shows of it, and the
//! counts on the status line. The wall only watches runs; `r` opens the
//! console, which drives them.

use crate::model::desk::{Attention, Desk, Run};
use ren_harness::drive::ApprovalAsk;
use ren_recorder::id::RunId;
use ren_supervisor::RunState;

/// A live run's mark on its session's pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunMark {
    pub run: RunId,
    pub phase: RunPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunPhase {
    /// Waiting for a prompt.
    Idle,
    /// A turn is running.
    Working,
    /// The agent waits for the operator: the oldest ask, and how many more.
    Waiting { ask: ApprovalAsk, more: usize },
}

/// Live runs, and how many wait for an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunCounts {
    pub live: usize,
    pub waiting: usize,
}

impl RunMark {
    /// What `run`'s pane shows of it, while it can take commands.
    pub(crate) fn of(run: &Run) -> Option<Self> {
        let phase = match &run.live {
            Some(RunState::Idle { .. }) => RunPhase::Idle,
            Some(RunState::Working { approvals, .. }) => match approvals.first() {
                Some(waiting) => RunPhase::Waiting {
                    ask: waiting.ask.clone(),
                    more: approvals.len() - 1,
                },
                None => RunPhase::Working,
            },
            Some(RunState::Starting | RunState::Stopping { .. } | RunState::Ended { .. })
            | None => return None,
        };
        Some(Self { run: run.id, phase })
    }
}

impl RunCounts {
    pub(crate) fn of(desk: &Desk) -> Self {
        let live = desk.ordered();
        Self {
            live: live.len(),
            waiting: live
                .iter()
                .filter(|run| run.attention() == Attention::NeedsYou)
                .count(),
        }
    }
}
