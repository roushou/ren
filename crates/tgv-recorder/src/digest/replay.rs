//! A finished call's stored bodies, replayed through the same readers that
//! read them while they streamed, so a digest can be derived again: for calls
//! recorded before digests existed, or by an older [`CallDigest::VERSION`].

use super::{CallDigest, DigestBudget};
use crate::capture::BodyEnding;
use crate::id::CallId;
use crate::secrets::SecretScanner;
use std::ops::Range;
use tgv_harness::{Harness, tools::InterpretedCall};
use tgv_protocol::{
    CallInspector,
    activity::{ActivityLimits, Reading, RequestReader, ResponseActivity},
    encoding::{ResponseEncoding, ResponseFormat},
    inspect::pipeline::{BodyEnd, InspectionLimits, Protocol},
    route::Endpoint,
};

/// A finished call, as the sink stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredExchange {
    pub call: CallId,
    /// `None` for an endpoint this build does not know.
    pub endpoint: Option<Endpoint>,
    /// The harness whose hints attributed the call.
    pub harness: Option<Harness>,
    pub request: Option<StoredBody>,
    /// The final attempt's response, when it had one.
    pub response: Option<StoredResponse>,
}

/// A stored body: its captured prefix and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBody {
    pub bytes: Vec<u8>,
    /// Captured bytes are missing after `bytes`, or within the body.
    pub complete: Captured,
    pub ending: BodyEnding,
    /// WebSocket message boundaries within `bytes`.
    pub messages: Option<Vec<Range<usize>>>,
}

/// Whether the stored prefix is all of what was observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Captured {
    Whole,
    /// Bytes are missing: a gap, a limit, or a prefix read.
    Partial,
}

/// A stored response and how it was declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredResponse {
    pub body: StoredBody,
    pub format: ResponseFormat,
    pub encoding: ResponseEncoding,
}

impl StoredExchange {
    /// The call's digest, derived from its stored bodies.
    pub fn digest(self, scanner: &SecretScanner) -> CallDigest {
        let harness = self.harness;
        CallDigest::seal(
            InterpretedCall::new(harness, self.reading()),
            scanner,
            DigestBudget::STORED,
        )
    }

    fn reading(self) -> Reading {
        let Some(protocol) = self.endpoint.and_then(Protocol::for_endpoint) else {
            return Reading::unsupported();
        };
        let request = self.request.map_or_else(Default::default, |body| {
            let mut reader = RequestReader::new(protocol, ActivityLimits::default());
            if body.messages.is_some() {
                reader.message();
            }
            reader.feed(&body.bytes);
            if body.complete == Captured::Partial {
                reader.gap();
            }
            reader.finish(body.ending.end())
        });
        let response = self
            .response
            .map_or_else(ResponseActivity::default, |response| {
                let mut inspector = CallInspector::new(
                    protocol,
                    response.format,
                    response.encoding,
                    InspectionLimits::default(),
                );
                let body = response.body;
                match &body.messages {
                    Some(messages) => {
                        for range in messages {
                            inspector.message(u64::try_from(range.len()).unwrap_or(u64::MAX));
                            inspector.feed(body.bytes.get(range.clone()).unwrap_or_default());
                        }
                    }
                    None => inspector.feed(&body.bytes),
                }
                if body.complete == Captured::Partial {
                    inspector.gap();
                }
                inspector.finish(body.ending.end()).activity
            });
        Reading { request, response }
    }
}

impl BodyEnding {
    /// How the body ended, for a reader.
    pub fn end(self) -> BodyEnd {
        match self {
            Self::Ended => BodyEnd::Complete,
            Self::Open | Self::Unfinished => BodyEnd::Unfinished,
        }
    }
}
