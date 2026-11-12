//! A run's conversation, whole: read once from what the daemon stored of it
//! (every turn of the run, and of the runs before it on its thread), then
//! kept current by the run feed as it streams. Both say the same events, so
//! one is merged into the other by what each entry is: an item by its id, a
//! turn's end by its turn, anything else by what it says. Nothing is cut and
//! nothing gives way to a shorter copy.

use ren_harness::drive::{DriveEvent, ItemId, ItemPhase, ItemRead, ItemStatus, Retry, TurnEnd};
use ren_protocol::native::NativeTurnId;
use ren_store::view::ConversationView;

/// One thing shown in the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// The operator's prompt: as the client echoed it (its item), or as
    /// sent from here before the echo came.
    Prompt {
        item: Option<ItemId>,
        text: String,
    },
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
    TurnEnd {
        turn: NativeTurnId,
        end: TurnEnd,
    },
}

impl Entry {
    fn item(&self) -> Option<&ItemId> {
        match self {
            Self::Prompt { item, .. } => item.as_ref(),
            Self::Message { item, .. }
            | Self::Reasoning { item, .. }
            | Self::Command { item, .. }
            | Self::FileChange { item, .. }
            | Self::Tool { item, .. } => Some(item),
            Self::Notice { .. } | Self::TurnEnd { .. } => None,
        }
    }

    /// Whether it says all it will: a reply or step that finished.
    fn done(&self) -> bool {
        match self {
            Self::Message { done, .. } | Self::Reasoning { done, .. } => *done,
            Self::Command { status, .. }
            | Self::FileChange { status, .. }
            | Self::Tool { status, .. } => *status != ItemStatus::InProgress,
            Self::Prompt { .. } | Self::Notice { .. } | Self::TurnEnd { .. } => true,
        }
    }

    /// Whether `other` is the same entry, said again.
    fn same(&self, other: &Self) -> bool {
        match (self.item(), other.item(), self, other) {
            (Some(one), Some(two), _, _) => one == two,
            (_, _, Self::TurnEnd { turn: one, .. }, Self::TurnEnd { turn: two, .. }) => one == two,
            // A prompt sent from here is the client's echo of it.
            (_, _, Self::Prompt { text: one, .. }, Self::Prompt { text: two, .. }) => one == two,
            _ => self == other,
        }
    }
}

/// A run's conversation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transcript {
    entries: Vec<Entry>,
    /// The turn the newest entries belong to, once the client named it.
    turn: Option<NativeTurnId>,
    /// Whether what the daemon stored has been read.
    read: bool,
    /// The daemon kept more than one read returns: older entries are not
    /// shown.
    earlier: bool,
}

impl Transcript {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Whether what the daemon stored of it has been read.
    pub fn is_read(&self) -> bool {
        self.read
    }

    /// Whether older entries were left out of the read.
    pub fn earlier(&self) -> bool {
        self.earlier
    }

    /// What the daemon stored, with what streamed in meanwhile kept: an
    /// entry the store has is the store's, unless the live one says more.
    pub fn load(&mut self, stored: &ConversationView) {
        let mut loaded = Self::default();
        for entry in &stored.entries {
            loaded.apply(&entry.event);
        }
        for live in std::mem::take(&mut self.entries) {
            match loaded.entries.iter().position(|entry| entry.same(&live)) {
                Some(at) if live.done() && !loaded.entries[at].done() => loaded.entries[at] = live,
                Some(_) => {}
                None => loaded.entries.push(live),
            }
        }
        loaded.turn = self.turn.or(loaded.turn);
        loaded.read = true;
        loaded.earlier = stored.earlier;
        *self = loaded;
    }

    /// The prompt the operator just sent. The client's echo of it may come
    /// first, so the turn shows it once.
    pub fn prompted(&mut self, text: &str) {
        self.push(Entry::Prompt {
            item: None,
            text: text.to_owned(),
        });
    }

