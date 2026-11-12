//! The recorder's pure core: records in, in queue order; a batch of
//! operations out. It cuts bodies into chunks, inspects responses and gathers
//! requests as their bytes arrive, and seals each finished call's digest. It
//! holds no permits, reads no clock, and does no I/O: the task around it
//! receives from the queue, commits through the sink, and keeps the metrics.
//! A source test keeps it free of runtime and I/O imports.

use crate::{
    batch::{
        AttemptBody, Batch, BodyEnd, BodyKey, CloseChannel, FinishAttempt, FinishCall, HistoryEdit,
        MessageMark, Op, OpenCall, OperatorAction, ResponseHead, RunEntry, RunRecord, Segment,
        Stamp, StartAttempt,
    },
    capture::{CallRejection, TerminalRecord},
    chunk::Chunker,
    digest::{CallDigest, DigestBudget, StoredExchange},
    id::{AttemptId, CallId},
    inspection::InspectionRecord,
    progress::{Progress, Throttle},
    secrets::SecretScanner,
    time::Monotonic,
};
use bytes::Bytes;
use ren_harness::{Harness, tools::InterpretedCall};
use ren_protocol::{
    CallInspector,
    activity::{ActivityLimits, Reading, RequestReader, ResponseActivity},
    inspect::pipeline::{InspectionLimits, Protocol},
    route::Endpoint,
};
use std::{collections::HashMap, sync::Arc};

/// One record, as the queue delivers it, without the permits it carried.
#[derive(Debug)]
pub(crate) enum Record {
    Open {
        open: Box<OpenCall>,
        /// The first attempt; absent for a call refused before dispatch.
        attempt: Option<StartAttempt>,
        endpoint: Endpoint,
    },
    /// A later attempt of an open call (failover).
    Attempt(StartAttempt),
    Head(ResponseHead),
    Data(Piece),
    /// A WebSocket channel ended.
    Close(CloseChannel),
    FinishAttempt {
        stamp: Stamp,
        attempt: AttemptId,
        end: TerminalRecord,
        response: BodyEnd,
        /// The changed request body's end, if the attempt sent one.
        request: Option<BodyEnd>,
    },
    FinishCall {
        stamp: Stamp,
        request: BodyEnd,
        rejection: Option<CallRejection>,
    },
    Operator(OperatorAction),
    Run(RunRecord),
    RunEntry(Box<RunEntry>),
    History(HistoryEdit),
}

impl Record {
    /// When the record was made, for the queue's lag.
    pub(crate) fn at(&self) -> Monotonic {
        match self {
            Self::Open { open, .. } => open.stamp.at.monotonic,
            Self::Attempt(attempt) => attempt.stamp.at.monotonic,
            Self::Head(head) => head.stamp.at.monotonic,
            Self::Data(piece) => piece.enqueued,
            Self::Close(close) => close.at.monotonic,
            Self::FinishAttempt { stamp, .. } | Self::FinishCall { stamp, .. } => {
                stamp.at.monotonic
            }
            Self::Operator(action) => action.at.monotonic,
            Self::Run(record) => record.at.monotonic,
            Self::History(edit) => edit.at.monotonic,
            Self::RunEntry(entry) => entry.at.monotonic,
        }
    }
}

/// Which body a piece belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Track {
    /// The call's request, as the client sent it.
    Request,
    /// The current attempt's request, as a transform changed it.
    AttemptRequest,
    Response,
}

/// A piece of body data at its offset in the body.
#[derive(Debug)]
pub(crate) struct Piece {
    pub call: CallId,
    pub track: Track,
    pub offset: u64,
    pub bytes: Bytes,
    pub enqueued: Monotonic,
    /// Set on the first piece of a WebSocket message: the whole message.
    pub message: Option<MessageMark>,
}

/// The calls in progress and the batch being built.
pub(crate) struct RecorderCore {
    calls: HashMap<CallId, CallInProgress>,
    batch: Batch,
    /// Updates on streaming calls since the last commit.
    progress: Vec<Progress>,
    /// Redacts every digest's text before it is stored.
    scanner: Arc<SecretScanner>,
}

/// Recorder state for a call between its open and finish records.
struct CallInProgress {
    protocol: Option<Protocol>,
    /// The harness whose hints attributed the call.
    harness: Option<Harness>,
    /// The request as it is captured, read when it ends.
    reading: Option<RequestReader>,
    /// What the latest attempt's response showed: the final one's, once the
    /// call finishes.
    response_activity: ResponseActivity,
    request: BodyCapture,
    /// The open attempt's changed request, if it sends one.
    attempt_request: Option<BodyCapture>,
    response: BodyCapture,
    inspector: Option<CallInspector>,
    /// When the call's next progress update is due.
    throttle: Throttle,
}

