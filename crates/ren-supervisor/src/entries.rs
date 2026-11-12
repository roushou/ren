//! What a run's conversation keeps of what its client reports: every event
//! but the streamed pieces, each in the turn it came in. A reply still
//! streaming when its turn ends (interrupted, or the process gone) is kept
//! as the text that arrived, so nothing shown live is lost from the record.

use ren_harness::drive::{DriveEvent, DriveItem, ItemId, ItemKind, ItemPhase, ItemRead};
use ren_protocol::native::NativeTurnId;

/// The run's turn, and the replies streaming in it.
#[derive(Debug, Default)]
pub(crate) struct Entries {
    turn: Option<NativeTurnId>,
    /// Text streamed into items that have not completed, in arrival order.
    streaming: Vec<(ItemId, String)>,
}

/// An event to record, in its turn.
pub(crate) type Entry = (Option<NativeTurnId>, DriveEvent);

impl Entries {
    /// What `event` adds to the conversation.
    pub fn take(&mut self, event: &DriveEvent) -> Vec<Entry> {
        match event {
            DriveEvent::Delta { item, text } => {
                match self.streaming.iter_mut().find(|(id, _)| id == item) {
                    Some((_, streamed)) => streamed.push_str(text),
                    None => self.streaming.push((item.clone(), text.clone())),
                }
                Vec::new()
            }
            // The run's opening is recorded with the run itself.
            DriveEvent::Ready { .. } => Vec::new(),
            DriveEvent::TurnStarted { turn } => {
                self.turn = Some(*turn);
                vec![(self.turn, event.clone())]
            }
            DriveEvent::Item { phase, item } => {
                if *phase == ItemPhase::Completed {
                    self.streaming.retain(|(id, _)| *id != item.id);
                }
                vec![(self.turn, event.clone())]
            }
            DriveEvent::TurnEnded { .. } => {
                let mut entries = self.unfinished();
                entries.push((self.turn, event.clone()));
                self.turn = None;
                entries
            }
            DriveEvent::Approval { .. }
            | DriveEvent::ApprovalResolved { .. }
            | DriveEvent::Error { .. }
            | DriveEvent::Refused { .. }
            | DriveEvent::Unsupported { .. }
            | DriveEvent::Unknown { .. }
            | DriveEvent::Unreadable { .. } => vec![(self.turn, event.clone())],
        }
    }

    /// The run ended: replies still streaming, as far as they came.
    pub fn finish(&mut self) -> Vec<Entry> {
        self.unfinished()
    }

    fn unfinished(&mut self) -> Vec<Entry> {
        std::mem::take(&mut self.streaming)
            .into_iter()
            .map(|(id, text)| {
                (
                    self.turn,
                    DriveEvent::Item {
                        phase: ItemPhase::Completed,
                        item: DriveItem {
                            id,
                            kind: ItemKind::AgentMessage,
                            read: ItemRead::Reply { text },
                            // Nothing came but the text.
                            body: Default::default(),
                        },
                    },
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Entries;
    use ren_harness::drive::{DriveEvent, ItemPhase, ItemRead, TurnEnd};
    use ren_protocol::native::NativeTurnId;

    fn item(id: &str) -> ren_harness::drive::ItemId {
        serde_json::from_value(serde_json::json!(id)).unwrap()
    }

    #[test]
    fn streamed_pieces_are_not_entries_but_an_unfinished_reply_is_kept() {
        let turn = NativeTurnId::new(uuid::Uuid::from_u128(7)).unwrap();
        let mut entries = Entries::default();
        assert!(
            entries
                .take(&DriveEvent::Ready {
                    thread: ren_protocol::native::NativeThreadId::new(uuid::Uuid::from_u128(1))
                        .unwrap()
                })
                .is_empty()
        );
        let started = entries.take(&DriveEvent::TurnStarted { turn });
        assert_eq!(started[0].0, Some(turn));
        for text in ["Look at the ", "lease"] {
            assert!(
                entries
                    .take(&DriveEvent::Delta {
                        item: item("msg_1"),
                        text: text.into(),
                    })
                    .is_empty()
            );
        }
        let ended = entries.take(&DriveEvent::TurnEnded {
            turn,
            end: TurnEnd::Interrupted,
        });
        let [
            (Some(first), DriveEvent::Item { phase, item: kept }),
            (Some(_), DriveEvent::TurnEnded { .. }),
        ] = ended.as_slice()
        else {
            panic!("{ended:?}");
        };
        assert_eq!((*first, *phase), (turn, ItemPhase::Completed));
        assert_eq!(
            kept.read,
            ItemRead::Reply {
                text: "Look at the lease".into()
            }
        );
        // After the turn, events have none; nothing is left to finish.
        assert!(entries.finish().is_empty());
    }
}
