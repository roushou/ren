//! Slash commands, the console's way to act: typed in the message box,
//! completed with tab. Parsing and completion are pure.

use ren_harness::Harness;
use ren_supervisor::{Project, ProjectGit};

/// A command typed in the message box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slash {
    /// `/new <project> [claude|codex] [first prompt…]`
    New {
        project: String,
        client: Option<Harness>,
        prompt: Option<String>,
    },
    /// `/resume [search]`
    Resume { search: String },
    /// `/stop`: end the run's process.
    Stop,
    /// `/wall`: every session's traffic.
    Wall,
    /// `/help`
    Help,
    /// `/quit`
    Quit,
    /// A command the console does not know, as typed.
    Unknown(String),
}

/// One completion offered under the message box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    /// The message box's text once it is chosen.
    pub replacement: String,
}

/// Why `/new` could not name a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolved {
    Missing,
    NoMatch(String),
    /// Several match; their names.
    Ambiguous(Vec<String>),
}

/// The commands, with what each does, as completion and help list them.
const COMMANDS: [(&str, &str); 6] = [
    ("new", "start a run: /new <project> [claude|codex] <prompt>"),
    ("resume", "every run, what needs you first"),
    ("stop", "end this run's process"),
    ("wall", "every session's traffic, one pane each"),
    ("help", "keys and commands"),
    ("quit", "leave (runs keep going)"),
];

impl Slash {
    /// The command `text` holds, if it starts with `/`.
    pub fn parse(text: &str) -> Option<Self> {
        let rest = text.trim().strip_prefix('/')?;
        let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let args = args.trim();
        Some(match name {
            "new" => {
                let mut words = args.splitn(2, char::is_whitespace);
                let project = words.next().unwrap_or_default().to_owned();
                let rest = words.next().unwrap_or_default().trim();
                let (client, prompt) = match rest.split_once(char::is_whitespace) {
                    Some((word, prompt)) => match Self::client(word) {
                        Some(client) => (Some(client), prompt.trim()),
                        None => (None, rest),
                    },
                    None => match Self::client(rest) {
                        Some(client) => (Some(client), ""),
                        None => (None, rest),
                    },
                };
                Self::New {
                    project,
                    client,
                    prompt: (!prompt.is_empty()).then(|| prompt.to_owned()),
                }
            }
            "resume" => Self::Resume {
                search: args.to_owned(),
            },
            "stop" => Self::Stop,
            "wall" => Self::Wall,
            "help" => Self::Help,
            "quit" | "exit" => Self::Quit,
            other => Self::Unknown(other.to_owned()),
        })
    }

    fn client(word: &str) -> Option<Harness> {
        match word {
            "claude" => Some(Harness::ClaudeCode),
            "codex" => Some(Harness::Codex),
            _ => None,
        }
    }

    /// What completes `text`, the message box's whole text.
    pub fn complete(text: &str, projects: &[Project]) -> Vec<Suggestion> {
        let Some(rest) = text.strip_prefix('/') else {
            return Vec::new();
        };
        let Some((name, args)) = rest.split_once(' ') else {
            return COMMANDS
                .iter()
                .filter(|(command, _)| command.starts_with(rest) && *command != rest)
                .map(|(command, detail)| Suggestion {
                    label: format!("/{command}"),
                    detail: (*detail).to_owned(),
                    replacement: format!("/{command} "),
                })
                .collect();
        };
        if name != "new" {
            return Vec::new();
        }
        match args.split_once(' ') {
            // Naming the project.
            None => Self::matching(projects, args)
                .into_iter()
                .filter(|project| project.name != args)
                .map(|project| Suggestion {
                    label: project.name.clone(),
                    detail: format!("{}  {}", project.path, Self::branch(&project.git)),
                    replacement: format!("/new {} ", project.name),
                })
                .collect(),
            // Naming the client, once the project is named.
            Some((project, client)) if !client.contains(' ') => {
                [("claude", "Claude Code"), ("codex", "Codex")]
                    .into_iter()
                    .filter(|(word, _)| word.starts_with(client) && *word != client)
                    .map(|(word, name)| Suggestion {
                        label: word.to_owned(),
                        detail: name.to_owned(),
                        replacement: format!("/new {project} {word} "),
                    })
                    .collect()
            }
            Some(_) => Vec::new(),
        }
    }

