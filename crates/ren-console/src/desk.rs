//! Every run the console knows of, live or recorded, and the order they are
//! visited in: what needs you first.

use crate::transcript::Transcript;
use ren_harness::{
    Harness,
    drive::{ApprovalAsk, ApprovalId, Prompt},
};
use ren_protocol::label::Stored;
use ren_recorder::{id::RunId, time::UnixMillis};
use ren_store::{activity::TurnActivity, view::RunView};
use ren_supervisor::{RunSnapshot, RunState, Waiting};

/// How much a run needs the operator, most first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Attention {
    /// It waits for an approval.
    NeedsYou,
    /// Its turn ended: it waits for a prompt.
    YourTurn,
    Working,
    Starting,
    /// No longer running.
    Ended,
}

/// One run, as far as the console knows it.
#[derive(Debug, Clone)]
pub struct Run {
    pub id: RunId,
    pub harness: Stored<Harness>,
    pub workdir: String,
    /// When it was seen first: for a stable order among equals.
    pub seen: u64,
    pub started_at: Option<UnixMillis>,
    /// Its state while the daemon holds it.
    pub live: Option<RunState>,
    pub recorded: Option<RunView>,
    pub transcript: Transcript,
    /// Recorded turns, oldest first.
    pub history: Vec<TurnActivity>,
    /// Typed while a turn ran: sent once the run waits for a prompt.
    pub queued: Option<Prompt>,
    /// History reads asked for since its last turn ended without its record.
    pub awaiting: u8,
    /// Whether its history was asked for once already, to name it.
    pub named: bool,
}

impl Run {
    pub fn new(id: RunId, harness: Stored<Harness>, workdir: String, seen: u64) -> Self {
        Self {
            id,
            harness,
            workdir,
            seen,
            started_at: None,
            live: None,
            recorded: None,
            transcript: Transcript::default(),
            history: Vec::new(),
            queued: None,
            awaiting: 0,
            named: false,
        }
    }

    pub fn from_snapshot(snapshot: &RunSnapshot, seen: u64) -> Self {
        let mut run = Self::new(
            snapshot.run,
            Stored::Known(snapshot.harness),
            snapshot.workdir.to_string_lossy().into_owned(),
            seen,
        );
        run.live = Some(snapshot.state.clone());
        run
    }

    pub fn attention(&self) -> Attention {
        match &self.live {
            Some(RunState::Working { approvals, .. }) if !approvals.is_empty() => {
                Attention::NeedsYou
            }
            Some(RunState::Working { .. }) => Attention::Working,
            Some(RunState::Idle { .. }) => Attention::YourTurn,
            Some(RunState::Starting) => Attention::Starting,
            Some(RunState::Stopping { .. } | RunState::Ended { .. }) | None => Attention::Ended,
        }
    }

    /// The oldest approval it waits for.
    pub fn waiting(&self) -> Option<&Waiting> {
        match &self.live {
            Some(RunState::Working { approvals, .. }) => approvals.first(),
            Some(
                RunState::Starting
                | RunState::Idle { .. }
                | RunState::Stopping { .. }
                | RunState::Ended { .. },
            )
            | None => None,
        }
    }

    pub fn approval(&self) -> Option<(ApprovalId, ApprovalAsk)> {
        self.waiting()
            .map(|waiting| (waiting.approval.clone(), waiting.ask.clone()))
    }

    pub fn harness(&self) -> Option<Harness> {
        self.harness.known()
    }

    /// The client's name, as the header writes it.
    pub fn client(&self) -> String {
        match self.harness() {
            Some(Harness::ClaudeCode) => "Claude Code".to_owned(),
            Some(Harness::Codex) => "Codex".to_owned(),
            Some(Harness::Pi) => "Pi".to_owned(),
            Some(Harness::OpenCode) => "OpenCode".to_owned(),
            None => self.harness.to_string(),
        }
    }

    /// The project, by its folder's name.
    pub fn project(&self) -> &str {
        self.workdir
            .rsplit('/')
            .find(|part| !part.is_empty())
            .unwrap_or(&self.workdir)
    }

    /// What the run is called: its first prompt, shortened.
    pub fn title(&self) -> String {
        let first = self
            .history
            .first()
            .and_then(|turn| turn.prompt.as_ref().map(|prompt| prompt.text.as_str()))
            .or_else(|| self.transcript.first_prompt())
            .or_else(|| self.queued.as_ref().map(Prompt::as_str));
        match (first, self.started_at) {
            (Some(text), _) => Self::shorten(text.lines().next().unwrap_or(text), 32),
            (None, Some(started)) => {
                // Hours and minutes, UTC: a run named by when it started.
                let minutes = started.as_millis() / 60_000;
                format!(
                    "{} run from {:02}:{:02}",
                    self.client(),
                    minutes / 60 % 24,
                    minutes % 60
                )
            }
            (None, None) => format!("new {} run", self.client()),
        }
    }

