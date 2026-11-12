//! The terminal console for agent runs: one conversation fills the screen,
//! with a message box below it. Runs are started, switched, and stopped with
//! slash commands typed in that box; the runs that need you come first, and
//! answering one moves on to the next that does.
//!
//! [`Console`] is sans-IO: it takes what was read ([`Input`]) and key
//! presses, and answers with [`Out`]s, the reads it needs and the commands
//! to send through the control API. [`Console::frame`] is what to draw;
//! `draw` renders it into a ratatui buffer, and the crate's `Screen` puts
//! it on the terminal.

mod composer;
mod desk;
mod draw;
mod slash;
#[cfg(test)]
mod tests;
mod transcript;
mod view;

pub use slash::Suggestion;
pub use transcript::Entry;
pub use view::{Frame, Styled};

use crate::{Key, Size};
use composer::Composer;
use desk::{Attention, Desk, Run};
use ren_harness::{
    Harness,
    drive::{ApprovalAsk, ApprovalId, Decision, DriveEvent, Prompt},
};
use ren_protocol::label::Stored;
use ren_protocol::native::NativeThreadId;
use ren_recorder::id::{RunId, SessionId};
use ren_store::{activity::TurnActivity, view::RunView};
use ren_supervisor::{Project, RunSnapshot, RunState, RunUpdate};
use slash::{Slash, Unresolved};

/// The console for one gateway's runs.
#[derive(Debug)]
pub struct Console {
    size: Size,
    desk: Desk,
    projects: Projects,
    screen: Screen,
    composer: Composer,
    /// Which completion is chosen, while some are offered.
    choice: usize,
    /// Completions closed with esc until the text changes.
    dismissed: bool,
    /// The client `/new` starts when none is named: the last one started.
    client: Harness,
    notice: Option<Notice>,
    /// What happened to the run just left, shown atop the next.
    handoff: Option<String>,
    /// Lines scrolled up from the newest.
    scroll: usize,
    control: Control,
    /// The run this console asked to start, until the daemon answers.
    starting: Option<Starting>,
}

/// A run asked for, to hold as soon as the daemon names it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Starting {
    harness: Harness,
    dir: String,
    resumes: bool,
    prompt: Option<Prompt>,
}

/// What fills the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Screen {
    /// No run open: the runs, when nothing needs you.
    Home {
        chosen: usize,
    },
    Run(RunId),
    /// Every run, to pick one, with the chosen one previewed.
    Resume {
        search: String,
        chosen: usize,
        back: Option<RunId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Projects {
    Unread,
    Read(Vec<Project>),
    Failed(String),
}

/// Whether runs can be driven from here.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Control {
    Unread,
    Ready,
    /// Why not: no control access, or runs are off on the daemon.
    Unavailable(String),
}

/// A line under the message box until the next key.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Notice {
    text: String,
    tone: Tone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Info,
    Refusal,
}

/// What was read, or what happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// The daemon's runs, or why they cannot be read.
    Runs(Result<Vec<RunSnapshot>, String>),
    /// Recorded runs, newest first.
    Recorded(Vec<RunView>),
    Projects(Result<Vec<Project>, String>),
    /// One update from the run feed.
    Update(RunUpdate),
    /// The feed skipped updates: the runs are read again.
    Lagged,
    /// A run's recorded turns, oldest first.
    History {
        run: RunId,
        turns: Vec<TurnActivity>,
    },
    /// How a command was answered.
    Answered(Result<Answered, String>),
    Resize(Size),
}

/// How the daemon took a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered {
    Sent,
    Started(RunId),
}

/// A read the console needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Read {
    Runs,
    Recorded,
    Projects,
    /// The run's recorded turns, from its session, now.
    History {
        run: RunId,
        session: SessionId,
    },
    /// Again after a pause: its newest turn ended and is not recorded yet.
    HistoryLater {
        run: RunId,
        session: SessionId,
    },
}

