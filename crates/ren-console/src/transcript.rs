//! A run's conversation as it streams in: what the agent reported since this
//! console started following the run, read through each item's `ItemRead`.
//! Recorded turns come from history; a live turn gives way to its record
//! once that lands ([`Transcript::recorded_in`]). Live events are not stored,
//! so this is all a console knows of a turn until it is recorded.

use ren_harness::drive::{DriveEvent, ItemId, ItemPhase, ItemRead, ItemStatus, Retry, TurnEnd};
use ren_protocol::native::NativeTurnId;
use ren_store::activity::TurnActivity;

/// One thing shown in the live conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// The operator's prompt.
    Prompt(String),
    /// Reply text, growing while it streams.
    Message {
        item: ItemId,
        text: String,
        done: bool,
    },
    Reasoning {
        item: ItemId,
        done: bool,
    },
    Command {
        item: ItemId,
        command: String,
        status: ItemStatus,
        output: Option<String>,
        exit_code: Option<i64>,
    },
    FileChange {
        item: ItemId,
        paths: Vec<String>,
        status: ItemStatus,
    },
    Tool {
        item: ItemId,
        name: String,
        status: ItemStatus,
    },
    /// An error or refusal the client reported.
    Notice {
        text: String,
        retrying: bool,
    },
    TurnEnd(TurnEnd),
}

impl Entry {
    fn item(&self) -> Option<&ItemId> {
        match self {
            Self::Message { item, .. }
            | Self::Reasoning { item, .. }
            | Self::Command { item, .. }
            | Self::FileChange { item, .. }
            | Self::Tool { item, .. } => Some(item),
            Self::Prompt(_) | Self::Notice { .. } | Self::TurnEnd(_) => None,
        }
    }
}

/// A run's live conversation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transcript {
    entries: Vec<Entry>,
    /// The turn the newest entries belong to, once the agent named it.
    turn: Option<NativeTurnId>,
    /// The recorded turns already read when this turn started, by native
    /// id; `None` until the console has read the run's history.
    known: Option<Vec<String>>,
}

impl Transcript {
    /// Entries kept: the oldest go first.
    const KEPT: usize = 400;

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The recorded turns read when the current turn started.
    pub fn knowing(&mut self, known: Vec<String>) {
        self.known = Some(known);
    }

    /// Whether the recorded turns before this one are known.
    pub fn knows(&self) -> bool {
        self.known.is_some()
    }

    /// The prompt the operator just sent. The agent's echo of it may arrive
    /// first, so the current turn shows it once.
    pub fn prompted(&mut self, text: &str) {
        if !self.prompts_since_turn_end().contains(&text) {
            self.push(Entry::Prompt(text.to_owned()));
        }
    }

    /// The first prompt shown, to name the run by.
    pub fn first_prompt(&self) -> Option<&str> {
        self.entries.iter().find_map(|entry| match entry {
            Entry::Prompt(text) => Some(text.as_str()),
            Entry::Message { .. }
            | Entry::Reasoning { .. }
            | Entry::Command { .. }
            | Entry::FileChange { .. }
            | Entry::Tool { .. }
            | Entry::Notice { .. }
            | Entry::TurnEnd(_) => None,
        })
    }

    /// The newest reply text, for a one-line summary.
    pub fn last_reply(&self) -> Option<&str> {
        self.entries.iter().rev().find_map(|entry| match entry {
            Entry::Message { text, .. } if !text.trim().is_empty() => Some(text.as_str()),
            Entry::Prompt(_)
            | Entry::Message { .. }
            | Entry::Reasoning { .. }
            | Entry::Command { .. }
            | Entry::FileChange { .. }
            | Entry::Tool { .. }
            | Entry::Notice { .. }
            | Entry::TurnEnd(_) => None,
        })
    }

    /// The newest item's action, in a few words, while a turn runs.
    pub fn doing(&self) -> Option<String> {
        self.entries.iter().rev().find_map(|entry| match entry {
            Entry::Command { command, .. } => Some(format!("running {command}")),
            Entry::FileChange { paths, .. } => Some(format!(
                "editing {}",
                paths.first().map_or("files", String::as_str)
            )),
            Entry::Tool { name, .. } => Some(name.clone()),
            Entry::Message { done: false, .. } => Some("writing".to_owned()),
            Entry::Reasoning { done: false, .. } => Some("thinking".to_owned()),
            Entry::Prompt(_)
            | Entry::Message { .. }
            | Entry::Reasoning { .. }
            | Entry::Notice { .. }
            | Entry::TurnEnd(_) => None,
        })
    }

    fn prompts_since_turn_end(&self) -> Vec<&str> {
        let start = self
            .entries
            .iter()
            .rposition(|entry| matches!(entry, Entry::TurnEnd(_)))
            .map_or(0, |at| at + 1);
        self.entries[start..]
            .iter()
            .filter_map(|entry| match entry {
                Entry::Prompt(text) => Some(text.as_str()),
                Entry::Message { .. }
                | Entry::Reasoning { .. }
                | Entry::Command { .. }
                | Entry::FileChange { .. }
                | Entry::Tool { .. }
                | Entry::Notice { .. }
                | Entry::TurnEnd(_) => None,
            })
            .collect()
    }

    /// Whether `turn` ended as this transcript's last entry.
    fn ended(&self) -> bool {
        self.turn.is_some() && matches!(self.entries.last(), Some(Entry::TurnEnd(_)))
    }

