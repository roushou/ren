//! Claude Code: what Ren knows of it, in one place. Its print mode with
//! streaming JSON and how Ren drives it and lists its models
//! (`stream_json`); how it is launched through the gateway (`launch`); the
//! attribution its requests carry (`headers`); its tools (`tools`), and the
//! questions its `AskUserQuestion` tool asks (`question`); and its
//! own sign-in commands (`auth`).
//! The rest of Ren reads Claude Code through the shared types these fill.

mod auth;
mod headers;
mod launch;
mod listing;
mod profile;
mod question;
mod stream_json;
mod tools;

pub use auth::ClaudeAuth;
pub use listing::{ClaudeCommands, ClaudeModels};
pub use question::ClaudeQuestionKind;
pub use stream_json::{ClaudeMessageType, ClaudeStreamJson, ClaudeSystemSubtype};
pub use tools::ClaudeCodeTool;
