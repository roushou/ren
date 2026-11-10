//! The coding-agent harnesses Ren serves (Claude Code, Codex, Pi, and
//! opencode) and what Ren knows about each: the attribution hints their
//! requests carry, how to launch one through the gateway for a session, Pi's
//! bundled extension, the gateway URL a harness is pointed at, and the host
//! environment launching reads. See docs/architecture.md.
//!
//! The primary type is [`Harness`]. Harnesses own their sign-in, token
//! refresh, and retries; nothing here handles credentials.

pub mod attribution;
pub mod drive;
pub mod endpoint;
pub mod host;
pub mod launch;
pub mod pi;
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
