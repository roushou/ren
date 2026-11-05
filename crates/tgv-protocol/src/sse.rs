//! Server-sent event framing, independent of payload format and provider.
//!
//! Follows the WHATWG event-stream parsing rules: CR, LF, and CRLF line
//! endings; a leading BOM; comments; multi-line `data` joined with LF; and
//! dispatch on a blank line. Input arrives as arbitrary byte fragments: chunk
//! boundaries never affect the output. Payload bytes are opaque.

use std::str;

/// Bounds for one stream. Exceeding them discards the affected event and is
/// reported, never silently truncated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SseLimits {
    /// Longest accepted line, and the largest accumulated `data` of one event.
    pub max_event_bytes: usize,
    /// Longest accepted `event` name.
    pub max_event_name_bytes: usize,
}

impl Default for SseLimits {
    fn default() -> Self {
        Self {
            max_event_bytes: 1024 * 1024,
            max_event_name_bytes: 256,
        }
    }
}

/// The `event` field of one dispatched event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventName<'a> {
    Absent,
    Present(&'a str),
    /// Not UTF-8, or longer than the limit. The event is still dispatched;
    /// consumers decide whether a name was required.
    Invalid,
}

/// Why an event was dropped by the framer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discard {
    /// A line exceeded `max_event_bytes`.
    LineTooLong,
    /// The event's accumulated data exceeded `max_event_bytes`.
    EventTooLarge,
}

/// One framer output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseOutput<'a> {
    Event { name: EventName<'a>, data: &'a [u8] },
    Discarded(Discard),
}

