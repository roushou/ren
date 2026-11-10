//! How a fake agent works through a turn, as pure data shared by the traffic
//! that sends the turn and the fake provider that answers it: which tool it
//! calls at each step, in its client's own tool names and input fields, with
//! what call ID, and what the tool returned. Both sides derive the same
//! steps, so a follow-up request carries the results of exactly the tool
//! calls the provider made, as a real client's does.

use serde_json::{Value, json};

/// A client's built-in tools, as Ren's harness reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toolset {
    /// `Read`, `Edit`, `Write`, `Bash` (also Claude Code on DeepSeek).
    ClaudeCode,
    /// `read_file`, `apply_patch`, `shell`.
    Codex,
    /// `read`, `edit`, `write`, `bash`.
    Pi,
    /// `read`, `edit`, `write`, `shell`.
    OpenCode,
}

/// What a step does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Read,
    Edit,
    Write,
    Run,
}

/// One tool call of a turn, and what it returned.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// What the model says before calling the tool.
    pub lead: &'static str,
    pub name: &'static str,
    pub input: Value,
    pub id: String,
    pub output: String,
    pub failed: bool,
}

/// The tool work of one turn: its client's tools, its prompt, and a salt
/// that keeps call IDs apart between sessions asking the same prompt.
#[derive(Debug, Clone)]
pub struct ToolRun {
    toolset: Toolset,
    seed: u64,
}

/// Invented files and commands of the invented "acme" project.
const FILES: [&str; 5] = [
    "src/orders/list.rs",
    "src/upload/client.rs",
    "crates/acme-billing/src/invoice.rs",
    "config/acme.toml",
    "migrations/0042_orders_created_at.sql",
];
const COMMANDS: [&str; 4] = [
    "cargo test -p acme-orders",
    "rg -n \"listen_addr\" --type rust",
    "cargo build --release -p acme-web",
    "cargo test -p acme-billing",
];
/// Each turn follows one of these, a step at a time.
const PLANS: [[Act; 3]; 3] = [
    [Act::Read, Act::Edit, Act::Run],
    [Act::Run, Act::Read, Act::Write],
    [Act::Read, Act::Read, Act::Edit],
];

impl Toolset {
    /// The toolset a request declares, from the names in its `tools`; `None`
    /// when it declares none this knows.
    pub fn declared(body: &Value) -> Option<Self> {
        let names: Vec<&str> = body["tools"]
            .as_array()?
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        let has = |name: &str| names.contains(&name);
        if has("Bash") {
            Some(Self::ClaudeCode)
        } else if has("apply_patch") {
            Some(Self::Codex)
        } else if has("bash") {
            Some(Self::Pi)
        } else if has("shell") {
            Some(Self::OpenCode)
        } else {
            None
        }
    }