/// A command for the control API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Start {
        harness: Harness,
        dir: String,
        resume: Option<NativeThreadId>,
        prompt: Option<Prompt>,
    },
    Prompt {
        run: RunId,
        prompt: Prompt,
    },
    Answer {
        run: RunId,
        approval: ApprovalId,
        decision: Decision,
    },
    Interrupt(RunId),
    Stop(RunId),
}

/// What the shell does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    Read(Read),
    Send(Command),
    Quit,
}

impl Console {
    /// History reads after a turn ends before its record is given up on.
    const HISTORY_TRIES: u8 = 5;

    pub fn new(size: Size) -> Self {
        Self {
            size,
            desk: Desk::default(),
            projects: Projects::Unread,
            screen: Screen::Home { chosen: 0 },
            composer: Composer::default(),
            choice: 0,
            dismissed: false,
            client: Harness::ClaudeCode,
            notice: None,
            handoff: None,
            scroll: 0,
            control: Control::Unread,
            starting: None,
        }
    }

    /// The reads to make when the console starts.
    pub fn start(&self) -> Vec<Out> {
        vec![
            Out::Read(Read::Runs),
            Out::Read(Read::Recorded),
            Out::Read(Read::Projects),
        ]
    }

    fn current(&self) -> Option<RunId> {
        match self.screen {
            Screen::Run(run) => Some(run),
            Screen::Home { .. } | Screen::Resume { .. } => None,
        }
    }

    fn run(&self) -> Option<&Run> {
        self.current().and_then(|run| self.desk.get(run))
    }

    fn projects(&self) -> &[Project] {
        match &self.projects {
            Projects::Read(projects) => projects,
            Projects::Unread | Projects::Failed(_) => &[],
        }
    }

    /// Completions for what is typed, unless closed.
    fn suggestions(&self) -> Vec<Suggestion> {
        if self.dismissed {
            return Vec::new();
        }
        let text = self.composer.text();
        if text == "/help" {
            return Slash::help();
        }
        Slash::complete(&text, self.projects())
    }

    fn open(&mut self, run: RunId) -> Vec<Out> {
        self.screen = Screen::Run(run);
        self.scroll = 0;
        vec![self.history(run, false)]
    }

