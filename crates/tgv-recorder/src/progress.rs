//! What a call's response says while it streams, for live viewers: the tail
//! of the reply so far, redacted, and the tools it has begun to call. A
//! progress update is never stored and never resumed; the finished call's
//! digest is the record. Updates are throttled per call, so a fast stream
//! costs a few updates a second however many pieces it arrives in.

use crate::{id::CallId, secrets::SecretScanner, time::Monotonic};
use serde::Serialize;
use std::time::Duration;
use tgv_protocol::activity::ResponseActivity;

/// One update on a streaming call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub call: CallId,
    /// The end of the reply so far, with credentials in well-known formats
    /// redacted.
    pub reply: String,
    /// Whether the reply so far is longer than `reply`, which keeps its end.
    pub cut: bool,
    /// The tools the response has begun to call, in order.
    pub tools: Vec<String>,
    /// Whether the model is thinking: the response so far ends inside a
    /// thinking block or reasoning item, whose text is not shown.
    pub thinking: bool,
}

impl Progress {
    /// The least time between two updates on one call.
    pub const EVERY: Duration = Duration::from_millis(250);
    /// How much of the reply's end an update carries, in characters.
    pub const TAIL_CHARS: usize = 2_000;
    /// How much of the reply's end is scanned for credentials before the
    /// tail is cut from it, so a credential across the cut is still found.
    const SCAN_CHARS: usize = 2 * Self::TAIL_CHARS;

    pub(crate) fn of(
        call: CallId,
        activity: &ResponseActivity,
        thinking: bool,
        scanner: &SecretScanner,
    ) -> Self {
        let scanned = Self::tail(&activity.reply, Self::SCAN_CHARS);
        let (redacted, _) = scanner.redact(scanned);
        let reply = Self::tail(&redacted, Self::TAIL_CHARS).to_owned();
        Self {
            call,
            cut: activity.reply.chars().count() > Self::TAIL_CHARS,
            reply,
            tools: activity
                .tool_uses
                .iter()
                .map(|tool| tool.name.clone())
                .collect(),
            thinking,
        }
    }

    /// The last `chars` characters of `text`.
    fn tail(text: &str, chars: usize) -> &str {
        let count = text.chars().count();
        if count <= chars {
            return text;
        }
        let start = text
            .char_indices()
            .nth(count - chars)
            .map_or(text.len(), |(index, _)| index);
        &text[start..]
    }
}

/// When a call's next update is due.
#[derive(Debug, Default)]
pub(crate) struct Throttle {
    last: Option<Monotonic>,
    /// The reply length, tool count, and thinking last sent.
    sent: (usize, usize, bool),
}

impl Throttle {
    /// Whether an update may go out at `now`.
    pub(crate) fn due(&self, now: Monotonic) -> bool {
        self.last
            .is_none_or(|last| now.saturating_since(last) >= Progress::EVERY)
    }

    /// Whether `activity` says more than the last update, or the model
    /// started or stopped thinking, recording it as sent at `now` when so.
    pub(crate) fn changed(
        &mut self,
        now: Monotonic,
        activity: &ResponseActivity,
        thinking: bool,
    ) -> bool {
        let next = (activity.reply.len(), activity.tool_uses.len(), thinking);
        if next == self.sent {
            return false;
        }
        self.sent = next;
        self.last = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{Progress, Throttle};
    use crate::{id::CallId, secrets::SecretScanner, time::Monotonic};
    use std::time::Duration;
    use tgv_protocol::activity::ResponseActivity;
    use uuid::Uuid;

    fn activity(reply: &str) -> ResponseActivity {
        ResponseActivity {
            reply: reply.to_owned(),
            ..ResponseActivity::default()
        }
    }

    #[test]
    fn an_update_keeps_the_end_of_the_reply_and_redacts_credentials() {
        let scanner = SecretScanner::new().unwrap();
        let call = CallId::restore(Uuid::from_u128(1));
        let key = format!("sk-ant-api03-{}", "A".repeat(93));
        let long = format!("{}{key} done", "x".repeat(Progress::TAIL_CHARS));
        let update = Progress::of(call, &activity(&long), false, &scanner);
        assert!(update.cut);
        assert!(update.reply.ends_with(" done"));
        assert!(!update.reply.contains("sk-ant-api03"), "{}", update.reply);
        assert!(update.reply.contains("«redacted"));
        assert!(update.reply.chars().count() <= Progress::TAIL_CHARS);
        let short = Progress::of(call, &activity("hello"), false, &scanner);
        assert_eq!((short.reply.as_str(), short.cut), ("hello", false));
    }

    #[test]
    fn updates_are_spaced_and_only_for_new_content() {
        let at = |ms| Monotonic::from_elapsed(Duration::from_millis(ms));
        let mut throttle = Throttle::default();
        assert!(throttle.due(at(0)));
        assert!(!throttle.changed(at(0), &activity(""), false));
        assert!(throttle.changed(at(0), &activity(""), true));
        assert!(!throttle.changed(at(300), &activity(""), true));
        assert!(throttle.changed(at(600), &activity("a"), false));
        assert!(!throttle.due(at(700)));
        assert!(throttle.due(at(850)));
        assert!(!throttle.changed(at(850), &activity("a"), false));
        assert!(throttle.changed(at(860), &activity("ab"), false));
        assert!(!throttle.due(at(1000)));
    }
}