    fn name(self, act: Act) -> &'static str {
        match (self, act) {
            (Self::ClaudeCode, Act::Read) => "Read",
            (Self::ClaudeCode, Act::Edit) => "Edit",
            (Self::ClaudeCode, Act::Write) => "Write",
            (Self::ClaudeCode, Act::Run) => "Bash",
            (Self::Codex, Act::Read) => "read_file",
            (Self::Codex, Act::Edit | Act::Write) => "apply_patch",
            (Self::Codex | Self::OpenCode, Act::Run) => "shell",
            (Self::Pi | Self::OpenCode, Act::Read) => "read",
            (Self::Pi | Self::OpenCode, Act::Edit) => "edit",
            (Self::Pi | Self::OpenCode, Act::Write) => "write",
            (Self::Pi, Act::Run) => "bash",
        }
    }

    /// Every tool this client declares, by name.
    fn names(self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = [Act::Read, Act::Edit, Act::Write, Act::Run]
            .into_iter()
            .map(|act| self.name(act))
            .collect();
        names.dedup();
        names
    }

    /// The `tools` a Messages request declares.
    pub fn messages_tools(self) -> Value {
        Value::Array(
            self.names()
                .into_iter()
                .map(|name| {
                    json!({ "name": name, "description": format!("The {name} tool."),
                                    "input_schema": { "type": "object" } })
                })
                .collect(),
        )
    }

    /// The `tools` a Responses request declares.
    pub fn responses_tools(self) -> Value {
        Value::Array(
            self.names()
                .into_iter()
                .map(|name| {
                    json!({ "type": "function", "name": name,
                                    "description": format!("The {name} tool."),
                                    "parameters": { "type": "object" } })
                })
                .collect(),
        )
    }

    /// The input `act` takes on `file` or `command`, in this client's fields.
    fn input(self, act: Act, file: &str, command: &str) -> Value {
        let path = match self {
            Self::ClaudeCode => "file_path",
            Self::Codex | Self::Pi | Self::OpenCode => "path",
        };
        match (self, act) {
            (Self::Codex, Act::Edit) => json!({ "input": format!(
                "*** Begin Patch\n*** Update File: {file}\n@@\n-    sort_by_key(|order| order.created_at);\n+    // Sorted by the query, newest first.\n*** End Patch"
            ) }),
            (Self::Codex, Act::Write) => json!({ "input": format!(
                "*** Begin Patch\n*** Add File: {file}\n+// Added for the acme change.\n*** End Patch"
            ) }),
            (Self::Codex, Act::Run) => json!({ "command": ["bash", "-lc", command] }),
            (Self::ClaudeCode, Act::Run) => {
                json!({ "command": command, "description": "Run in the project root" })
            }
            (Self::Pi | Self::OpenCode, Act::Run) => json!({ "command": command }),
            (Self::ClaudeCode, Act::Edit) => json!({ path: file,
                "old_string": "sort_by_key(|order| order.created_at)",
                "new_string": "// Sorted by the query, newest first." }),
            (Self::Pi, Act::Edit) => json!({ path: file,
                "oldText": "sort_by_key(|order| order.created_at)",
                "newText": "// Sorted by the query, newest first." }),
            (Self::OpenCode, Act::Edit) => json!({ path: file,
                "oldString": "sort_by_key(|order| order.created_at)",
                "newString": "// Sorted by the query, newest first." }),
            (Self::ClaudeCode | Self::Pi | Self::OpenCode, Act::Write) => {
                json!({ path: file, "content": "// Added for the acme change.\n" })
            }
            (Self::ClaudeCode | Self::Codex | Self::Pi | Self::OpenCode, Act::Read) => {
                json!({ path: file })
            }
        }
    }

    /// A tool's output as this client reports it: Codex puts a command's
    /// exit code first, since its results carry no error flag.
    fn output(self, act: Act, failed: bool, text: String) -> String {
        match (self, act) {
            (Self::Codex, Act::Run) => format!(
                "Exit code: {}\nWall time: 2.1 seconds\nOutput:\n{text}",
                if failed { 101 } else { 0 }
            ),
            (Self::Codex, Act::Edit | Act::Write) => {
                "Success. Updated the following files:\nM src/orders/list.rs".to_owned()
            }
            (
                Self::ClaudeCode | Self::Pi | Self::OpenCode,
                Act::Read | Act::Edit | Act::Write | Act::Run,
            )
            | (Self::Codex, Act::Read) => text,
        }
    }

    /// A call ID in the style of this client's wire format.
    fn id(self, hash: u64) -> String {
        match self {
            Self::ClaudeCode | Self::Pi => format!("toolu_01{hash:016x}"),
            Self::Codex | Self::OpenCode => format!("call_{hash:016x}"),
        }
    }
}

impl ToolRun {
    /// The work of a turn asking `prompt`, salted with something its session
    /// sends on every request (Claude Code's metadata, Codex's cache key).
    pub fn new(toolset: Toolset, prompt: &str, salt: &str) -> Self {
        Self {
            toolset,
            seed: Self::hash(&[salt.as_bytes(), prompt.as_bytes()]),
        }
    }