    /// Projects whose name or path holds `query`, ignoring case.
    fn matching<'a>(projects: &'a [Project], query: &str) -> Vec<&'a Project> {
        let query = query.to_lowercase();
        projects
            .iter()
            .filter(|project| {
                project.name.to_lowercase().contains(&query)
                    || project.path.to_lowercase().contains(&query)
            })
            .collect()
    }

    /// The project `query` names: by its exact name, or the only one it
    /// matches.
    pub fn resolve<'a>(query: &str, projects: &'a [Project]) -> Result<&'a Project, Unresolved> {
        if query.is_empty() {
            return Err(Unresolved::Missing);
        }
        if let Some(exact) = projects
            .iter()
            .find(|project| project.name == query || project.path == query)
        {
            return Ok(exact);
        }
        match Self::matching(projects, query).as_slice() {
            [] => Err(Unresolved::NoMatch(query.to_owned())),
            [only] => Ok(only),
            several => Err(Unresolved::Ambiguous(
                several.iter().map(|project| project.name.clone()).collect(),
            )),
        }
    }

    /// What a project's repository is on, in a few words.
    pub fn branch(git: &ProjectGit) -> String {
        match git {
            ProjectGit::Branch { name } => name.clone(),
            ProjectGit::Detached { commit } => format!("at {commit}"),
            ProjectGit::Unreadable => "git".to_owned(),
            ProjectGit::None => "no git".to_owned(),
        }
    }

    /// Every command with what it does, for `/help`.
    pub fn help() -> Vec<Suggestion> {
        COMMANDS
            .iter()
            .map(|(command, detail)| Suggestion {
                label: format!("/{command}"),
                detail: (*detail).to_owned(),
                replacement: format!("/{command} "),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Slash, Unresolved};
    use ren_harness::Harness;
    use ren_supervisor::{Project, ProjectGit};

    fn projects() -> Vec<Project> {
        let project = |name: &str, path: &str| Project {
            path: path.into(),
            name: name.into(),
            git: ProjectGit::Branch {
                name: "main".into(),
            },
        };
        vec![
            project("acme-api", "/dev/acme-api"),
            project("acme-web", "/dev/acme-web"),
            project("ren", "/srv/ren"),
        ]
    }

    #[test]
    fn parses_new_with_and_without_a_client_and_prompt() {
        assert_eq!(
            Slash::parse("/new ren claude fix the flaky test"),
            Some(Slash::New {
                project: "ren".into(),
                client: Some(Harness::ClaudeCode),
                prompt: Some("fix the flaky test".into()),
            })
        );
        assert_eq!(
            Slash::parse("/new ren fix it"),
            Some(Slash::New {
                project: "ren".into(),
                client: None,
                prompt: Some("fix it".into()),
            })
        );
        assert_eq!(
            Slash::parse("/new ren codex"),
            Some(Slash::New {
                project: "ren".into(),
                client: Some(Harness::Codex),
                prompt: None,
            })
        );
        assert_eq!(Slash::parse("fix it"), None);
        assert_eq!(
            Slash::parse("/frobnicate"),
            Some(Slash::Unknown("frobnicate".into()))
        );
    }

    #[test]
    fn completes_commands_then_projects_then_clients() {
        let projects = projects();
        let labels = |text: &str| -> Vec<String> {
            Slash::complete(text, &projects)
                .into_iter()
                .map(|suggestion| suggestion.label)
                .collect()
        };
        assert_eq!(labels("/re"), ["/resume"]);
        assert_eq!(labels("/new acme"), ["acme-api", "acme-web"]);
        assert_eq!(
            Slash::complete("/new acme-a", &projects)[0].replacement,
            "/new acme-api "
        );
        assert_eq!(labels("/new ren c"), ["claude", "codex"]);
        assert!(labels("/new ren claude fix").is_empty());
        assert!(labels("plain text").is_empty());
    }

    #[test]
    fn resolves_a_project_by_name_or_a_unique_match() {
        let projects = projects();
        assert_eq!(
            Slash::resolve("ren", &projects).map(|p| p.path.as_str()),
            Ok("/srv/ren")
        );
        assert_eq!(
            Slash::resolve("api", &projects).map(|p| p.path.as_str()),
            Ok("/dev/acme-api")
        );
        assert_eq!(
            Slash::resolve("acme", &projects),
            Err(Unresolved::Ambiguous(vec![
                "acme-api".into(),
                "acme-web".into()
            ]))
        );
        assert_eq!(
            Slash::resolve("nope", &projects),
            Err(Unresolved::NoMatch("nope".into()))
        );
    }
}
