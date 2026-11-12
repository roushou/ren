//! What the message box offers as the operator types: the run's client's
//! commands after `/` at the start of the prompt (or Codex's skills after
//! `$`), and its folder's files after `@`. The same rules as the web
//! composer's (`ui/src/lib/mention.ts`).

use super::message::MessageBox;
use crate::model::{FileSearch, Model, Read};
use ren_harness::commands::{ClientCommand, CommandKind, Sigil};
use ren_recorder::id::RunId;
use ren_supervisor::{FileMatch, PathKind};

/// The word at the cursor that asks for something to be offered, in
/// characters of the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mention {
    pub asks: Asks,
    /// What was typed after the sigil, up to the cursor.
    pub query: String,
    /// Where the word starts and ends: what choosing replaces.
    pub start: usize,
    pub end: usize,
}

/// What a mention asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asks {
    /// `/`: every command the client offers.
    Commands,
    /// `$`: the commands named with `$` (Codex's skills).
    Skills,
    /// `@`: a path of the run's folder.
    Files,
}

impl Mention {
    /// The most commands offered at once.
    const COMMANDS: usize = 50;

    /// What the word at `cursor` of `text` asks for, if anything.
    pub fn at(text: &[char], cursor: usize) -> Option<Self> {
        if cursor > text.len() {
            return None;
        }
        let start = text
            .get(..cursor)?
            .iter()
            .rposition(|ch| ch.is_whitespace())
            .map_or(0, |space| space + 1);
        let end = text
            .get(cursor..)?
            .iter()
            .position(|ch| ch.is_whitespace())
            .map_or(text.len(), |space| cursor + space);
        let (&sigil, typed) = text.get(start..cursor)?.split_first()?;
        let query: String = typed.iter().collect();
        let asks = match sigil {
            '@' => Asks::Files,
            // A command opens the prompt; a path elsewhere is just a path.
            '/' if text.get(..start)?.iter().all(|ch| ch.is_whitespace())
                && !query.contains('/') =>
            {
                Asks::Commands
            }
            '$' if query
                .chars()
                .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | ':' | '.' | '-')) =>
            {
                Asks::Skills
            }
            _ => return None,
        };
        Some(Self {
            asks,
            query,
            start,
            end,
        })
    }

    /// The commands to offer: names starting with the query, then names
    /// holding it, then descriptions mentioning it; by name within each.
    pub fn commands<'a>(&self, commands: &'a [ClientCommand]) -> Vec<&'a ClientCommand> {
        let query = self.query.to_lowercase();
        let mut ranked: Vec<(u8, &ClientCommand)> = commands
            .iter()
            .filter(|command| match self.asks {
                Asks::Skills => command.sigil == Sigil::Dollar,
                Asks::Commands | Asks::Files => true,
            })
            .filter_map(|command| {
                let name = command.name.to_lowercase();
                let rank = if name.starts_with(&query) {
                    0
                } else if name.contains(&query) {
                    1
                } else if !query.is_empty() && command.description.to_lowercase().contains(&query) {
                    2
                } else {
                    return None;
                };
                Some((rank, command))
            })
            .collect();
        ranked.sort_by(|(a, left), (b, right)| a.cmp(b).then_with(|| left.name.cmp(&right.name)));
        ranked
            .into_iter()
            .take(Self::COMMANDS)
            .map(|(_, command)| command)
            .collect()
    }

    /// What choosing `command` writes in place of the mention.
    pub fn command(command: &ClientCommand) -> String {
        format!("{} ", command.invocation())
    }

    /// What choosing `file` writes: `@path`, a folder left open to go into.
    pub fn file(file: &FileMatch) -> String {
        match file.kind {
            PathKind::Directory => format!("@{}/", file.path),
            PathKind::File => format!("@{} ", file.path),
        }
    }
}

/// One offer as the menu shows it, and what choosing it writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Offer {
    pub label: String,
    /// The label's characters (by index) the typed text matched.
    pub hits: Vec<usize>,
    /// What follows the label, quieter: a command's arguments.
    pub hint: Option<String>,
    /// What it is, when not a plain command: skill, template, folder.
    pub tag: Option<&'static str>,
    pub detail: String,
    pub written: String,
}

/// The menu's state while writing: which offer is lit, a mention closed
/// with Esc, and the newest read asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Offering {
    pub active: usize,
    /// Where the mention closed with Esc starts; it stays closed until the
    /// text changes.
    pub dismissed: Option<usize>,
    /// The mention the lit offer belongs to, to start a new one at the top.
    shown: Option<(usize, String)>,
    /// The newest read asked for, and at which mention, not to ask twice.
    asked: Option<(Read, usize)>,
}

impl Offering {
    /// The mention at the cursor, unless it was closed.
    pub fn open(&self, message: &MessageBox) -> Option<Mention> {
        message
            .mention()
            .filter(|mention| Some(mention.start) != self.dismissed)
    }

