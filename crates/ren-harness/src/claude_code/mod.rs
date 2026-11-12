//! Claude Code: what Ren knows of it, in one place. Its print mode with
//! streaming JSON and how Ren drives it and lists its models
//! (`stream_json`); how it is launched through the gateway (`launch`); the
//! attribution its requests carry (`headers`); and its tools (`tools`).
//! The rest of Ren reads Claude Code through the shared types these fill.

mod headers;
mod launch;
mod stream_json;
mod tools;

pub use stream_json::{ClaudeModels, ClaudeStreamJson};
pub use tools::ClaudeCodeTool;