    pub fn shorten(text: &str, chars: usize) -> String {
        let text = text.trim();
        if text.chars().count() <= chars {
            return text.to_owned();
        }
        let mut short: String = text.chars().take(chars.saturating_sub(1)).collect();
        short.push('…');
        short
    }

    /// The thread to resume its conversation with, once it has one.
    pub fn thread(&self) -> Option<&str> {
        match &self.live {
            Some(RunState::Idle { .. } | RunState::Working { .. }) => None,
            Some(RunState::Starting | RunState::Stopping { .. } | RunState::Ended { .. })
            | None => self
                .recorded
                .as_ref()
                .and_then(|recorded| recorded.thread.as_deref()),
        }
    }

    /// The state in a word or two, as the header and lists write it.
    pub fn phase(&self) -> &'static str {
        match self.attention() {
            Attention::NeedsYou => "needs you",
            Attention::YourTurn => "your turn",
            Attention::Working => "working",
            Attention::Starting => "starting",
            Attention::Ended => "ended",
        }
    }
}

/// The runs a console holds, and their order.
#[derive(Debug, Default)]
pub struct Desk {
    runs: Vec<Run>,
    seen: u64,
}

impl Desk {
    pub fn get(&self, id: RunId) -> Option<&Run> {
        self.runs.iter().find(|run| run.id == id)
    }

    pub fn get_mut(&mut self, id: RunId) -> Option<&mut Run> {
        self.runs.iter_mut().find(|run| run.id == id)
    }

    /// The run with `id`, made from `make` if it is new.
    pub fn entry(&mut self, id: RunId, make: impl FnOnce(u64) -> Run) -> &mut Run {
        let index = match self.runs.iter().position(|run| run.id == id) {
            Some(index) => index,
            None => {
                self.seen += 1;
                self.runs.push(make(self.seen));
                self.runs.len() - 1
            }
        };
        &mut self.runs[index]
    }

    /// Live runs as the daemon lists them now: ones it no longer lists have
    /// ended.
    pub fn live(&mut self, snapshots: &[RunSnapshot]) {
        for run in &mut self.runs {
            if !snapshots.iter().any(|snapshot| snapshot.run == run.id) {
                match run.live {
                    Some(RunState::Ended { .. }) | None => {}
                    Some(_) => run.live = None,
                }
            }
        }
        for snapshot in snapshots {
            let run = self.entry(snapshot.run, |seen| Run::from_snapshot(snapshot, seen));
            run.live = Some(snapshot.state.clone());
        }
    }

    pub fn recorded(&mut self, recorded: Vec<RunView>) {
        for view in recorded {
            let run = self.entry(view.id, |seen| {
                Run::new(view.id, view.harness.clone(), view.workdir.clone(), seen)
            });
            run.started_at = Some(view.started_at);
            // No call recorded yet: no turn either, so every turn recorded
            // from now on is new.
            if view.session.is_none() && !run.transcript.knows() {
                run.transcript.knowing(Vec::new());
            }
            run.recorded = Some(view);
        }
    }

    /// Runs that are not over, what needs you first, then newest first.
    pub fn ordered(&self) -> Vec<&Run> {
        let mut live: Vec<&Run> = self
            .runs
            .iter()
            .filter(|run| run.attention() != Attention::Ended)
            .collect();
        live.sort_by(|a, b| a.attention().cmp(&b.attention()).then(b.seen.cmp(&a.seen)));
        live
    }

    /// Every run: what needs you first, then the newest ended ones.
    pub fn all(&self) -> Vec<&Run> {
        const ENDED: usize = 20;
        let mut all: Vec<&Run> = self.runs.iter().collect();
        all.sort_by(|a, b| {
            a.attention()
                .cmp(&b.attention())
                .then(b.started_at.cmp(&a.started_at))
                .then(b.seen.cmp(&a.seen))
        });
        let live = all
            .iter()
            .take_while(|run| run.attention() != Attention::Ended)
            .count();
        all.truncate(live + ENDED);
        all
    }

    /// The live run after (or before) `from` in [`Desk::ordered`].
    pub fn step(&self, from: Option<RunId>, forward: bool) -> Option<RunId> {
        let order = self.ordered();
        if order.is_empty() {
            return None;
        }
        let at = from.and_then(|from| order.iter().position(|run| run.id == from));
        let next = match (at, forward) {
            (None, true) => 0,
            (None, false) => order.len() - 1,
            (Some(at), true) => (at + 1) % order.len(),
            (Some(at), false) => (at + order.len() - 1) % order.len(),
        };
        order.get(next).map(|run| run.id)
    }

    /// The run that needs you most, other than `except`.
    pub fn next_needing(&self, except: Option<RunId>) -> Option<RunId> {
        self.ordered()
            .into_iter()
            .find(|run| run.attention() == Attention::NeedsYou && Some(run.id) != except)
            .map(|run| run.id)
    }

    pub fn needing(&self) -> usize {
        self.ordered()
            .iter()
            .filter(|run| run.attention() == Attention::NeedsYou)
            .count()
    }

    pub fn live_count(&self) -> usize {
        self.ordered().len()
    }
}