/// One body being cut into chunks.
struct BodyCapture {
    key: BodyKey,
    chunker: Chunker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Continuity {
    Contiguous,
    /// Bytes before this piece were omitted.
    AfterGap,
}

impl BodyCapture {
    fn new(key: BodyKey) -> Self {
        Self {
            key,
            chunker: Chunker::at(0),
        }
    }

    fn push(&mut self, offset: u64, bytes: &[u8], batch: &mut Batch) -> Continuity {
        let continuity = if offset == self.chunker.next_offset() {
            Continuity::Contiguous
        } else {
            self.flush(batch);
            self.chunker = Chunker::at(offset);
            Continuity::AfterGap
        };
        let mut cuts = Vec::new();
        self.chunker.push(bytes, &mut cuts);
        self.emit(cuts, batch);
        continuity
    }

    fn flush(&mut self, batch: &mut Batch) {
        let mut cuts = Vec::new();
        self.chunker.flush(&mut cuts);
        self.emit(cuts, batch);
    }

    fn emit(&self, cuts: Vec<crate::chunk::Cut>, batch: &mut Batch) {
        if cuts.is_empty() {
            return;
        }
        let segments = cuts
            .into_iter()
            .map(|cut| Segment {
                offset: cut.offset,
                chunk: batch.add_chunk(cut.bytes),
            })
            .collect();
        batch.push(Op::Segments {
            body: self.key,
            segments,
        });
    }

    /// The end of the contiguous bytes received since the last gap.
    fn received(&self) -> u64 {
        self.chunker.next_offset()
    }
}

impl RecorderCore {
    pub(crate) fn new(scanner: Arc<SecretScanner>) -> Self {
        Self {
            calls: HashMap::new(),
            batch: Batch::default(),
            progress: Vec::new(),
            scanner,
        }
    }

    /// The batch built so far, to decide whether to commit early.
    pub(crate) fn pending(&self) -> &Batch {
        &self.batch
    }

    /// The batch to commit, with every streaming response cut where it is,
    /// so live previews advance; requests keep content-defined cuts, where
    /// deduplication matters. The core starts an empty batch.
    pub(crate) fn take_batch(&mut self) -> Batch {
        for call in self.calls.values_mut() {
            call.response.flush(&mut self.batch);
        }
        std::mem::take(&mut self.batch)
    }

    /// The progress updates since the last commit.
    pub(crate) fn take_progress(&mut self) -> Vec<Progress> {
        std::mem::take(&mut self.progress)
    }

    /// Digests derived again from stored bodies.
    pub(crate) fn rederived(&mut self, exchanges: Vec<StoredExchange>) {
        for exchange in exchanges {
            let call = exchange.call;
            let digest = exchange.digest(&self.scanner);
            self.batch.push(Op::Digest {
                call,
                digest: Box::new(digest),
            });
        }
    }

