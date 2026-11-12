//! Starting a run: a form over the main screen. Type to find the project
//! (by name or path, the first match chosen), pick the client, and write
//! the first prompt; Enter on the prompt starts it.

use super::composer::Composer;
use super::{Action, Ctx, Look, Notice};
use crate::Key;
use crate::model::Read;
use crate::ui::{Anchor, Emphasis, Ink, Line, Place, Span, Text, View};
use ren_harness::Harness;
use ren_harness::drive::Prompt;
use ren_supervisor::{Client, ClientStatus, Project, ProjectGit};

/// The form, as filled in so far.
#[derive(Debug)]
pub(crate) struct NewRun {
    field: Field,
    search: String,
    chosen: usize,
    client: Harness,
    prompt: Composer,
}

/// The field that has the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Project,
    Client,
    Prompt,
}

impl NewRun {
    /// Projects shown at once.
    const SHOWN: usize = 8;

    pub fn new(client: Harness) -> Self {
        Self {
            field: Field::Project,
            search: String::new(),
            chosen: 0,
            client,
            prompt: Composer::default(),
        }
    }

    pub fn shown(&mut self) -> Vec<Action> {
        vec![Action::Read(Read::Projects), Action::Read(Read::Clients)]
    }

    /// Projects whose name or path holds the search, ignoring case.
    fn matching<'a>(&self, projects: &'a [Project]) -> Vec<&'a Project> {
        let search = self.search.to_lowercase();
        projects
            .iter()
            .filter(|project| {
                project.name.to_lowercase().contains(&search)
                    || project.path.to_lowercase().contains(&search)
            })
            .collect()
    }

    pub fn press(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        match (key, self.field) {
            (Key::Esc | Key::Interrupt, _) => return vec![Action::Pop],
            (Key::Tab, _) => self.field = self.field.next(),
            (Key::BackTab, _) => self.field = self.field.previous(),
            (Key::Char(ch), Field::Project) => {
                self.search.push(ch);
                self.chosen = 0;
            }
            (Key::Backspace, Field::Project) => {
                self.search.pop();
                self.chosen = 0;
            }
            (Key::Up, Field::Project) => self.chosen = self.chosen.saturating_sub(1),
            (Key::Down, Field::Project) => {
                let count = self.matching(ctx.model.projects()).len();
                self.chosen = (self.chosen + 1).min(count.saturating_sub(1));
            }
            (Key::Enter, Field::Project | Field::Client) => self.field = self.field.next(),
            (Key::Right | Key::Char(' '), Field::Client) => self.step(&ctx.model.clients(), false),
            (Key::Left, Field::Client) => self.step(&ctx.model.clients(), true),
            (Key::Enter, Field::Prompt) => return self.start(ctx),
            (Key::Char(ch), Field::Prompt) => self.prompt.insert(ch),
            (Key::Newline, Field::Prompt) => self.prompt.insert('\n'),
            (Key::Backspace, Field::Prompt) => self.prompt.backspace(),
            (Key::Delete, Field::Prompt) => self.prompt.delete(),
            (Key::Left, Field::Prompt) => self.prompt.left(),
            (Key::Right, Field::Prompt) => self.prompt.right(),
            (Key::Home, Field::Prompt) => self.prompt.home(),
            (Key::End, Field::Prompt) => self.prompt.end(),
            (
                Key::Char(_)
                | Key::Left
                | Key::Right
                | Key::Up
                | Key::Down
                | Key::Backspace
                | Key::Delete
                | Key::Home
                | Key::End
                | Key::PageUp
                | Key::PageDown
                | Key::AltLeft
                | Key::AltRight
                | Key::Newline
                | Key::Older
                | Key::Newer,
                _,
            ) => {}
        }
        Vec::new()
    }

    /// The next (or, `back`, the previous) client a run can start with
    /// here; the first one when the chosen client is not among them.
    fn step(&mut self, clients: &[Client], back: bool) {
        let ready: Vec<Harness> = clients
            .iter()
            .filter(|client| client.status == ClientStatus::Ready)
            .map(|client| client.harness)
            .collect();
        let next = match ready.iter().position(|&harness| harness == self.client) {
            Some(at) if back => (at + ready.len() - 1) % ready.len(),
            Some(at) => (at + 1) % ready.len(),
            None => 0,
        };
        if let Some(&harness) = ready.get(next) {
            self.client = harness;
        }
    }

    fn start(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let clients = ctx.model.clients();
        match clients.iter().find(|client| client.harness == self.client) {
            Some(Client {
                status: ClientStatus::Ready,
                ..
            }) => {}
            Some(Client {
                status: ClientStatus::NotInstalled | ClientStatus::NotDrivable,
                ..
            })
            | None => {
                self.field = Field::Client;
                return vec![Action::Say(Notice::refusal(format!(
                    "{} is not installed on the daemon's host",
                    Self::name(self.client)
                )))];
            }
        }
        let Some(project) = self
            .matching(ctx.model.projects())
            .get(self.chosen)
            .copied()
        else {
            self.field = Field::Project;
            return vec![Action::Say(Notice::refusal("choose a project first"))];
        };
        let dir = project.path.clone();
        let prompt = Prompt::parse(self.prompt.text()).ok();
        ctx.model.client = self.client;
        let command = ctx.model.start(self.client, dir, None, prompt);
        vec![Action::Pop, Action::Send(command)]
    }

    pub fn view(&self, under: View, look: &Look<'_>) -> View {
        let projects = self.matching(look.model.projects());
        let chosen = self.chosen.min(projects.len().saturating_sub(1));
        let field = |is: Field| {
            if self.field == is {
                Ink::Focus
            } else {
                Ink::Muted
            }
        };
        let mut lines = vec![Line::new(vec![
            Span::new(" project ", field(Field::Project)),
            Span::new(format!("  {}▏", self.search), Ink::Text),
        ])];
        let first = chosen.saturating_sub(Self::SHOWN - 1);
        for (index, project) in projects.iter().enumerate().skip(first).take(Self::SHOWN) {
            let mark = if index == chosen { "› " } else { "  " };
            let branch = match &project.git {
                ProjectGit::Branch { name } => name.clone(),
                ProjectGit::Detached { commit } => format!("at {commit}"),
                ProjectGit::Unreadable => "git".to_owned(),
                ProjectGit::None => "no git".to_owned(),
            };
            lines.push(Line::new(vec![
                Span::new(
                    format!("   {mark}{}", project.name),
                    if index == chosen {
                        Ink::Text
                    } else {
                        Ink::Muted
                    },
                ),
                Span::new(
                    format!("  {}  {branch}", Text(&project.path).fit(40)),
                    Ink::Muted,
                ),
            ]));
        }
        if projects.is_empty() {
            lines.push(Line::of("     no project matches", Ink::Muted));
        }
        lines.push(Line::default());
        let mut client = Line::new(vec![
            Span::new(" client  ", field(Field::Client)),
            Span::plain("  "),
        ]);
        for Client { harness, status } in look.model.clients() {
            let (mark, ink) = if harness == self.client {
                ("◉", Ink::Text)
            } else {
                ("○", Ink::Muted)
            };
            let name = Self::name(harness);
            match status {
                ClientStatus::Ready => client.push(Span::new(format!("{mark} {name}   "), ink)),
                ClientStatus::NotInstalled | ClientStatus::NotDrivable => client.push(Span::new(
                    format!("{mark} {name} (not installed)   "),
                    Ink::Muted,
                )),
            }
        }
        lines.push(client);
        lines.push(Line::default());
        lines.push(Line::new(vec![Span::new(
            " prompt  ",
            field(Field::Prompt),
        )]));
        let text = self.prompt.text();
        for (row, part) in text.split('\n').enumerate() {
            let mut line = Line::new(vec![
                Span::plain("   "),
                Span::new(part.to_owned(), Ink::Text),
            ]);
            let last = row == text.matches('\n').count();
            if last && self.field == Field::Prompt {
                line.push(Span::new(" ", Ink::Text).with(Emphasis::Reversed));
            }
            lines.push(line);
        }
        lines.push(Line::default());
        lines.push(Line::of(
            " tab next field · ↑↓ project · ←→ client · enter start · esc cancel",
            Ink::Muted,
        ));
        let height = u16::try_from(lines.len() + 2).unwrap_or(u16::MAX);
        View::layer(
            under,
            View::Framed {
                title: " new run ".to_owned(),
                ink: Ink::Bar,
                body: Box::new(View::lines(lines, Anchor::Top)),
            },
            Place::Center {
                width: look.size.width.saturating_sub(8).min(90),
                height,
            },
        )
    }
}

impl NewRun {
    fn name(harness: Harness) -> &'static str {
        match harness {
            Harness::ClaudeCode => "Claude Code",
            Harness::Codex => "Codex",
            Harness::Pi => "Pi",
            Harness::OpenCode => "OpenCode",
        }
    }
}

impl Field {
    fn next(self) -> Self {
        match self {
            Self::Project => Self::Client,
            Self::Client => Self::Prompt,
            Self::Prompt => Self::Project,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Project => Self::Prompt,
            Self::Client => Self::Project,
            Self::Prompt => Self::Client,
        }
    }
}
