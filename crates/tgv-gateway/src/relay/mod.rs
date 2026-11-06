//! The imperative shell around [`HttpAttempt`]: it turns body polls, dispatch
//! results, cancellation, and drops into events, and runs the actions the
//! machine returns. Every byte passes through unchanged; capture only copies.

use super::upstream::UpstreamBody;
use crate::attempt::{
    AttemptAction, AttemptEnd, Canceller, EndKind, HttpAttempt, HttpEvent, IllegalTransition,
};
use axum::body::Body;
use bytes::Bytes;
use futures_util::future::BoxFuture;
use http_body::{Body as _, Frame, SizeHint};
use std::{
    io,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
    task::{Context, Poll},
};
use tgv_protocol::{
    direction::Direction,
    encoding::{ResponseEncoding, ResponseFormat},
};
use tgv_recorder::{
    CallTap,
    capture::{AttemptPhase, TerminalRecord},
};
use tokio::sync::OwnedSemaphorePermit;
use tokio_util::sync::CancellationToken;

/// Whether an attempt's end also ends its call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Closing {
    /// The last attempt: its end is the call's.
    Call,
    /// Another attempt follows (failover); the call stays open.
    AttemptOnly,
}

/// One attempt's lifecycle, capture, cancellation, and admission. Shared by the
/// upload body, the dispatch, and the response body.
pub(super) struct AttemptCell {
    state: Mutex<HttpAttempt>,
    tap: Arc<CallTap>,
    /// Cancelled when the attempt ends, which stops an upload still streaming
    /// and a dispatch still waiting.
    ended: CancellationToken,
    /// The daemon's shutdown abort: cancel whatever is still running.
    abort: CancellationToken,
    /// The operator's cancel for this call.
    operator: CancellationToken,
    closing: Mutex<Closing>,
    admission: Mutex<Option<OwnedSemaphorePermit>>,
    /// Kept until the attempt ends: the call's control registration.
    held: Mutex<Option<Box<dyn Send>>>,
}

