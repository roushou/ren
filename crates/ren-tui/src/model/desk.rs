//! Every run the console knows of, live or recorded, and the order they are
//! visited in: what needs you first.

use super::transcript::Transcript;
use crate::ui::Text;
use ren_harness::{
    Harness,
    drive::{ApprovalAsk, ApprovalId, Prompt},
    settings::{ApprovalPolicy, RunSettings},
};
use ren_protocol::label::Stored;
use ren_protocol::native::NativeThreadId;
use ren_recorder::{id::RunId, time::UnixMillis};
use ren_store::view::RunView;
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
    /// Typed while a turn ran: sent once the run waits for a prompt.
    pub queued: Option<Prompt>,
    /// Whether its conversation was asked for once already.
    pub asked: bool,
    /// What it was started with, as the daemon holds it while it runs.
    pub settings: Option<RunSettings>,
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
            queued: None,
            asked: false,
            settings: None,
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

    /// Its settings in a few words, each left to the client left out:
    /// `gpt-deep · high effort · edits alone`.
    pub fn setup(&self) -> Option<String> {
        let settings = self.settings();
        let words: Vec<String> = [
            settings.model.map(|model| model.to_string()),
            settings.effort.map(|effort| format!("{effort} effort")),
            settings
                .approvals
                .map(|policy| Self::policy(policy).to_owned()),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!words.is_empty()).then(|| words.join(" · "))
    }

    /// An approval policy in a few words.
    pub fn policy(policy: ApprovalPolicy) -> &'static str {
        match policy {
            ApprovalPolicy::Ask => "ask first",
            ApprovalPolicy::Edits => "edits alone",
            ApprovalPolicy::Auto => "auto",
        }
    }

    /// What it was started with: as the daemon holds it, or as recorded.
    /// Settings this binary does not know are left to the client.
    pub fn settings(&self) -> RunSettings {
        if let Some(settings) = &self.settings {
            return settings.clone();
        }
        match &self.recorded {
            Some(recorded) => RunSettings {
                model: recorded
                    .model
                    .as_deref()
                    .and_then(|model| model.parse().ok()),
                effort: recorded.effort.as_ref().and_then(Stored::known),
                approvals: recorded.approvals.as_ref().and_then(Stored::known),
            },
            None => RunSettings::default(),
        }
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
            .transcript
            .first_prompt()
            .or_else(|| self.queued.as_ref().map(Prompt::as_str));
        match (first, self.started_at) {
            (Some(text), _) => Text(text.lines().next().unwrap_or(text)).summary(32),
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

    /// The thread a live run holds now.
    pub fn thread_now(&self) -> Option<&NativeThreadId> {
        match &self.live {
            Some(RunState::Idle { thread } | RunState::Working { thread, .. }) => Some(thread),
            Some(RunState::Starting | RunState::Stopping { .. } | RunState::Ended { .. })
            | None => None,
        }
    }

    /// What an approval asks, in a few words.
    pub fn asked(ask: Option<&ApprovalAsk>) -> String {
        match ask {
            Some(ApprovalAsk::Command {
                command: Some(command),
                ..
            }) => command.clone(),
            Some(ApprovalAsk::Command { command: None, .. }) => "a command".to_owned(),
            Some(ApprovalAsk::FileChange { reason }) => reason.clone().map_or_else(
                || "a file change".to_owned(),
                |reason| format!("changing {reason}"),
            ),
            Some(ApprovalAsk::Tool { tool, .. }) => tool.clone(),
            None => "its request".to_owned(),
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
            run.settings = Some(snapshot.settings.clone());
        }
    }

    /// The runs history lists now. An ended run it no longer lists was
    /// archived or deleted, and leaves the desk.
    pub fn recorded(&mut self, recorded: Vec<RunView>) {
        self.runs.retain(|run| {
            run.attention() != Attention::Ended || recorded.iter().any(|view| view.id == run.id)
        });
        for view in recorded {
            let run = self.entry(view.id, |seen| {
                Run::new(view.id, view.harness.clone(), view.workdir.clone(), seen)
            });
            run.started_at = Some(view.started_at);
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

    /// The folders runs worked in, newest first, each once.
    pub fn recent_folders(&self) -> Vec<&str> {
        let mut runs: Vec<&Run> = self.runs.iter().collect();
        runs.sort_by(|a, b| b.started_at.cmp(&a.started_at).then(b.seen.cmp(&a.seen)));
        let mut folders: Vec<&str> = Vec::new();
        for run in runs {
            if !folders.contains(&run.workdir.as_str()) {
                folders.push(&run.workdir);
            }
        }
        folders
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

    /// The live run driving the session whose client names its
    /// conversation `native`: a run's thread id.
    pub fn driving(&self, native: &str) -> Option<&Run> {
        self.runs.iter().find(|run| {
            run.thread_now()
                .is_some_and(|thread| thread.to_string() == native)
        })
    }

    /// The threads live runs hold: their sessions keep a pane.
    pub fn threads(&self) -> Vec<String> {
        self.runs
            .iter()
            .filter_map(|run| run.thread_now().map(ToString::to_string))
            .collect()
    }
}
