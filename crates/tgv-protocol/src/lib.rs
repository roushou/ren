//! Sans-IO understanding of native provider protocols.
//!
//! Everything here is pure computation over bytes the caller already has:
//! SSE framing, Anthropic Messages and OpenAI Responses wire vocabularies,
//! WebSocket message classification, incremental decompression, and per-call
//! inspection, plus the vocabularies that name providers, endpoints, and
//! directions, and the native identifiers the wire carries. Nothing here
//! performs I/O, reads time, or depends on the async runtime, HTTP framework,
//! database, or CLI parser; the manifest lists the only dependencies allowed.
//!
//! The primary type is [`CallInspector`], which inspects one call's captured
//! bytes. Observation never changes traffic: callers forward bytes unchanged
//! and feed copies here. Unknown events, kinds, and statuses are kept as text
//! and counted, never dropped. Malformed input becomes a [`Limitation`], never
//! a fact. See docs/architecture.md.
//!
//! [`Limitation`]: inspect::Limitation

pub mod activity;
pub mod direction;
pub mod encoding;
pub mod inspect;
pub mod json;
pub mod label;
pub mod messages;
pub mod native;
pub mod responses;
pub mod route;
pub mod sse;
pub mod text;
pub mod websocket;

pub use inspect::pipeline::CallInspector;

#[cfg(test)]
mod tests;