    /// Read `run`'s recorded turns, now or `later`; a run whose session is
    /// not known yet has the recorded runs read first.
    fn history(&self, run: RunId, later: bool) -> Out {
        let session = self
            .desk
            .get(run)
            .and_then(|run| run.recorded.as_ref())
            .and_then(|recorded| recorded.session);
        match (session, later) {
            (Some(session), false) => Out::Read(Read::History { run, session }),
            (Some(session), true) => Out::Read(Read::HistoryLater { run, session }),
            (None, _) => Out::Read(Read::Recorded),
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            tone: Tone::Info,
        });
    }

    fn refuse(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            tone: Tone::Refusal,
        });
    }

    /// Takes one key press.
    pub fn press(&mut self, key: Key) -> Vec<Out> {
        self.notice = None;
        if let Screen::Resume { .. } = self.screen {
            return self.press_resume(key);
        }
        let suggestions = self.suggestions();
        if !suggestions.is_empty() {
            match key {
                Key::Up => {
                    self.choice = self.choice.saturating_sub(1);
                    return Vec::new();
                }
                Key::Down => {
                    self.choice = (self.choice + 1).min(suggestions.len() - 1);
                    return Vec::new();
                }
                Key::Tab => {
                    if let Some(chosen) = suggestions.get(self.choice) {
                        self.composer.set(&chosen.replacement);
                        self.choice = 0;
                    }
                    return Vec::new();
                }
                Key::Esc => {
                    self.dismissed = true;
                    return Vec::new();
                }
                Key::Char(_)
                | Key::BackTab
                | Key::Left
                | Key::Right
                | Key::Enter
                | Key::Backspace
                | Key::Delete
                | Key::Home
                | Key::End
                | Key::PageUp
                | Key::PageDown
                | Key::AltLeft
                | Key::AltRight
                | Key::Newline
                | Key::Interrupt => {}
            }
        }
        // An approval waiting takes 1-4 while nothing is typed.
        if let (Key::Char(digit @ '1'..='4'), true) = (key, self.composer.is_empty())
            && let Some((approval, _)) = self.run().and_then(Run::approval)
        {
            let decision = match digit {
                '1' => Decision::Accept,
                '2' => Decision::AcceptForSession,
                '3' => Decision::Decline,
                _ => Decision::Cancel,
            };
            return self.answer(approval, decision, None);
        }
        match key {
            Key::Char(ch) => self.typed(|composer| composer.insert(ch)),
            Key::Newline => self.typed(|composer| composer.insert('\n')),
            Key::Backspace => self.typed(Composer::backspace),
            Key::Delete => self.typed(Composer::delete),
            Key::Left => self.composer.left(),
            Key::Right => self.composer.right(),
            Key::Home => self.composer.home(),
            Key::End => self.composer.end(),
            Key::PageUp => self.scroll = self.scroll.saturating_add(usize::from(self.page())),
            Key::PageDown => self.scroll = self.scroll.saturating_sub(usize::from(self.page())),
            Key::Up => match &mut self.screen {
                Screen::Home { chosen } => *chosen = chosen.saturating_sub(1),
                Screen::Run(_) | Screen::Resume { .. } => {
                    self.scroll = self.scroll.saturating_add(1);
                }
            },
            Key::Down => match &mut self.screen {
                Screen::Home { chosen } => {
                    *chosen = (*chosen + 1).min(self.desk.live_count().saturating_sub(1));
                }
                Screen::Run(_) | Screen::Resume { .. } => {
                    self.scroll = self.scroll.saturating_sub(1);
                }
            },
            Key::AltLeft | Key::AltRight => {
                return match self.desk.step(self.current(), key == Key::AltRight) {
                    Some(run) => {
                        self.handoff = None;
                        self.open(run)
                    }
                    None => {
                        self.say("no live runs: /new starts one");
                        Vec::new()
                    }
                };
            }
            Key::Tab | Key::BackTab => {}
            Key::Esc => return self.escape(),
            Key::Interrupt => {
                if self.composer.is_empty() {
                    return vec![Out::Quit];
                }
                self.composer.take();
            }
            Key::Enter => return self.submit(),
        }
        Vec::new()
    }

    fn typed(&mut self, edit: impl FnOnce(&mut Composer)) {
        edit(&mut self.composer);
        self.dismissed = false;
        self.choice = 0;
    }

    fn page(&self) -> u16 {
        self.size.height.saturating_sub(8).max(1)
    }

    /// Esc interrupts the running turn, as in the clients themselves.
    fn escape(&mut self) -> Vec<Out> {
        let Some(run) = self.run() else {
            return Vec::new();
        };
        match run.attention() {
            Attention::Working | Attention::NeedsYou => {
                vec![Out::Send(Command::Interrupt(run.id))]
            }
            Attention::YourTurn | Attention::Starting | Attention::Ended => Vec::new(),
        }
    }

    fn press_resume(&mut self, key: Key) -> Vec<Out> {
        let Screen::Resume {
            search,
            chosen,
            back,
        } = &mut self.screen
        else {
            return Vec::new();
        };
        match key {
            Key::Char(ch) => {
                search.push(ch);
                *chosen = 0;
            }
            Key::Backspace => {
                search.pop();
                *chosen = 0;
            }
            Key::Up => *chosen = chosen.saturating_sub(1),
            Key::Down => *chosen += 1,
            Key::Esc => {
                return match *back {
                    Some(run) => self.open(run),
                    None => {
                        self.screen = Screen::Home { chosen: 0 };
                        Vec::new()
                    }
                };
            }
            Key::Interrupt => return vec![Out::Quit],
            Key::Enter => {
                let found = self.resume_list();
                return match found.get(self.resume_chosen()).map(|run| run.id) {
                    Some(run) => {
                        self.handoff = None;
                        self.open(run)
                    }
                    None => Vec::new(),
                };
            }
            Key::Tab
            | Key::BackTab
            | Key::Left
            | Key::Right
            | Key::Delete
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
            | Key::AltLeft
            | Key::AltRight
            | Key::Newline => {}
        }
        // The chosen row stays within the list.
        let last = self.resume_list().len().saturating_sub(1);
        if let Screen::Resume { chosen, .. } = &mut self.screen {
            *chosen = (*chosen).min(last);
        }
        let preview = self
            .resume_list()
            .get(self.resume_chosen())
            .map(|run| run.id);
        preview.map_or_else(Vec::new, |run| vec![self.history(run, false)])
    }

    /// The runs `/resume` lists for its search, what needs you first.
    fn resume_list(&self) -> Vec<&Run> {
        let search = match &self.screen {
            Screen::Resume { search, .. } => search.to_lowercase(),
            Screen::Home { .. } | Screen::Run(_) => String::new(),
        };
        self.desk
            .all()
            .into_iter()
            .filter(|run| {
                search.is_empty()
                    || run.title().to_lowercase().contains(&search)
                    || run.workdir.to_lowercase().contains(&search)
            })
            .collect()
    }

    fn resume_chosen(&self) -> usize {
        match &self.screen {
            Screen::Resume { chosen, .. } => *chosen,
            Screen::Home { .. } | Screen::Run(_) => 0,
        }
    }

    fn submit(&mut self) -> Vec<Out> {
        let text = self.composer.text();
        if text.trim().is_empty() {
            if let Screen::Home { chosen } = self.screen
                && let Some(run) = self.desk.ordered().get(chosen).map(|run| run.id)
            {
                return self.open(run);
            }
            return Vec::new();
        }
        if let Some(slash) = Slash::parse(&text) {
            self.composer.take();
            self.dismissed = false;
            return self.slash(slash);
        }
        if let Control::Unavailable(why) = &self.control {
            let why = why.clone();
            self.refuse(format!("runs cannot be driven from here: {why}"));
            return Vec::new();
        }
        let Some(run) = self.run() else {
            self.say("open a run first, or start one: /new <project> <prompt>");
            return Vec::new();
        };
        let Ok(prompt) = Prompt::parse(text) else {
            return Vec::new();
        };
        let id = run.id;
        let approval = run.approval();
        let attention = run.attention();
        let resume = match (run.harness(), run.thread()) {
            (Some(harness), Some(thread)) => Some((
                harness,
                run.workdir.clone(),
                thread
                    .parse::<uuid::Uuid>()
                    .ok()
                    .and_then(NativeThreadId::new),
            )),
            (Some(_) | None, Some(_) | None) => None,
        };
        self.composer.take();
        // Typing while an approval waits declines it; the words go next.
        if let Some((approval, _)) = approval {
            return self.answer(approval, Decision::Decline, Some(prompt));
        }
        match attention {
            Attention::YourTurn => {
                if let Some(run) = self.desk.get_mut(id) {
                    run.transcript.prompted(prompt.as_str());
                }
                self.scroll = 0;
                vec![Out::Send(Command::Prompt { run: id, prompt })]
            }
            Attention::Working | Attention::Starting | Attention::NeedsYou => {
                self.queue(id, prompt);
                Vec::new()
            }
            // A message to a run that ended continues its conversation in a
            // new run.
            Attention::Ended => match resume {
                Some((harness, dir, resume)) => self.start_run(harness, dir, resume, Some(prompt)),
                None => {
                    self.say("this run ended before it opened a conversation: /new starts another");
                    Vec::new()
                }
            },
        }
    }

    fn start_run(
        &mut self,
        harness: Harness,
        dir: String,
        resume: Option<NativeThreadId>,
        prompt: Option<Prompt>,
    ) -> Vec<Out> {
        self.starting = Some(Starting {
            harness,
            dir: dir.clone(),
            resumes: resume.is_some(),
            prompt: prompt.clone(),
        });
        vec![Out::Send(Command::Start {
            harness,
            dir,
            resume,
            prompt,
        })]
    }

    /// Keep `prompt` until the run waits for one.
    fn queue(&mut self, run: RunId, prompt: Prompt) {
        if let Some(run) = self.desk.get_mut(run) {
            run.queued = Some(match run.queued.take() {
                Some(earlier) => {
                    Prompt::parse(format!("{}\n\n{}", earlier.as_str(), prompt.as_str()))
                        .unwrap_or(prompt)
                }
                None => prompt,
            });
        }
    }

    fn answer(
        &mut self,
        approval: ApprovalId,
        decision: Decision,
        then: Option<Prompt>,
    ) -> Vec<Out> {
        let Some(run) = self.run() else {
            return Vec::new();
        };
        let id = run.id;
        let what = Self::asked(run.approval().map(|(_, ask)| ask).as_ref());
        let left = run.title();
        let project = run.project().to_owned();
        let mut out = vec![Out::Send(Command::Answer {
            run: id,
            approval: approval.clone(),
            decision,
        })];
        if let Some(prompt) = then {
            self.queue(id, prompt);
        }
        // The run waits for nothing else: move on to the next that does.
        let more = match self.run().map(|run| &run.live) {
            Some(Some(RunState::Working { approvals, .. })) => approvals.len() > 1,
            Some(_) | None => false,
        };
        if !more && let Some(next) = self.desk.next_needing(Some(id)) {
            let verb = match decision {
                Decision::Accept | Decision::AcceptForSession => "allowed",
                Decision::Decline => "denied",
                Decision::Cancel => "denied and stopped",
            };
            self.handoff = Some(format!("from {project} · {left}: {verb} {what}"));
            out.extend(self.open(next));
        }
        out
    }

    /// What an approval asks, in a few words.
    fn asked(ask: Option<&ApprovalAsk>) -> String {
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

    fn slash(&mut self, slash: Slash) -> Vec<Out> {
        match slash {
            Slash::New {
                project,
                client,
                prompt,
            } => {
                let found =
                    Slash::resolve(&project, self.projects()).map(|found| found.path.clone());
                let dir = match found {
                    Ok(dir) => dir,
                    Err(Unresolved::Missing) => {
                        self.say("name a project: /new <project> [claude|codex] <prompt>");
                        return vec![Out::Read(Read::Projects)];
                    }
                    Err(Unresolved::NoMatch(_)) if project.starts_with('/') => project,
                    Err(Unresolved::NoMatch(name)) => {
                        self.refuse(format!("no project matches {name}"));
                        return vec![Out::Read(Read::Projects)];
                    }
                    Err(Unresolved::Ambiguous(names)) => {
                        self.refuse(format!("which one: {}", names.join(", ")));
                        return Vec::new();
                    }
                };
                let harness = client.unwrap_or(self.client);
                self.client = harness;
                self.start_run(
                    harness,
                    dir,
                    None,
                    prompt.and_then(|prompt| Prompt::parse(prompt).ok()),
                )
            }
            Slash::Resume { search } => {
                let back = self.current();
                self.screen = Screen::Resume {
                    search,
                    chosen: 0,
                    back,
                };
                let first = self.resume_list().first().map(|run| run.id);
                let mut out = vec![Out::Read(Read::Recorded)];
                out.extend(first.map(|run| self.history(run, false)));
                out
            }
            Slash::Stop => match self.run() {
                Some(run) if run.attention() != Attention::Ended => {
                    vec![Out::Send(Command::Stop(run.id))]
                }
                Some(_) | None => {
                    self.say("no live run here to stop");
                    Vec::new()
                }
            },
            Slash::Help => {
                self.composer.set("/help");
                Vec::new()
            }
            Slash::Quit => vec![Out::Quit],
            Slash::Unknown(name) => {
                self.refuse(format!("no command /{name}: /help lists them"));
                Vec::new()
            }
        }
    }

    pub fn apply(&mut self, input: Input) -> Vec<Out> {
        match input {
            Input::Runs(Ok(snapshots)) => {
                self.control = Control::Ready;
                self.desk.live(&snapshots);
                self.settle()
            }
            Input::Runs(Err(why)) => {
                self.control = Control::Unavailable(why);
                Vec::new()
            }
            Input::Recorded(recorded) => {
                self.desk.recorded(recorded);
                let mut named = self.name_runs();
                // The open run's history, once its session is known.
                let shown = match &self.screen {
                    Screen::Run(run) => Some(*run),
                    Screen::Resume { .. } => self
                        .resume_list()
                        .get(self.resume_chosen())
                        .map(|run| run.id),
                    Screen::Home { .. } => None,
                };
                shown
                    .filter(|run| {
                        self.desk.get(*run).is_some_and(|held| {
                            held.history.is_empty()
                                && held
                                    .recorded
                                    .as_ref()
                                    .is_some_and(|recorded| recorded.session.is_some())
                        })
                    })
                    .map(|run| self.history(run, false))
                    .filter(|read| !named.contains(read))
                    .into_iter()
                    .for_each(|read| named.push(read));
                named
            }
            Input::Projects(projects) => {
                self.projects = match projects {
                    Ok(projects) => Projects::Read(projects),
                    Err(why) => Projects::Failed(why),
                };
                Vec::new()
            }
            Input::Lagged => vec![Out::Read(Read::Runs)],
            Input::Update(update) => self.update(update),
            Input::History { run, turns } => {
                let Some(held) = self.desk.get_mut(run) else {
                    return Vec::new();
                };
                let recorded = held.transcript.recorded_in(&turns);
                let waiting = held.awaiting > 0;
                if recorded {
                    held.transcript.clear();
                    held.awaiting = 0;
                } else if waiting {
                    held.awaiting += 1;
                }
                if held.transcript.is_empty() || !waiting {
                    // What was recorded before the next turn starts.
                    held.transcript
                        .knowing(turns.iter().map(|turn| turn.native_id.clone()).collect());
                }
                held.history = turns;
                if !recorded && waiting && held.awaiting <= Self::HISTORY_TRIES {
                    return vec![self.history(run, true)];
                }
                if !recorded && waiting {
                    held.awaiting = 0;
                }
                Vec::new()
            }
            Input::Answered(Ok(Answered::Started(run))) => {
                self.handoff = None;
                if let Some(starting) = self.starting.take() {
                    let held = self.desk.entry(run, |seen| {
                        Run::new(
                            run,
                            Stored::Known(starting.harness),
                            starting.dir.clone(),
                            seen,
                        )
                    });
                    // A new conversation has no recorded turns: every one
                    // recorded from now on is new.
                    if !starting.resumes {
                        held.transcript.knowing(Vec::new());
                    }
                    if let Some(prompt) = &starting.prompt {
                        held.transcript.prompted(prompt.as_str());
                    }
                }
                let mut out = vec![Out::Read(Read::Runs), Out::Read(Read::Recorded)];
                out.extend(self.open(run));
                out
            }
            Input::Answered(Ok(Answered::Sent)) => Vec::new(),
            Input::Answered(Err(why)) => {
                self.starting = None;
                self.refuse(Self::refusal(&why));
                vec![Out::Read(Read::Runs)]
            }
            Input::Resize(size) => {
                self.size = size;
                Vec::new()
            }
        }
    }

    /// After the runs were read: open what needs you when nothing is open,
    /// and send prompts kept for runs that now wait for one.
    fn settle(&mut self) -> Vec<Out> {
        let mut out = self.release_queued();
        out.extend(self.name_runs());
        if let Screen::Home { .. } = self.screen
            && let Some(run) = self.desk.next_needing(None)
        {
            out.extend(self.open(run));
        }
        out
    }

    /// Read the history of live runs not read yet, which names them by
    /// their first prompt.
    fn name_runs(&mut self) -> Vec<Out> {
        let unnamed: Vec<RunId> = self
            .desk
            .ordered()
            .iter()
            .filter(|run| {
                !run.named
                    && run
                        .recorded
                        .as_ref()
                        .is_some_and(|recorded| recorded.session.is_some())
            })
            .map(|run| run.id)
            .collect();
        unnamed
            .into_iter()
            .map(|id| {
                if let Some(run) = self.desk.get_mut(id) {
                    run.named = true;
                }
                self.history(id, false)
            })
            .collect()
    }

    fn release_queued(&mut self) -> Vec<Out> {
        let ready: Vec<RunId> = self
            .desk
            .ordered()
            .iter()
            .filter(|run| run.queued.is_some() && run.attention() == Attention::YourTurn)
            .map(|run| run.id)
            .collect();
        let mut out = Vec::new();
        for id in ready {
            if let Some(run) = self.desk.get_mut(id)
                && let Some(prompt) = run.queued.take()
            {
                run.transcript.prompted(prompt.as_str());
                out.push(Out::Send(Command::Prompt { run: id, prompt }));
            }
        }
        out
    }

    fn update(&mut self, update: RunUpdate) -> Vec<Out> {
        match update {
            RunUpdate::State { run, state } => {
                let known = self.desk.get(run).is_some();
                if !known {
                    // A run started elsewhere: its folder and client come
                    // with the next listing.
                    return vec![Out::Read(Read::Runs), Out::Read(Read::Recorded)];
                }
                if let Some(held) = self.desk.get_mut(run) {
                    held.live = Some(state);
                }
                let mut out = self.release_queued();
                // A run that ended is read again to learn how.
                if let Some(RunState::Ended { .. }) =
                    self.desk.get(run).and_then(|run| run.live.as_ref())
                {
                    out.push(Out::Read(Read::Recorded));
                }
                out
            }
            RunUpdate::Event { run, event } => {
                let Some(held) = self.desk.get_mut(run) else {
                    return vec![Out::Read(Read::Runs)];
                };
                held.transcript.apply(&event);
                match event {
                    DriveEvent::TurnEnded { .. } => {
                        held.awaiting = 1;
                        vec![self.history(run, true)]
                    }
                    DriveEvent::Ready { .. }
                    | DriveEvent::TurnStarted { .. }
                    | DriveEvent::Delta { .. }
                    | DriveEvent::Item { .. }
                    | DriveEvent::Approval { .. }
                    | DriveEvent::ApprovalResolved { .. }
                    | DriveEvent::Error { .. }
                    | DriveEvent::Refused { .. }
                    | DriveEvent::Unsupported { .. }
                    | DriveEvent::Unknown { .. }
                    | DriveEvent::Unreadable { .. } => Vec::new(),
                }
            }
        }
    }

    /// The daemon's error code for a refused command, in words.
    fn refusal(code: &str) -> String {
        match code {
            "runs_disabled" => "runs are off on the daemon: add a folder to [runs] roots",
            "invalid_directory" => "that folder is not under one of the daemon's run roots",
            "directory_not_found" => "that folder does not exist on the daemon's host",
            "not_drivable" => "Ren cannot drive that client yet",
            "spawn_failed" => "the client did not start: is it installed on the daemon's host?",
            "unrecorded" => "the recorder has no room to record a new run right now",
            "busy" => "a turn is already running",
            "starting" => "the run is still opening its conversation",
            "not_working" | "turn_not_started" => "no turn is running",
            "no_such_approval" => "that approval no longer waits",
            "stopping" | "ended" => "the run is stopping or has ended",
            "not_found" => "the daemon does not know that run",
            "unauthorized" | "forbidden" => "the daemon refused this terminal's control access",
            other => return other.to_owned(),
        }
        .to_owned()
    }
}
