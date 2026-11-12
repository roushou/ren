//! Slash commands, the console's way to act: typed in the message box,
//! completed with tab, and carried out the same from any console screen.

use super::{Action, Ctx, Notice, Resume, Screen};
use crate::model::desk::Attention;
use crate::model::{Model, Read};
use crate::ui::Text;
use ren_harness::{Harness, drive::Prompt};
use ren_recorder::id::RunId;
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
    /// `/pin`: keep the wall's focused session on it, even when quiet.
    Pin,
    /// `/theme`: the next theme, for this time.
    Theme,
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

/// What completion knows of: the projects a run can start in, and the
/// runs `/resume` can open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Known<'a> {
    pub projects: &'a [Project],
    pub runs: Vec<KnownRun>,
}

/// A run as completion offers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KnownRun {
    pub title: String,
    pub project: String,
    pub phase: &'static str,
}

impl<'a> Known<'a> {
    /// What `model` knows: every run `/resume` lists, what needs you first.
    pub fn of(model: &'a Model) -> Self {
        Self {
            projects: model.projects(),
            runs: model
                .desk
                .all()
                .into_iter()
                .map(|run| KnownRun {
                    title: run.title(),
                    project: run.project().to_owned(),
                    phase: run.phase(),
                })
                .collect(),
        }
    }
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
const COMMANDS: [(&str, &str); 8] = [
    ("new", "start a run: /new <project> [claude|codex] <prompt>"),
    ("resume", "every run, what needs you first"),
    ("stop", "end this run's process"),
    ("wall", "back to the wall of every session's traffic"),
    (
        "pin",
        "keep the wall's focused session on it, even when quiet",
    ),
    ("theme", "the next theme: terminal, dark, light (not saved)"),
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
            "pin" => Self::Pin,
            "theme" => Self::Theme,
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

    /// Carry the command out, from a screen showing `current`.
    pub(crate) fn act(self, ctx: &mut Ctx<'_>, current: Option<RunId>) -> Vec<Action> {
        match self {
            Self::New {
                project,
                client,
                prompt,
            } => {
                let found =
                    Self::resolve(&project, ctx.model.projects()).map(|found| found.path.clone());
                let dir = match found {
                    Ok(dir) => dir,
                    Err(Unresolved::Missing) => {
                        return vec![
                            Action::Say(Notice::info(
                                "name a project: /new <project> [claude|codex] <prompt>",
                            )),
                            Action::Read(Read::Projects),
                        ];
                    }
                    Err(Unresolved::NoMatch(_)) if project.starts_with('/') => project,
                    Err(Unresolved::NoMatch(name)) => {
                        return vec![
                            Action::Say(Notice::refusal(format!("no project matches {name}"))),
                            Action::Read(Read::Projects),
                        ];
                    }
                    Err(Unresolved::Ambiguous(names)) => {
                        return vec![Action::Say(Notice::refusal(format!(
                            "which one: {}",
                            names.join(", ")
                        )))];
                    }
                };
                let harness = client.unwrap_or(ctx.model.client);
                ctx.model.client = harness;
                let prompt = prompt.and_then(|prompt| Prompt::parse(prompt).ok());
                vec![Action::Send(ctx.model.start(harness, dir, None, prompt))]
            }
            Self::Resume { search } => vec![Action::Push(Screen::Resume(Resume::new(search)))],
            Self::Pin => vec![Action::Pin],
            Self::Theme => vec![Action::CycleTheme],
            Self::Stop => match current.and_then(|run| ctx.model.desk.get(run)) {
                Some(run) if run.attention() != Attention::Ended => {
                    vec![Action::Send(crate::model::Command::Stop(run.id))]
                }
                Some(_) | None => vec![Action::Say(Notice::info("no live run here to stop"))],
            },
            Self::Help => vec![Action::Help],
            Self::Wall => vec![Action::Wall],
            Self::Quit => vec![Action::Quit],
            Self::Unknown(name) => vec![Action::Say(Notice::refusal(format!(
                "no command /{name}: /help lists them"
            )))],
        }
    }

    /// Every command with what it does, for the help.
    pub(crate) fn commands() -> impl Iterator<Item = (&'static str, &'static str)> {
        COMMANDS.into_iter()
    }

    /// What completes `text`, the message box's whole text.
    pub fn complete(text: &str, known: &Known<'_>) -> Vec<Suggestion> {
        let projects = known.projects;
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
        match name {
            "new" => {}
            // A run, by its title or its project.
            "resume" => {
                let query = args.to_lowercase();
                return known
                    .runs
                    .iter()
                    .filter(|run| {
                        run.title != args
                            && (run.title.to_lowercase().contains(&query)
                                || run.project.to_lowercase().contains(&query))
                    })
                    .map(|run| Suggestion {
                        label: Text(&run.title).summary(40),
                        detail: format!("{} · {}", run.project, run.phase),
                        replacement: format!("/resume {}", run.title),
                    })
                    .collect();
            }
            _ => return Vec::new(),
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
}

#[cfg(test)]
mod tests {
    use super::{Known, KnownRun, Slash, Unresolved};
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
            Slash::complete(
                text,
                &Known {
                    projects: &projects,
                    runs: Vec::new(),
                },
            )
            .into_iter()
            .map(|suggestion| suggestion.label)
            .collect()
        };
        assert_eq!(labels("/re"), ["/resume"]);
        assert_eq!(labels("/new acme"), ["acme-api", "acme-web"]);
        assert_eq!(
            Slash::complete(
                "/new acme-a",
                &Known {
                    projects: &projects,
                    runs: Vec::new(),
                }
            )[0]
            .replacement,
            "/new acme-api "
        );
        assert_eq!(labels("/new ren c"), ["claude", "codex"]);
        assert!(labels("/new ren claude fix").is_empty());
        assert!(labels("plain text").is_empty());
    }

    #[test]
    fn resume_completes_runs_by_title_or_project() {
        let known = Known {
            projects: &[],
            runs: vec![
                KnownRun {
                    title: "fix the flaky upload test".into(),
                    project: "acme-web".into(),
                    phase: "needs you",
                },
                KnownRun {
                    title: "tidy the docs".into(),
                    project: "ren".into(),
                    phase: "ended",
                },
            ],
        };
        let labels = |text: &str| -> Vec<String> {
            Slash::complete(text, &known)
                .into_iter()
                .map(|suggestion| suggestion.label)
                .collect()
        };
        assert_eq!(
            labels("/resume "),
            ["fix the flaky upload test", "tidy the docs"]
        );
        assert_eq!(labels("/resume FLAKY"), ["fix the flaky upload test"]);
        assert_eq!(labels("/resume ren"), ["tidy the docs"]);
        assert_eq!(
            Slash::complete("/resume fix", &known)[0].replacement,
            "/resume fix the flaky upload test"
        );
        // Chosen already: nothing more to offer.
        assert!(labels("/resume tidy the docs").is_empty());
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