    /// The turn's `index`th tool call (from 0).
    pub fn step(&self, index: usize) -> Step {
        let at = u64::try_from(index).unwrap_or(u64::MAX);
        let mix = Self::hash(&[&self.seed.to_le_bytes(), &at.to_le_bytes()]);
        let plan = PLANS[Self::pick(self.seed, PLANS.len())];
        let act = plan[index % plan.len()];
        let file = FILES[Self::pick(self.seed.wrapping_add(at), FILES.len())];
        let command = COMMANDS[Self::pick(self.seed.wrapping_add(at), COMMANDS.len())];
        // About one command in four fails, as test runs do.
        let failed = act == Act::Run && mix.is_multiple_of(4);
        let (lead, output) = match act {
            Act::Read => (
                "Let me read it before changing anything.",
                format!(
                    "     1\t// {file}\n     2\tuse crate::orders::Order;\n     3\t\n     4\tpub fn list(orders: &mut [Order]) {{\n     5\t    orders.sort_by_key(|order| order.created_at);\n     6\t}}\n"
                ),
            ),
            Act::Edit => (
                "I'll make the change.",
                format!("The file {file} has been updated."),
            ),
            Act::Write => (
                "I'll add the new file.",
                format!("File created successfully at: {file}"),
            ),
            Act::Run if failed => (
                "Let me run it to check.",
                "running 14 tests\ntest orders::list::newest_first ... FAILED\n\nfailures:\n    orders::list::newest_first\n\ntest result: FAILED. 13 passed; 1 failed\nerror: test failed, to rerun pass `-p acme-orders --lib`".to_owned(),
            ),
            Act::Run => (
                "Let me run it to check.",
                "running 14 tests\n..............\ntest result: ok. 14 passed; 0 failed; finished in 0.42s".to_owned(),
            ),
        };
        Step {
            lead,
            name: self.toolset.name(act),
            input: self.toolset.input(act, file, command),
            id: self.toolset.id(mix),
            output: self.toolset.output(act, failed, output),
            failed,
        }
    }

    /// FNV-1a, so both sides agree with no shared state.
    fn hash(parts: &[&[u8]]) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for part in parts {
            for byte in *part {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
            hash ^= 0xff;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash
    }

    fn pick(seed: u64, len: usize) -> usize {
        usize::try_from(seed % u64::try_from(len).unwrap_or(1)).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{ToolRun, Toolset};
    use serde_json::json;

    #[test]
    fn a_request_declares_its_toolset_by_its_tool_names() {
        for toolset in [
            Toolset::ClaudeCode,
            Toolset::Codex,
            Toolset::Pi,
            Toolset::OpenCode,
        ] {
            assert_eq!(
                Toolset::declared(&json!({ "tools": toolset.messages_tools() })),
                Some(toolset)
            );
            assert_eq!(
                Toolset::declared(&json!({ "tools": toolset.responses_tools() })),
                Some(toolset)
            );
        }
        assert_eq!(Toolset::declared(&json!({})), None);
        assert_eq!(
            Toolset::declared(&json!({ "tools": [{ "name": "web" }] })),
            None
        );
    }

    #[test]
    fn steps_agree_on_both_sides_and_differ_between_sessions() {
        let run = ToolRun::new(Toolset::ClaudeCode, "Fix the list", "session-a");
        let again = ToolRun::new(Toolset::ClaudeCode, "Fix the list", "session-a");
        let other = ToolRun::new(Toolset::ClaudeCode, "Fix the list", "session-b");
        assert_eq!(run.step(1), again.step(1));
        assert_ne!(run.step(1).id, other.step(1).id);
        assert_ne!(run.step(0).id, run.step(1).id);
    }

    #[test]
    fn every_client_uses_its_own_tool_names_and_fields() {
        let names = |toolset: Toolset| {
            let run = ToolRun::new(toolset, "Fix the list", "s");
            (0..3).map(|index| run.step(index).name).collect::<Vec<_>>()
        };
        for (toolset, known) in [
            (Toolset::ClaudeCode, &["Read", "Edit", "Write", "Bash"][..]),
            (Toolset::Codex, &["read_file", "apply_patch", "shell"][..]),
            (Toolset::Pi, &["read", "edit", "write", "bash"][..]),
            (Toolset::OpenCode, &["read", "edit", "write", "shell"][..]),
        ] {
            for name in names(toolset) {
                assert!(known.contains(&name), "{toolset:?} {name}");
            }
        }
        // Claude Code names files `file_path`; the others `path`.
        let claude = ToolRun::new(Toolset::ClaudeCode, "p", "s");
        let pi = ToolRun::new(Toolset::Pi, "p", "s");
        let read = (0..3)
            .map(|index| claude.step(index))
            .find(|step| step.name == "Read")
            .unwrap();
        assert!(read.input["file_path"].is_string());
        let read = (0..3)
            .map(|index| pi.step(index))
            .find(|step| step.name == "read")
            .unwrap();
        assert!(read.input["path"].is_string());
        // Codex reports a command's exit code on its first line.
        let codex = ToolRun::new(Toolset::Codex, "p", "s");
        let run = (0..3)
            .map(|index| codex.step(index))
            .find(|step| step.name == "shell")
            .unwrap();
        assert!(run.output.starts_with("Exit code: "), "{}", run.output);
    }
}