    /// Take one record, in queue order.
    pub(crate) fn accept(&mut self, record: Record) {
        match record {
            Record::Open {
                open,
                attempt,
                endpoint,
            } => {
                let call = open.stamp.call;
                let protocol = Protocol::for_endpoint(endpoint);
                self.calls.insert(
                    call,
                    CallInProgress {
                        protocol,
                        harness: open
                            .attribution
                            .hint
                            .as_ref()
                            .map(|hint| hint.source.harness()),
                        reading: protocol.map(|protocol| {
                            RequestReader::new(protocol, ActivityLimits::default())
                        }),
                        response_activity: ResponseActivity::default(),
                        request: BodyCapture::new(BodyKey::Request(call)),
                        attempt_request: None,
                        // Replaced when the first attempt starts.
                        response: BodyCapture::new(BodyKey::Request(call)),
                        inspector: None,
                        throttle: Throttle::default(),
                    },
                );
                self.batch.push(Op::OpenCall(*open));
                if let Some(attempt) = attempt {
                    self.attempt(attempt);
                }
            }
            Record::Attempt(attempt) => {
                self.attempt(attempt);
            }
            Record::Head(head) => {
                if let Some(call) = self.calls.get_mut(&head.stamp.call) {
                    call.inspector = call.protocol.map(|protocol| {
                        CallInspector::new(
                            protocol,
                            head.format,
                            head.encoding,
                            InspectionLimits::default(),
                        )
                    });
                }
                self.batch.push(Op::ResponseHead(head));
            }
            Record::Data(piece) => self.data(piece),
            Record::Close(close) => self.batch.push(Op::CloseChannel(close)),
            Record::Operator(action) => self.batch.push(Op::Operator(action)),
            Record::Run(record) => self.batch.push(Op::Run(record)),
            Record::History(edit) => self.batch.push(Op::History(edit)),
            Record::RunEntry(entry) => self.batch.push(Op::RunEntry(*entry)),
            Record::FinishAttempt {
                stamp,
                attempt,
                end,
                response,
                request,
            } => {
                let inspection = self.calls.get_mut(&stamp.call).and_then(|call| {
                    if let Some(mut changed) = call.attempt_request.take() {
                        changed.flush(&mut self.batch);
                    }
                    call.response.flush(&mut self.batch);
                    let mut inspector = call.inspector.take()?;
                    // Observed bytes that never reached the recorder are a gap.
                    if call.response.received() < response.observed_bytes {
                        inspector.gap();
                    }
                    let reading = inspector.finish(response.ending.end());
                    call.response_activity = reading.activity;
                    Some(Box::new(InspectionRecord::from(&reading.inspection)))
                });
                self.batch.push(Op::FinishAttempt(FinishAttempt {
                    stamp,
                    attempt,
                    end,
                    response,
                    request,
                    inspection,
                }));
            }
            Record::FinishCall {
                stamp,
                request,
                rejection,
            } => {
                let digest = self.calls.remove(&stamp.call).map(|mut call| {
                    call.request.flush(&mut self.batch);
                    let reading = match call.reading {
                        Some(mut reader) => {
                            // Observed bytes that never reached the recorder are a gap.
                            if call.request.received() < request.observed_bytes {
                                reader.gap();
                            }
                            Reading {
                                request: reader.finish(request.ending.end()),
                                response: call.response_activity,
                            }
                        }
                        None => Reading::unsupported(),
                    };
                    CallDigest::seal(
                        InterpretedCall::new(call.harness, reading),
                        &self.scanner,
                        DigestBudget::STORED,
                    )
                });
                self.batch.push(Op::FinishCall(FinishCall {
                    stamp,
                    request,
                    rejection,
                }));
                if let Some(digest) = digest {
                    self.batch.push(Op::Digest {
                        call: stamp.call,
                        digest: Box::new(digest),
                    });
                }
            }
        }
    }

    /// An attempt started: its bodies are captured from here on.
    fn attempt(&mut self, attempt: StartAttempt) {
        if let Some(call) = self.calls.get_mut(&attempt.stamp.call) {
            call.response = BodyCapture::new(BodyKey::Response(attempt.attempt));
            call.attempt_request = match attempt.body {
                AttemptBody::Original => None,
                AttemptBody::Changed => {
                    Some(BodyCapture::new(BodyKey::AttemptRequest(attempt.attempt)))
                }
            };
            call.inspector = None;
        }
        self.batch.push(Op::StartAttempt(attempt));
    }

    fn data(&mut self, piece: Piece) {
        let Some(call) = self.calls.get_mut(&piece.call) else {
            return;
        };
        let body = match piece.track {
            Track::Request => &mut call.request,
            Track::Response => &mut call.response,
            Track::AttemptRequest => match &mut call.attempt_request {
                Some(body) => body,
                None => return,
            },
        };
        if let Some(mark) = piece.message {
            self.batch.push(Op::Message {
                body: body.key,
                mark,
            });
        }
        let continuity = body.push(piece.offset, &piece.bytes, &mut self.batch);
        match piece.track {
            Track::AttemptRequest => {}
            Track::Request => {
                if let Some(reader) = &mut call.reading {
                    match continuity {
                        Continuity::Contiguous => {}
                        Continuity::AfterGap => reader.gap(),
                    }
                    if piece.message.is_some() {
                        reader.message();
                    }
                    reader.feed(&piece.bytes);
                }
            }
            Track::Response => {
                if let Some(inspector) = &mut call.inspector {
                    match continuity {
                        Continuity::Contiguous => {}
                        Continuity::AfterGap => inspector.gap(),
                    }
                    if let Some(mark) = piece.message {
                        inspector.message(mark.len);
                    }
                    inspector.feed(&piece.bytes);
                    // Read the reply so far only when an update may go out.
                    if call.throttle.due(piece.enqueued)
                        && let Some(activity) = inspector.so_far()
                        && call
                            .throttle
                            .changed(piece.enqueued, &activity, inspector.thinking())
                    {
                        self.progress.push(Progress::of(
                            piece.call,
                            &activity,
                            inspector.thinking(),
                            &self.scanner,
                        ));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