    /// The first prompt shown, to name the run by.
    /// Every prompt sent, oldest first, to recall.
    pub fn prompts(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Prompt { text, .. } => Some(text.clone()),
                Entry::Message { .. }
                | Entry::Reasoning { .. }
                | Entry::Command { .. }
                | Entry::FileChange { .. }
                | Entry::Tool { .. }
                | Entry::Notice { .. }
                | Entry::TurnEnd { .. } => None,
            })
            .collect()
    }

    pub fn first_prompt(&self) -> Option<&str> {
        self.entries.iter().find_map(|entry| match entry {
            Entry::Prompt { text, .. } => Some(text.as_str()),
            Entry::Message { .. }
            | Entry::Reasoning { .. }
            | Entry::Command { .. }
            | Entry::FileChange { .. }
            | Entry::Tool { .. }
            | Entry::Notice { .. }
            | Entry::TurnEnd { .. } => None,
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
            Entry::Prompt { .. }
            | Entry::Message { .. }
            | Entry::Reasoning { .. }
            | Entry::Notice { .. }
            | Entry::TurnEnd { .. } => None,
        })
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
                self.turn = Some(*turn);
                self.push(Entry::TurnEnd {
                    turn: *turn,
                    end: end.clone(),
                });
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
            ItemRead::Prompt { text } if text.is_empty() => return,
            ItemRead::Prompt { text } => Entry::Prompt {
                item: Some(id.clone()),
                text: text.clone(),
            },
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
        self.push(entry);
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

    /// Add `entry`, or put it in the place of the same entry said before.
    /// A prompt sent from here and echoed by the client is one prompt, in
    /// the current turn.
    fn push(&mut self, entry: Entry) {
        let start = match &entry {
            Entry::Prompt { .. } => self
                .entries
                .iter()
                .rposition(|entry| matches!(entry, Entry::TurnEnd { .. }))
                .map_or(0, |at| at + 1),
            Entry::Message { .. }
            | Entry::Reasoning { .. }
            | Entry::Command { .. }
            | Entry::FileChange { .. }
            | Entry::Tool { .. }
            | Entry::Notice { .. }
            | Entry::TurnEnd { .. } => 0,
        };
        let same = self.entries[start..]
            .iter()
            .rposition(|shown| match (&entry, shown) {
                // Notices repeat legitimately (a retried error); only items,
                // turns, and prompts are said again.
                (Entry::Notice { .. }, _) => false,
                _ => shown.same(&entry),
            })
            .map(|at| start + at);
        match same {
            Some(at) => {
                // The client's echo names the prompt sent from here.
                if let (Entry::Prompt { item: None, .. }, Entry::Prompt { item: Some(_), .. }) =
                    (&self.entries[at], &entry)
                {
                    self.entries[at] = entry;
                } else if !matches!(entry, Entry::Prompt { item: None, .. }) {
                    self.entries[at] = entry;
                }
            }
            None => self.entries.push(entry),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Entry, Transcript};
    use ren_harness::drive::{DriveEvent, TurnEnd};
    use ren_protocol::native::NativeTurnId;
    use ren_store::view::{ConversationView, RunEntryView};
    use serde_json::json;

    fn event(value: serde_json::Value) -> DriveEvent {
        serde_json::from_value(value).unwrap()
    }

    fn reply(id: &str, text: &str) -> DriveEvent {
        event(json!({ "kind": "item", "phase": "completed", "item": {
            "id": id, "kind": "agent_message", "read": { "as": "reply", "text": text }, "body": null } }))
    }

    fn prompt(id: &str, text: &str) -> DriveEvent {
        event(json!({ "kind": "item", "phase": "completed", "item": {
            "id": id, "kind": "user_message", "read": { "as": "prompt", "text": text }, "body": null } }))
    }

    fn stored(events: Vec<DriveEvent>) -> ConversationView {
        ConversationView {
            entries: events
                .into_iter()
                .enumerate()
                .map(|(seq, event)| {
                    serde_json::from_value::<RunEntryView>(json!({
                        "run": "00000000-0000-0000-0000-000000000001",
                        "seq": seq + 1, "at": 0, "turn": null, "event": event,
                    }))
                    .unwrap()
                })
                .collect(),
            earlier: false,
        }
    }

    #[test]
    fn the_stored_conversation_and_the_live_feed_merge_into_one() {
        let turn = NativeTurnId::new(uuid::Uuid::from_u128(5)).unwrap();
        let long = "a very long reply ".repeat(500);
        let mut transcript = Transcript::default();
        // Live, before the read lands: a prompt sent from here, its echo,
        // and the next reply streaming.
        transcript.prompted("and the docs");
        transcript.apply(&prompt("u2", "and the docs"));
        transcript.apply(&event(
            json!({ "kind": "delta", "item": "m2", "text": "Upd" }),
        ));
        assert_eq!(transcript.entries().len(), 2, "the echo is the prompt");
        transcript.load(&stored(vec![
            prompt("u1", "fix the test"),
            reply("m1", &long),
            event(json!({ "kind": "turn_ended", "turn": turn, "end": { "kind": "completed" } })),
            prompt("u2", "and the docs"),
        ]));
        let entries = transcript.entries();
        assert!(transcript.is_read());
        assert_eq!(entries.len(), 5);
        // Whole, however long.
        assert_eq!(
            entries[1],
            Entry::Message {
                item: serde_json::from_value(json!("m1")).unwrap(),
                text: long,
                done: true
            }
        );
        assert!(matches!(&entries[3], Entry::Prompt { text, .. } if text == "and the docs"));
        assert!(matches!(&entries[4], Entry::Message { text, done: false, .. } if text == "Upd"));
        // It goes on streaming and completes in place.
        transcript.apply(&event(
            json!({ "kind": "delta", "item": "m2", "text": "ated." }),
        ));
        transcript.apply(&reply("m2", "Updated."));
        assert_eq!(transcript.entries().len(), 5);
        assert_eq!(transcript.first_prompt(), Some("fix the test"));
        assert!(matches!(
            transcript.entries()[2],
            Entry::TurnEnd {
                end: TurnEnd::Completed,
                ..
            }
        ));
    }
}