impl AttemptCell {
    pub(super) fn new(
        tap: Arc<CallTap>,
        admission: Option<OwnedSemaphorePermit>,
        abort: CancellationToken,
        operator: CancellationToken,
    ) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(HttpAttempt::DISPATCHED),
            tap,
            ended: CancellationToken::new(),
            abort,
            operator,
            closing: Mutex::new(Closing::Call),
            admission: Mutex::new(admission),
            held: Mutex::new(None),
        })
    }

    /// The next end is this attempt's only; take back the call's admission
    /// for the next attempt.
    pub(super) fn hand_over(&self) -> Option<OwnedSemaphorePermit> {
        *self.closing.lock().unwrap_or_else(PoisonError::into_inner) = Closing::AttemptOnly;
        self.admission
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Keep `value` alive until the attempt ends.
    pub(super) fn hold(&self, value: Box<dyn Send>) {
        *self.held.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
    }

    pub(super) async fn operator_cancelled(&self) {
        self.operator.cancelled().await;
    }

    /// Feed an event and run the resulting actions. Events after the end are
    /// expected races (a send error after a cancel) and are dropped here, by
    /// decision; any other illegal event is a relay bug and is logged.
    pub(super) fn observe(&self, event: HttpEvent) {
        let actions = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            match state.step(event) {
                Ok(transition) => {
                    *state = transition.state;
                    transition.actions
                }
                Err(IllegalTransition::AfterEnd { .. }) => return,
                Err(error) => {
                    tracing::warn!(%error, ?event, "relay produced an illegal attempt event");
                    return;
                }
            }
        };
        for action in actions {
            match action {
                AttemptAction::Finish(end) => self.finish(end),
            }
        }
    }

    fn finish(&self, end: AttemptEnd) {
        match *self.closing.lock().unwrap_or_else(PoisonError::into_inner) {
            Closing::Call => self.tap.finish(TerminalRecord::from(end)),
            Closing::AttemptOnly => self.tap.finish_attempt(TerminalRecord::from(end)),
        }
        self.ended.cancel();
        drop(
            self.held
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        // Release admission now, even if the client keeps the finished stream.
        drop(
            self.admission
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
    }

    /// How the attempt ended, once it has.
    pub(super) fn end(&self) -> Option<AttemptEnd> {
        match *self.state.lock().unwrap_or_else(PoisonError::into_inner) {
            HttpAttempt::Ended(end) => Some(end),
            HttpAttempt::Dispatched { .. } | HttpAttempt::Streaming { .. } => None,
        }
    }

    pub(super) async fn ended(&self) {
        self.ended.cancelled().await;
    }

    pub(super) async fn aborted(&self) {
        self.abort.cancelled().await;
    }

    pub(super) fn response_head(&self, response: &http::Response<UpstreamBody>) {
        self.tap.response_head(
            response.status().as_u16(),
            ResponseFormat::from_headers(response.headers()),
            ResponseEncoding::from_headers(response.headers()),
        );
    }
}

/// Owns the attempt while the handler waits for the response head. Dropping
/// it unresolved (the client's connection closed) is the `Dropped` event.
pub(super) struct Dispatch(Option<Arc<AttemptCell>>);

impl Dispatch {
    pub(super) fn new(cell: Arc<AttemptCell>) -> Self {
        Self(Some(cell))
    }

    /// The upstream answered. Ownership passes to the response body.
    pub(super) fn responded(
        mut self,
        response: http::Response<UpstreamBody>,
    ) -> Result<http::Response<Body>, AttemptEnd> {
        let Some(cell) = self.0.take() else {
            return Err(Self::UNREACHABLE);
        };
        cell.observe(HttpEvent::Responded {
            status: response.status().as_u16(),
        });
        if let Some(end) = cell.end() {
            // The attempt ended while the head was in flight; its end stands.
            return Err(end);
        }
        cell.response_head(&response);
        let declared = ObservedResponse::declared_length(response.headers());
        let (parts, body) = response.into_parts();
        Ok(http::Response::from_parts(
            parts,
            ObservedResponse::wrap(body, cell, declared),
        ))
    }

    /// Dispatch stopped without a head: apply why, and report the end.
    pub(super) fn stopped(mut self, event: HttpEvent) -> AttemptEnd {
        let Some(cell) = self.0.take() else {
            return Self::UNREACHABLE;
        };
        cell.observe(event);
        cell.end().unwrap_or(Self::UNREACHABLE)
    }

    /// The attempt already ended through another owner (its upload failed);
    /// report that end without a new event.
    pub(super) fn concluded(mut self) -> AttemptEnd {
        self.0
            .take()
            .and_then(|cell| cell.end())
            .unwrap_or(Self::UNREACHABLE)
    }

    /// Reported only if a dispatch is resolved twice, which ownership forbids.
    const UNREACHABLE: AttemptEnd = AttemptEnd {
        how: EndKind::Interrupted,
        phase: AttemptPhase::AwaitingHead,
    };
}

impl Drop for Dispatch {
    fn drop(&mut self) {
        if let Some(cell) = self.0.take() {
            cell.observe(HttpEvent::Dropped);
        }
    }
}

/// Whether a body wrapper still reports events for its attempt.
enum Watch {
    Streaming(Arc<AttemptCell>),
    Done,
}

/// The request body on its way upstream. It reports upload events but never
/// owns the attempt. The gateway buffered and recorded the body already, so
/// the upload records nothing itself.
pub(super) struct ObservedUpload {
    inner: Body,
    stopped: BoxFuture<'static, ()>,
    watch: Watch,
}

impl ObservedUpload {
    pub(super) fn wrap(inner: Body, cell: Arc<AttemptCell>) -> Body {
        let ended = cell.ended.clone();
        let mut upload = Self {
            inner,
            stopped: Box::pin(ended.cancelled_owned()),
            watch: Watch::Streaming(cell),
        };
        if upload.inner.is_end_stream() {
            upload.ended();
        }
        Body::new(upload)
    }

    fn ended(&mut self) {
        if let Watch::Streaming(cell) = std::mem::replace(&mut self.watch, Watch::Done) {
            cell.observe(HttpEvent::UploadEnded);
        }
    }

    fn failed(
        &mut self,
        event: Option<HttpEvent>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        if let (Watch::Streaming(cell), Some(event)) =
            (std::mem::replace(&mut self.watch, Watch::Done), event)
        {
            cell.observe(event);
        }
        Poll::Ready(Some(Err(io::Error::other("request body stopped"))))
    }
}

impl http_body::Body for ObservedUpload {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let this = self.get_mut();
        let Watch::Streaming(_) = &this.watch else {
            return Poll::Ready(None);
        };
        // The attempt ended (its response finished or dispatch failed): the
        // rest of the upload has nowhere to go.
        if this.stopped.as_mut().poll(cx).is_ready() {
            return this.failed(None);
        }
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(frame))) => {
                if this.inner.is_end_stream() {
                    this.ended();
                }
                Poll::Ready(Some(Ok(frame)))
            }
            // The client's body failed: it went away mid-upload.
            Poll::Ready(Some(Err(_))) => this.failed(Some(HttpEvent::UploadFailed)),
            Poll::Ready(None) => {
                this.ended();
                Poll::Ready(None)
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        matches!(self.watch, Watch::Done)
    }

    fn size_hint(&self) -> SizeHint {
        if self.is_end_stream() {
            SizeHint::with_exact(0)
        } else {
            self.inner.size_hint()
        }
    }
}