    /// What `mention` offers for `run`, from what was read.
    pub fn offers(model: &Model, run: RunId, mention: &Mention) -> Vec<Offer> {
        match mention.asks {
            Asks::Commands | Asks::Skills => match model.commands.get(&run) {
                Some(Ok(commands)) => mention
                    .commands(commands)
                    .into_iter()
                    .map(|command| {
                        let label = command.invocation();
                        let query = mention.query.to_lowercase();
                        let hits = match command.name.to_lowercase().find(&query) {
                            Some(at) if !query.is_empty() => {
                                let first =
                                    command.name.get(..at).map_or(0, |b| b.chars().count()) + 1;
                                (first..first + query.chars().count()).collect()
                            }
                            Some(_) | None => Vec::new(),
                        };
                        Offer {
                            label,
                            hits,
                            hint: command.hint.clone(),
                            tag: match command.kind {
                                CommandKind::Command => None,
                                CommandKind::Skill => Some("skill"),
                                CommandKind::Template => Some("template"),
                            },
                            detail: command.description.clone(),
                            written: Mention::command(command),
                        }
                    })
                    .collect(),
                Some(Err(_)) | None => Vec::new(),
            },
            Asks::Files => match &model.files {
                Some(FileSearch {
                    run: searched,
                    files: Ok(files),
                    ..
                }) if *searched == run => files
                    .iter()
                    .map(|file| Offer {
                        label: file.path.clone(),
                        hits: file.positions.clone(),
                        hint: None,
                        tag: match file.kind {
                            PathKind::Directory => Some("folder"),
                            PathKind::File => None,
                        },
                        detail: String::new(),
                        written: Mention::file(file),
                    })
                    .collect(),
                Some(_) | None => Vec::new(),
            },
        }
    }

    /// What the menu says instead of offers: still reading, nothing found,
    /// or why not.
    pub fn note(model: &Model, run: RunId, mention: &Mention) -> &'static str {
        match mention.asks {
            Asks::Commands | Asks::Skills => match model.commands.get(&run) {
                None => "reading the commands…",
                Some(Ok(commands)) if commands.is_empty() => {
                    "the client lists its commands once the conversation opens"
                }
                Some(Ok(_)) => "no command matches",
                Some(Err(_)) => "the commands could not be read",
            },
            Asks::Files => match &model.files {
                Some(FileSearch { files: Err(_), .. }) => "the folder could not be searched",
                Some(FileSearch { query, .. }) if *query == mention.query => "no file matches",
                Some(_) | None => "searching…",
            },
        }
    }

    /// The read the mention needs, once per mention for commands (again
    /// while the client has listed none) and once per query for files.
    pub fn read(&mut self, model: &Model, run: RunId, mention: &Mention) -> Option<Read> {
        let read = match mention.asks {
            Asks::Commands | Asks::Skills => {
                let listed =
                    matches!(model.commands.get(&run), Some(Ok(commands)) if !commands.is_empty());
                if listed {
                    return None;
                }
                Read::Commands(run)
            }
            Asks::Files => Read::Files {
                run,
                query: mention.query.clone(),
            },
        };
        let asked = (read.clone(), mention.start);
        if self.asked.as_ref() == Some(&asked) {
            return None;
        }
        self.asked = Some(asked);
        Some(read)
    }

    /// A new mention starts with its first offer lit.
    pub fn follow(&mut self, mention: Option<&Mention>) {
        let shown = mention.map(|mention| (mention.start, mention.query.clone()));
        if shown != self.shown {
            self.shown = shown;
            self.active = 0;
        }
    }

    /// The lit offer moved by one, wrapping around `count`.
    pub fn step(&mut self, forward: bool, count: usize) {
        if count > 0 {
            self.active = if forward {
                (self.active + 1) % count
            } else {
                (self.active + count - 1) % count
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, cursor: usize) -> Option<Mention> {
        Mention::at(&text.chars().collect::<Vec<_>>(), cursor)
    }

    fn command(name: &str, sigil: Sigil, description: &str) -> ClientCommand {
        ClientCommand {
            name: name.into(),
            description: description.into(),
            hint: None,
            kind: CommandKind::Command,
            sigil,
        }
    }

    #[test]
    fn a_slash_asks_only_at_the_start_and_an_at_sign_anywhere() {
        let slash = at("/rev", 4).unwrap();
        assert_eq!(
            (slash.asks, slash.query.as_str(), slash.start, slash.end),
            (Asks::Commands, "rev", 0, 4)
        );
        assert!(at("look at /etc/hosts", 18).is_none());
        assert!(at("/src/lib", 8).is_none());
        let file = at("fix @src/up and more", 11).unwrap();
        assert_eq!(
            (file.asks, file.query.as_str(), file.start, file.end),
            (Asks::Files, "src/up", 4, 11)
        );
        let inside = at("see @runs.ts", 8).unwrap();
        assert_eq!((inside.query.as_str(), inside.end), ("run", 12));
        assert_eq!(at("use $rel", 8).unwrap().asks, Asks::Skills);
        assert!(at("costs $5,000", 12).is_none());
        assert!(at("plain words", 5).is_none());
    }

    #[test]
    fn commands_rank_by_name_then_description() {
        let listed = [
            command("code-review", Sigil::Slash, "Review the diff"),
            command("review", Sigil::Slash, ""),
            command("release-notes", Sigil::Dollar, "Write notes"),
            command("compact", Sigil::Slash, "Summarize to review later"),
        ];
        let names = |mention: Mention| -> Vec<String> {
            mention
                .commands(&listed)
                .iter()
                .map(|c| c.name.clone())
                .collect()
        };
        assert_eq!(
            names(at("/rev", 4).unwrap()),
            ["review", "code-review", "compact"]
        );
        assert_eq!(names(at("$", 1).unwrap()), ["release-notes"]);
    }

    #[test]
    fn choosing_writes_the_invocation_or_the_path() {
        assert_eq!(
            Mention::command(&command("review", Sigil::Slash, "")),
            "/review "
        );
        let folder = FileMatch {
            path: "src/lib".into(),
            kind: PathKind::Directory,
            positions: Vec::new(),
        };
        assert_eq!(Mention::file(&folder), "@src/lib/");
    }
}
