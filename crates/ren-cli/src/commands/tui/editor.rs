//! Writing a prompt in the operator's own editor: `$VISUAL`, then
//! `$EDITOR`, then `vi`, on a temporary file holding the text, while the
//! terminal app hands over the terminal. What the editor saves comes back.

use ren_harness::host::HostEnv;
use std::io::Write as _;

/// The operator's editor, as the environment names it.
pub(super) struct Editor {
    program: String,
    arguments: Vec<String>,
}

/// Why the text was not edited.
#[derive(Debug, thiserror::Error)]
pub(super) enum EditError {
    #[error("cannot write the text to a temporary file")]
    File(#[source] std::io::Error),
    #[error("cannot start {0}; set VISUAL or EDITOR")]
    Start(String, #[source] std::io::Error),
    #[error("{0} exited with an error; the text is kept as it was")]
    Failed(String),
    #[error("cannot read the edited text back")]
    Read(#[source] std::io::Error),
}

impl Editor {
    /// `$VISUAL`, then `$EDITOR` (each may carry arguments, as in
    /// `code --wait`), else `vi`.
    pub fn of(env: &HostEnv) -> Self {
        let named = ["VISUAL", "EDITOR"]
            .into_iter()
            .filter_map(|name| env.get(name))
            .find(|value| !value.trim().is_empty())
            .unwrap_or("vi");
        let mut words = named.split_whitespace().map(str::to_owned);
        Self {
            program: words.next().unwrap_or_else(|| "vi".to_owned()),
            arguments: words.collect(),
        }
    }

    /// `text` edited: written to a private temporary file, opened in the
    /// editor until it exits, and read back. Blocks until then; the caller
    /// has handed it the terminal.
    pub fn edit(&self, text: &str) -> Result<String, EditError> {
        let mut file = tempfile::Builder::new()
            .prefix("ren-prompt-")
            .suffix(".md")
            .tempfile()
            .map_err(EditError::File)?;
        file.write_all(text.as_bytes()).map_err(EditError::File)?;
        file.flush().map_err(EditError::File)?;
        let status = std::process::Command::new(&self.program)
            .args(&self.arguments)
            .arg(file.path())
            .status()
            .map_err(|error| EditError::Start(self.program.clone(), error))?;
        if !status.success() {
            return Err(EditError::Failed(self.program.clone()));
        }
        std::fs::read_to_string(file.path()).map_err(EditError::Read)
    }
}

#[cfg(test)]
mod tests {
    use super::Editor;
    use ren_harness::host::HostEnv;

    #[test]
    fn the_editor_is_visual_then_editor_then_vi_with_its_arguments() {
        let editor = Editor::of(&HostEnv::from_pairs([
            ("EDITOR", "nano"),
            ("VISUAL", "code --wait"),
        ]));
        assert_eq!(editor.program, "code");
        assert_eq!(editor.arguments, ["--wait"]);
        assert_eq!(
            Editor::of(&HostEnv::from_pairs([("EDITOR", "nano")])).program,
            "nano"
        );
        assert_eq!(
            Editor::of(&HostEnv::from_pairs([("VISUAL", " ")])).program,
            "vi"
        );
    }

    #[test]
    fn what_the_editor_saves_comes_back() {
        // `true` leaves the file as written: the text comes back unchanged.
        let editor = Editor {
            program: "true".to_owned(),
            arguments: Vec::new(),
        };
        assert_eq!(
            editor.edit("fix it\nand test it").unwrap(),
            "fix it\nand test it"
        );
        let failing = Editor {
            program: "false".to_owned(),
            arguments: Vec::new(),
        };
        assert!(failing.edit("x").is_err());
    }
}