/// How the stream ended, from the framer's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    /// Ended on an event boundary.
    Clean,
    /// Fields or a partial line arrived after the last dispatch; per the spec
    /// that event is not dispatched.
    PartialEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineState {
    Normal,
    /// The previous byte was CR: a following LF belongs to the same line end.
    AfterCr,
    /// The current line exceeded the limit; skip bytes until the line ends.
    Overflowing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamStart {
    /// A BOM is stripped only from the very first line.
    AwaitingFirstLine,
    Started,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventState {
    /// No field lines since the last dispatch.
    Empty,
    Collecting,
    /// The current event exceeded a limit; drop it at the next blank line.
    Discarding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NameField {
    Absent,
    Present(String),
    Invalid,
}

#[derive(Debug)]
pub struct SseFramer {
    limits: SseLimits,
    line: Vec<u8>,
    line_state: LineState,
    start: StreamStart,
    event: EventState,
    data: Vec<u8>,
    name: NameField,
}

impl SseFramer {
    pub fn new(limits: SseLimits) -> Self {
        Self {
            limits,
            line: Vec::new(),
            line_state: LineState::Normal,
            start: StreamStart::AwaitingFirstLine,
            event: EventState::Empty,
            data: Vec::new(),
            name: NameField::Absent,
        }
    }

    /// Feed the next fragment. `emit` is called for every dispatched or
    /// discarded event, in stream order.
    pub fn feed(&mut self, bytes: &[u8], mut emit: impl FnMut(SseOutput<'_>)) {
        for &byte in bytes {
            match (self.line_state, byte) {
                (LineState::AfterCr, b'\n') => self.line_state = LineState::Normal,
                (LineState::Normal | LineState::AfterCr | LineState::Overflowing, b'\r') => {
                    self.end_line(&mut emit);
                    self.line_state = LineState::AfterCr;
                }
                (LineState::Normal | LineState::Overflowing, b'\n') => {
                    self.end_line(&mut emit);
                    self.line_state = LineState::Normal;
                }
                (LineState::Overflowing, _) => {}
                (LineState::Normal | LineState::AfterCr, _) => {
                    self.line_state = LineState::Normal;
                    if self.line.len() == self.limits.max_event_bytes {
                        // An overlong comment is skipped; an overlong field
                        // line makes its event unreliable.
                        if !self.line_is_comment() {
                            self.discard(Discard::LineTooLong, &mut emit);
                        }
                        self.line.clear();
                        self.line_state = LineState::Overflowing;
                    } else {
                        self.line.push(byte);
                    }
                }
            }
        }
    }

    /// Whether the stream ended on an event boundary.
    pub fn end(&self) -> StreamEnd {
        let partial_line = self.line_state == LineState::Overflowing
            || (!self.line.is_empty() && !self.line_is_comment());
        if partial_line || self.event != EventState::Empty {
            StreamEnd::PartialEvent
        } else {
            StreamEnd::Clean
        }
    }

    fn line_is_comment(&self) -> bool {
        let line = match self.start {
            StreamStart::AwaitingFirstLine => Self::strip_bom(&self.line),
            StreamStart::Started => &self.line,
        };
        line.first() == Some(&b':')
    }

    fn strip_bom(line: &[u8]) -> &[u8] {
        line.strip_prefix(b"\xef\xbb\xbf").unwrap_or(line)
    }

    fn discard(&mut self, reason: Discard, emit: &mut impl FnMut(SseOutput<'_>)) {
        if self.event != EventState::Discarding {
            emit(SseOutput::Discarded(reason));
        }
        self.event = EventState::Discarding;
        self.data.clear();
    }

    fn end_line(&mut self, emit: &mut impl FnMut(SseOutput<'_>)) {
        if self.line_state == LineState::Overflowing {
            self.start = StreamStart::Started;
            return;
        }
        let line = std::mem::take(&mut self.line);
        let content = match self.start {
            StreamStart::AwaitingFirstLine => Self::strip_bom(&line),
            StreamStart::Started => &line,
        };
        self.start = StreamStart::Started;
        if content.is_empty() {
            self.dispatch(emit);
        } else if content.first() != Some(&b':') {
            self.field(content, emit);
        }
        // Reuse the allocation for the next line.
        self.line = line;
        self.line.clear();
    }

    fn dispatch(&mut self, emit: &mut impl FnMut(SseOutput<'_>)) {
        if self.event == EventState::Collecting && !self.data.is_empty() {
            // Every data line appended a trailing LF; the last one is not data.
            self.data.pop();
            let name = match &self.name {
                NameField::Absent => EventName::Absent,
                NameField::Present(name) => EventName::Present(name),
                NameField::Invalid => EventName::Invalid,
            };
            emit(SseOutput::Event {
                name,
                data: &self.data,
            });
        }
        self.data.clear();
        self.name = NameField::Absent;
        self.event = EventState::Empty;
    }

    fn field(&mut self, line: &[u8], emit: &mut impl FnMut(SseOutput<'_>)) {
        if self.event == EventState::Discarding {
            return;
        }
        self.event = EventState::Collecting;
        let (key, value) = match line.iter().position(|byte| *byte == b':') {
            Some(index) => {
                let value = &line[index + 1..];
                (&line[..index], value.strip_prefix(b" ").unwrap_or(value))
            }
            None => (line, &[][..]),
        };
        match key {
            b"data" => {
                let needed = self
                    .data
                    .len()
                    .saturating_add(value.len())
                    .saturating_add(1);
                if needed > self.limits.max_event_bytes {
                    self.discard(Discard::EventTooLarge, emit);
                } else {
                    self.data.extend_from_slice(value);
                    self.data.push(b'\n');
                }
            }
            b"event" => {
                self.name = match str::from_utf8(value) {
                    Ok(name) if name.len() <= self.limits.max_event_name_bytes => {
                        NameField::Present(name.to_owned())
                    }
                    Ok(_) | Err(_) => NameField::Invalid,
                };
            }
            // `id`, `retry`, and unknown fields do not affect inspection.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Discard, EventName, SseFramer, SseLimits, SseOutput, StreamEnd};
    use proptest::prelude::*;

    /// Owned framer output, for comparing runs.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Owned {
        Event(Option<Option<String>>, Vec<u8>),
        Discarded(Discard),
    }

    fn run(limits: SseLimits, chunks: &[&[u8]]) -> (Vec<Owned>, StreamEnd) {
        let mut framer = SseFramer::new(limits);
        let mut out = Vec::new();
        for chunk in chunks {
            framer.feed(chunk, |output| {
                out.push(match output {
                    SseOutput::Event { name, data } => Owned::Event(
                        match name {
                            EventName::Absent => Some(None),
                            EventName::Present(name) => Some(Some(name.to_owned())),
                            EventName::Invalid => None,
                        },
                        data.to_vec(),
                    ),
                    SseOutput::Discarded(reason) => Owned::Discarded(reason),
                });
            });
        }
        (out, framer.end())
    }

    fn split<'a>(bytes: &'a [u8], cuts: &[usize]) -> Vec<&'a [u8]> {
        let mut positions: Vec<usize> = cuts.iter().map(|cut| cut % (bytes.len() + 1)).collect();
        positions.sort_unstable();
        let mut chunks = Vec::new();
        let mut start = 0;
        for position in positions {
            chunks.push(&bytes[start..position]);
            start = position;
        }
        chunks.push(&bytes[start..]);
        chunks
    }

    #[test]
    fn frames_bom_comments_multiline_data_and_every_line_ending() {
        let bytes = b"\xef\xbb\xbf: comment\r\nevent: custom\r\ndata: not JSON\rdata: \xff\n\ndata: [DONE]\n\ndata:no-space\n\ndata: unfinished";
        let (out, end) = run(SseLimits::default(), &[bytes]);
        assert_eq!(
            out,
            [
                Owned::Event(Some(Some("custom".into())), b"not JSON\n\xff".to_vec()),
                Owned::Event(Some(None), b"[DONE]".to_vec()),
                Owned::Event(Some(None), b"no-space".to_vec()),
            ]
        );
        assert_eq!(end, StreamEnd::PartialEvent);
    }

    #[test]
    fn empty_data_dispatches_and_events_without_data_do_not() {
        let (out, end) = run(SseLimits::default(), &[b"data:\n\nevent: only\n\ndata\n\n"]);
        assert_eq!(
            out,
            [
                Owned::Event(Some(None), Vec::new()),
                Owned::Event(Some(None), Vec::new()),
            ]
        );
        assert_eq!(end, StreamEnd::Clean);
    }

    #[test]
    fn stream_end_distinguishes_pending_fields_from_trailing_comments() {
        assert_eq!(
            run(SseLimits::default(), &[b"data: x\n\n: tail"]).1,
            StreamEnd::Clean
        );
        assert_eq!(
            run(SseLimits::default(), &[b"event: pending\n"]).1,
            StreamEnd::PartialEvent
        );
        assert_eq!(
            run(SseLimits::default(), &[b"data: x\n"]).1,
            StreamEnd::PartialEvent
        );
        assert_eq!(run(SseLimits::default(), &[b""]).1, StreamEnd::Clean);
    }

    #[test]
    fn oversized_lines_and_events_are_discarded_until_the_next_boundary() {
        let limits = SseLimits {
            max_event_bytes: 12,
            max_event_name_bytes: 4,
        };
        let (out, end) = run(
            limits,
            &[b": a comment far longer than the limit\ndata: 0123456789ab\ndata: ok\n\ndata: abcdef\ndata: ghijkl\n\nevent: names\ndata: x\n\ndata: y\n\n"],
        );
        assert_eq!(
            out,
            [
                Owned::Discarded(Discard::LineTooLong),
                Owned::Discarded(Discard::EventTooLarge),
                Owned::Event(None, b"x".to_vec()),
                Owned::Event(Some(None), b"y".to_vec()),
            ]
        );
        assert_eq!(end, StreamEnd::Clean);
    }

    /// Bytes drawn mostly from the characters that matter to framing.
    fn stream_bytes() -> impl Strategy<Value = Vec<u8>> {
        let token = prop_oneof![
            Just(b"\r".to_vec()),
            Just(b"\n".to_vec()),
            Just(b"\r\n".to_vec()),
            Just(b"data:".to_vec()),
            Just(b"data: ".to_vec()),
            Just(b"event: ".to_vec()),
            Just(b": ".to_vec()),
            Just(b"\xef\xbb\xbf".to_vec()),
            Just(b"id: 1".to_vec()),
            proptest::collection::vec(any::<u8>(), 1..6),
        ];
        proptest::collection::vec(token, 0..64).prop_map(|tokens| tokens.concat())
    }

    proptest! {
        #[test]
        fn output_is_independent_of_chunk_boundaries(
            bytes in stream_bytes(),
            cuts in proptest::collection::vec(any::<usize>(), 0..24),
            limit in 1usize..64,
        ) {
            let limits = SseLimits { max_event_bytes: limit, max_event_name_bytes: 8 };
            let whole = run(limits, &[&bytes]);
            let fragmented = run(limits, &split(&bytes, &cuts));
            prop_assert_eq!(&whole, &fragmented);
            let bytewise: Vec<&[u8]> = bytes.chunks(1).collect();
            prop_assert_eq!(whole, run(limits, &bytewise));
        }
    }
}