/// The upstream's response body on its way to the client. It owns the attempt:
/// its end, failure, cancellation, or drop is the attempt's terminal.
struct ObservedResponse {
    inner: UpstreamBody,
    aborted: BoxFuture<'static, ()>,
    operator: BoxFuture<'static, ()>,
    watch: Watch,
    /// The body length the response head declared, if any.
    declared: Option<u64>,
    /// Body bytes passed on so far.
    relayed: u64,
}

impl ObservedResponse {
    fn wrap(inner: UpstreamBody, cell: Arc<AttemptCell>, declared: Option<u64>) -> Body {
        let abort = cell.abort.clone();
        let operator = cell.operator.clone();
        let mut response = Self {
            inner,
            aborted: Box::pin(abort.cancelled_owned()),
            operator: Box::pin(operator.cancelled_owned()),
            watch: Watch::Streaming(cell),
            declared,
            relayed: 0,
        };
        // An empty body ends without a poll; a client may never poll it.
        if response.inner.is_end_stream() {
            response.report(HttpEvent::BodyEnded);
        }
        Body::new(response)
    }

    /// `content-length`, when it is one decimal length.
    fn declared_length(headers: &http::HeaderMap) -> Option<u64> {
        let mut values = headers.get_all(http::header::CONTENT_LENGTH).iter();
        match (values.next(), values.next()) {
            (Some(value), None) => value.to_str().ok()?.parse().ok(),
            (None | Some(_), _) => None,
        }
    }

    fn report(&mut self, event: HttpEvent) {
        if let Watch::Streaming(cell) = std::mem::replace(&mut self.watch, Watch::Done) {
            match event {
                HttpEvent::BodyEnded | HttpEvent::LengthDelivered => {
                    cell.tap.end(Direction::Response);
                }
                HttpEvent::UploadEnded
                | HttpEvent::UploadFailed
                | HttpEvent::Responded { .. }
                | HttpEvent::SendFailed(_)
                | HttpEvent::BodyFailed(_)
                | HttpEvent::Cancelled(_)
                | HttpEvent::Dropped => {}
            }
            cell.observe(event);
        }
    }

    fn fail(&mut self, event: HttpEvent) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        self.report(event);
        Poll::Ready(Some(Err(io::Error::other("response stream stopped"))))
    }
}

impl http_body::Body for ObservedResponse {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let this = self.get_mut();
        let Watch::Streaming(cell) = &this.watch else {
            return Poll::Ready(None);
        };
        if this.aborted.as_mut().poll(cx).is_ready() {
            return this.fail(HttpEvent::Cancelled(Canceller::Shutdown));
        }
        // Output may already be flowing: the client sees an interrupted stream.
        if this.operator.as_mut().poll(cx).is_ready() {
            return this.fail(HttpEvent::Cancelled(Canceller::Operator));
        }
        let cell = cell.clone();
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(bytes) = frame.data_ref() {
                    cell.tap.data(Direction::Response, bytes);
                    this.relayed = this
                        .relayed
                        .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                }
                // Trailers can follow data; the end is the stream's, not a length.
                if this.inner.is_end_stream() {
                    this.report(HttpEvent::BodyEnded);
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(failure))) => this.fail(HttpEvent::BodyFailed(failure)),
            Poll::Ready(None) => {
                this.report(HttpEvent::BodyEnded);
                Poll::Ready(None)
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        matches!(self.watch, Watch::Done)
    }

    fn size_hint(&self) -> SizeHint {
        if self.is_end_stream() {
            SizeHint::with_exact(0)
        } else {
            self.inner.size_hint()
        }
    }
}

impl Drop for ObservedResponse {
    fn drop(&mut self) {
        // Dropped before its end: the client or the server let go of it. A
        // server writing HTTP/1.1 lets go once the declared length is
        // written, which is the body's end by its framing; anything short of
        // that length is an interruption.
        let event = match self.declared {
            Some(declared) if declared == self.relayed => HttpEvent::LengthDelivered,
            Some(_) | None => HttpEvent::Dropped,
        };
        self.report(event);
    }
}

#[cfg(test)]
mod tests;