    /// Whether the turn that just ended is among `turns`, the recorded
    /// ones: by its id where the client names its turns the same live and
    /// on the wire (Codex), or else (Claude Code) as a recorded turn that
    /// was not there when it started and carries its prompt, or any such
    /// turn when the console joined after the prompt was sent.
    pub fn recorded_in(&self, turns: &[TurnActivity]) -> bool {
        let Some(turn) = self.turn else {
            return false;
        };
        if !self.ended() {
            return false;
        }
        let turn = turn.to_string();
        if turns.iter().any(|recorded| recorded.native_id == turn) {
            return true;
        }
        let Some(known) = &self.known else {
            return false;
        };
        let prompt = self.ended_prompt();
        turns.iter().any(|recorded| {
            !known.contains(&recorded.native_id)
                && match (prompt, &recorded.prompt) {
                    (None, _) => true,
                    (Some(_), None) => false,
                    (Some(prompt), Some(excerpt)) if excerpt.omitted_chars > 0 => {
                        prompt.starts_with(&excerpt.text)
                    }
                    (Some(prompt), Some(excerpt)) => prompt == excerpt.text,
                }
        })
    }

    /// The prompt of the turn the transcript ends with.
    fn ended_prompt(&self) -> Option<&str> {
        let last = self.entries.len().checked_sub(1)?;
        let previous = self.entries[..last]
            .iter()
            .rposition(|entry| matches!(entry, Entry::TurnEnd(_)))
            .map_or(0, |at| at + 1);
        self.entries[previous..last]
            .iter()
            .rev()
            .find_map(|entry| match entry {
                Entry::Prompt(text) => Some(text.as_str()),
                Entry::Message { .. }
                | Entry::Reasoning { .. }
                | Entry::Command { .. }
                | Entry::FileChange { .. }
                | Entry::Tool { .. }
                | Entry::Notice { .. }
                | Entry::TurnEnd(_) => None,
            })
    }

    /// The live copy is no longer needed: its turn is recorded.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.turn = None;
    }

    pub fn apply(&mut self, event: &DriveEvent) {
        match event {
            DriveEvent::TurnStarted { turn } => self.turn = Some(*turn),
            DriveEvent::Delta { item, text } => match self.find(item) {
                Some(index) => {
                    if let Some(Entry::Message { text: shown, .. }) = self.entries.get_mut(index) {
                        shown.push_str(text);
                    }
                }
                None => self.push(Entry::Message {
                    item: item.clone(),
                    text: text.clone(),
                    done: false,
                }),
            },
            DriveEvent::Item { phase, item } => self.item(*phase, &item.id, &item.read, &item.kind),
            DriveEvent::TurnEnded { turn, end } => {
                // A console that joined mid-turn learns the turn here.
                self.turn = Some(*turn);
                self.push(Entry::TurnEnd(end.clone()));
            }
            DriveEvent::Error { message, retry } => self.push(Entry::Notice {
                text: message.clone(),
                retrying: match retry {
                    Retry::WillRetry => true,
                    Retry::Final => false,
                },
            }),
            DriveEvent::Refused { message, .. } => self.push(Entry::Notice {
                text: message.clone(),
                retrying: false,
            }),
            // Approvals live in the run's state; the rest says nothing to show.
            DriveEvent::Ready { .. }
            | DriveEvent::Approval { .. }
            | DriveEvent::ApprovalResolved { .. }
            | DriveEvent::Unsupported { .. }
            | DriveEvent::Unknown { .. }
            | DriveEvent::Unreadable { .. } => {}
        }
    }

    fn item(
        &mut self,
        phase: ItemPhase,
        id: &ItemId,
        read: &ItemRead,
        kind: &ren_harness::drive::ItemKind,
    ) {
        let done = phase == ItemPhase::Completed;
        let entry = match read {
            ItemRead::Prompt { text } => {
                // The operator's prompt, which `prompted` usually showed already.
                if !text.is_empty() {
                    self.prompted(text);
                }
                return;
            }
            ItemRead::Reply { text } => {
                let streamed = match self.find(id).and_then(|index| self.entries.get(index)) {
                    Some(Entry::Message { text, .. }) => text.clone(),
                    Some(_) | None => String::new(),
                };
                Entry::Message {
                    item: id.clone(),
                    text: if text.is_empty() {
                        streamed
                    } else {
                        text.clone()
                    },
                    done,
                }
            }
            ItemRead::Reasoning => Entry::Reasoning {
                item: id.clone(),
                done,
            },
            ItemRead::Plan { text } if text.is_empty() => return,
            ItemRead::Plan { text } => Entry::Message {
                item: id.clone(),
                text: text.clone(),
                done,
            },
            ItemRead::Command {
                command,
                status,
                output,
                exit_code,
            } => Entry::Command {
                item: id.clone(),
                command: command.clone(),
                status: status.clone(),
                output: output.clone(),
                exit_code: *exit_code,
            },
            ItemRead::FileChange { paths, status } => Entry::FileChange {
                item: id.clone(),
                paths: paths.clone(),
                status: status.clone(),
            },
            ItemRead::Tool { name, status } => Entry::Tool {
                item: id.clone(),
                name: name.clone(),
                status: status.clone(),
            },
            ItemRead::Unread => Entry::Tool {
                item: id.clone(),
                name: Self::kind_name(kind),
                status: ItemStatus::of(phase),
            },
        };
        match self.find(id) {
            Some(index) => {
                if let Some(slot) = self.entries.get_mut(index) {
                    *slot = entry;
                }
            }
            None => self.push(entry),
        }
    }

    fn kind_name(kind: &ren_harness::drive::ItemKind) -> String {
        serde_json::to_value(kind)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    fn find(&self, item: &ItemId) -> Option<usize> {
        self.entries
            .iter()
            .rposition(|entry| entry.item() == Some(item))
    }

    fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
        if self.entries.len() > Self::KEPT {
            let over = self.entries.len() - Self::KEPT;
            self.entries.drain(..over);
        }
    }
}
