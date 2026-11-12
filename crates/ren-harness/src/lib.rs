//! The coding-agent harnesses Ren serves (Claude Code, Codex, Pi, and
//! opencode) and what Ren knows about each. Each client is one module
//! (`claude_code`, `codex`, `pi`, `opencode`) holding its own ways: its
//! structured mode, launch plan, attribution headers, and tools. Each fills
//! the shared types the rest of Ren reads (`drive`, `models`, `settings`,
//! `launch`, `attribution`, `tools`), and [`Harness`] dispatches to them by
//! `match`. See docs/architecture.md.
//!
//! The primary type is [`Harness`]. Harnesses own their sign-in, token
//! refresh, and retries; nothing here handles credentials.

pub mod attribution;
pub mod claude_code;
pub mod codex;
pub mod drive;
pub mod endpoint;
pub mod host;
pub mod launch;
pub mod lines;
pub mod models;
pub mod opencode;
pub mod pi;
pub mod settings;
pub mod tools;

/// A coding-agent harness Ren serves: the clients of AGENTS.md section 1. The
/// set is closed; adding one is a new variant, and every match must handle it.
/// Its labels name it in queries (`?client=`).
#[derive(strum::Display, strum::EnumString, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    serde_with::SerializeDisplay,
    serde_with::DeserializeFromStr,
)]
pub enum Harness {
    ClaudeCode,
    Codex,
    Pi,
    #[strum(to_string = "opencode")]
    OpenCode,
}
