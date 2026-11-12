//! Starting a run: a full-screen composer. The prompt owns the screen, as
//! long as it needs to be; the project and client are one header line,
//! defaulting to what was used last, each opened as a list under it when
//! changed. Below the prompt a line is kept for the files a prompt carries,
//! which Ren cannot add yet.
//!
//! The prompt has the keys first: typing, pasting, and moving through its
//! wrapped rows; Enter starts the run, Ctrl-J adds a line, Ctrl-E opens it
//! in the operator's editor. Tab moves to the header and back.

use super::composer::Composer;
use super::{Action, Ctx, Look, Notice};
use crate::Key;
use crate::model::{Model, Read};
use crate::ui::{
    Anchor, Bar, Emphasis, Ink, Keep, Line, Place, Rule, Size as Fit, Span, Text, View,
};
use ren_harness::Harness;
use ren_harness::drive::Prompt;
use ren_supervisor::{Client, ClientStatus, Project, ProjectGit};

/// The composer, as written so far.
#[derive(Debug)]
pub(crate) struct NewRun {
    prompt: Composer,
    /// The project's folder, once chosen; until then the last one used, or
    /// the first offered.
    project: Option<String>,
    client: Harness,
    focus: Focus,
}

/// What has the keys.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Focus {
    Prompt,
    /// A setting in the header, its list closed.
    Setting(Setting),
    /// A setting's list, open under the header.
    Picking(Picker),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setting {
    Project,
    Client,
}

/// A setting's list: what is typed to narrow it, and the entry chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Picker {
    setting: Setting,
    search: String,
    at: usize,
}

impl Setting {
    fn other(self) -> Self {
        match self {
            Self::Project => Self::Client,
            Self::Client => Self::Project,
        }
    }
}

impl NewRun {
    /// Rows a setting's list shows at once.
    const LISTED: usize = 10;
    /// The rows around the prompt: header, rules, files, and key bar.
    const CHROME: u16 = 6;
    /// Where the header's first setting starts.
    const SETTINGS_AT: u16 = 11;

    pub fn new(model: &Model) -> Self {
        Self {
            prompt: Composer::default(),
            project: model.project.clone(),
            client: model.client,
            focus: Focus::Prompt,
        }
    }

    pub fn shown(&mut self) -> Vec<Action> {
        vec![Action::Read(Read::Projects), Action::Read(Read::Clients)]
    }

    /// The prompt's width in a screen `width` wide: a margin each side.
    fn text_width(width: u16) -> usize {
        usize::from(width).saturating_sub(2)
    }

    fn text_height(height: u16) -> usize {
        usize::from(height.saturating_sub(Self::CHROME)).max(1)
    }

    /// The project a run would start in: the one chosen, if offered, else
    /// the first offered.
    fn project<'a>(&self, projects: &'a [Project]) -> Option<&'a Project> {
        self.project
            .as_ref()
            .and_then(|path| projects.iter().find(|project| &project.path == path))
            .or_else(|| projects.first())
    }

    /// Projects whose name or path holds `search`, ignoring case.
    fn matching<'a>(search: &str, projects: &'a [Project]) -> Vec<&'a Project> {
        let search = search.to_lowercase();
        projects
            .iter()
            .filter(|project| {
                project.name.to_lowercase().contains(&search)
                    || project.path.to_lowercase().contains(&search)
            })
            .collect()
    }

    pub fn paste(&mut self, text: &str) -> Vec<Action> {
        match &mut self.focus {
            Focus::Picking(picker) => picker
                .search
                .push_str(text.lines().next().unwrap_or_default()),
            Focus::Prompt | Focus::Setting(_) => {
                self.focus = Focus::Prompt;
                self.prompt.paste(text);
            }
        }
        Vec::new()
    }

    /// The prompt as the operator's editor gave it back.
    pub fn edited(&mut self, text: &str) -> Vec<Action> {
        self.prompt.set(text.trim_end_matches('\n'));
        self.focus = Focus::Prompt;
        Vec::new()
    }

    pub fn press(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        match self.focus.clone() {
            Focus::Prompt => self.write(key, ctx),
            Focus::Setting(setting) => self.setting(setting, key),
            Focus::Picking(picker) => self.pick(picker, key, ctx),
        }
    }

    fn write(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let width = Self::text_width(ctx.size.width);
        match key {
            Key::Enter => return self.start(ctx),
            Key::Esc | Key::Interrupt => return vec![Action::Pop],
            Key::Tab => self.focus = Focus::Setting(Setting::Project),
            Key::BackTab => self.focus = Focus::Setting(Setting::Client),
            Key::Editor => return vec![Action::Edit(self.prompt.text())],
            Key::Char(ch) => self.prompt.insert(ch),
            Key::Newline => self.prompt.insert('\n'),
            Key::Backspace => self.prompt.backspace(),
            Key::Delete => self.prompt.delete(),
            Key::Left => self.prompt.left(),
            Key::Right => self.prompt.right(),
            Key::Home => self.prompt.home(),
            Key::End => self.prompt.end(),
            Key::Up => {
                let _ = self.prompt.up(width);
            }
            Key::Down => {
                let _ = self.prompt.down(width);
            }
            Key::PageUp | Key::PageDown => self.prompt.page(
                width,
                Self::text_height(ctx.size.height),
                key == Key::PageDown,
            ),
            Key::AltLeft | Key::AltRight | Key::Older | Key::Newer => {}
        }
        Vec::new()
    }

    fn setting(&mut self, setting: Setting, key: Key) -> Vec<Action> {
        match key {
            Key::Left | Key::Right => self.focus = Focus::Setting(setting.other()),
            Key::Enter | Key::Down | Key::Char(' ') => {
                self.focus = Focus::Picking(Picker {
                    setting,
                    search: String::new(),
                    at: 0,
                });
            }
            Key::Char(ch) if setting == Setting::Project => {
                // Typing on the project searches it at once.
                self.focus = Focus::Picking(Picker {
                    setting,
                    search: ch.to_string(),
                    at: 0,
                });
            }
            Key::Tab | Key::BackTab | Key::Esc | Key::Up => self.focus = Focus::Prompt,
            Key::Interrupt => return vec![Action::Pop],
            Key::Char(_)
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
            | Key::Newer
            | Key::Editor => {}
        }
        Vec::new()
    }

    fn pick(&mut self, mut picker: Picker, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let count = match picker.setting {
            Setting::Project => Self::matching(&picker.search, ctx.model.projects()).len(),
            Setting::Client => ctx.model.clients().len(),
        };
        match key {
            Key::Up => picker.at = picker.at.saturating_sub(1),
            Key::Down => picker.at = (picker.at + 1).min(count.saturating_sub(1)),
            Key::Char(ch) if picker.setting == Setting::Project => {
                picker.search.push(ch);
                picker.at = 0;
            }
            Key::Backspace if picker.setting == Setting::Project => {
                picker.search.pop();
                picker.at = 0;
            }
            Key::Enter | Key::Tab => return self.choose(&picker, ctx.model),
            Key::Esc => {
                self.focus = Focus::Setting(picker.setting);
                return Vec::new();
            }
            Key::Interrupt => return vec![Action::Pop],
            Key::Char(_)
            | Key::BackTab
            | Key::Left
            | Key::Right
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
            | Key::Newer
            | Key::Editor => {}
        }
        self.focus = Focus::Picking(picker);
        Vec::new()
    }

    /// Take the picked entry, and give the keys back to the prompt.
    fn choose(&mut self, picker: &Picker, model: &Model) -> Vec<Action> {
        match picker.setting {
            Setting::Project => {
                if let Some(project) =
                    Self::matching(&picker.search, model.projects()).get(picker.at)
                {
                    self.project = Some(project.path.clone());
                }
            }
            Setting::Client => match model.clients().get(picker.at) {
                Some(Client {
                    harness,
                    status: ClientStatus::Ready,
                }) => self.client = *harness,
                Some(Client { harness, .. }) => {
                    return vec![Action::Say(Notice::refusal(format!(
                        "{} is not installed on the daemon's host",
                        Self::name(*harness)
                    )))];
                }
                None => {}
            },
        }
        self.focus = Focus::Prompt;
        Vec::new()
    }

    fn start(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let Some(project) = self.project(ctx.model.projects()) else {
            self.focus = Focus::Setting(Setting::Project);
            return vec![Action::Say(Notice::refusal("choose a project first"))];
        };
        let ready =
            ctx.model.clients().iter().any(|client| {
                client.harness == self.client && client.status == ClientStatus::Ready
            });
        if !ready {
            self.focus = Focus::Setting(Setting::Client);
            return vec![Action::Say(Notice::refusal(format!(
                "{} is not installed on the daemon's host",
                Self::name(self.client)
            )))];
        }
        let dir = project.path.clone();
        let prompt = Prompt::parse(self.prompt.text()).ok();
        ctx.model.client = self.client;
        ctx.model.project = Some(dir.clone());
        let command = ctx.model.start(self.client, dir, None, prompt);
        vec![Action::Pop, Action::Send(command)]
    }

    fn name(harness: Harness) -> &'static str {
        match harness {
            Harness::ClaudeCode => "Claude Code",
            Harness::Codex => "Codex",
            Harness::Pi => "Pi",
            Harness::OpenCode => "OpenCode",
        }
    }

    fn branch(git: &ProjectGit) -> String {
        match git {
            ProjectGit::Branch { name } => name.clone(),
            ProjectGit::Detached { commit } => format!("at {commit}"),
            ProjectGit::Unreadable => "git".to_owned(),
            ProjectGit::None => "no git".to_owned(),
        }
    }

    /// The whole screen: the header, the prompt, the files, and the keys.
    pub fn view(&self, look: &Look<'_>) -> View {
        let width = Self::text_width(look.size.width);
        let height = Self::text_height(look.size.height);
        let writing = self.focus == Focus::Prompt;
        let body = if self.prompt.is_empty() {
            // The placeholder sits where the text will start.
            let mut line = Line::new(vec![Span::plain(" ")]);
            if writing {
                line.push(Span::new(" ", Ink::Text).with(Emphasis::Reversed));
            }
            line.push(Span::new(
                "What should the agent work on? Type or paste it; ctrl-e opens your editor.",
                Ink::Muted,
            ));
            vec![line]
        } else {
            self.prompt
                .wrap(width)
                .lines(height, writing, |_| Span::plain(" "))
        };
        let screen = View::fill(
            Ink::Pane,
            View::Rows(vec![
                (Fit::Exact(1), View::Bar(self.header(look.model))),
                (Fit::Exact(1), View::Rule(Rule::Plain)),
                (Fit::Fill, View::lines(body, Anchor::Top)),
                (Fit::Exact(1), View::Rule(Rule::Plain)),
                (
                    Fit::Exact(1),
                    View::lines(
                        vec![Line::new(vec![
                            Span::new(" files   ", Ink::Muted),
                            Span::new("none", Ink::Muted),
                        ])],
                        Anchor::Top,
                    ),
                ),
                (Fit::Exact(1), View::Rule(Rule::Plain)),
                (Fit::Exact(1), View::Bar(self.keys(look))),
            ]),
        );
        match &self.focus {
            Focus::Picking(picker) => {
                let lines = self.list(picker, look.model);
                let widest = lines.iter().map(Line::width).max().unwrap_or(0);
                let tall = lines.len();
                View::layer(
                    screen,
                    View::Framed {
                        title: match picker.setting {
                            Setting::Project => " project ".to_owned(),
                            Setting::Client => " client ".to_owned(),
                        },
                        ink: Ink::Bar,
                        body: Box::new(View::lines(lines, Anchor::Top)),
                    },
                    Place::Under {
                        top: 1,
                        inset: match picker.setting {
                            Setting::Project => Self::SETTINGS_AT,
                            Setting::Client => self.client_at(look.model),
                        },
                        width: u16::try_from(widest + 4).unwrap_or(u16::MAX),
                        height: u16::try_from(tall + 2).unwrap_or(u16::MAX),
                    },
                )
            }
            Focus::Prompt | Focus::Setting(_) => screen,
        }
    }

    /// Where the client's setting starts in the header.
    fn client_at(&self, model: &Model) -> u16 {
        let before = Text(&self.project_label(model)).width() + 2 + 4;
        Self::SETTINGS_AT + u16::try_from(before).unwrap_or(u16::MAX)
    }

    fn project_label(&self, model: &Model) -> String {
        match self.project(model.projects()) {
            Some(project) => format!(
                "{}  {} · {}",
                project.name,
                project.path,
                Self::branch(&project.git)
            ),
            None => "choose a project".to_owned(),
        }
    }

    /// The header: the screen's name, then each setting, the one with the
    /// keys marked.
    fn header(&self, model: &Model) -> Bar {
        let marked = |setting: Setting| match &self.focus {
            Focus::Setting(at) => *at == setting,
            Focus::Picking(picker) => picker.setting == setting,
            Focus::Prompt => false,
        };
        let value = |setting: Setting, text: String| {
            let span = Span::new(format!(" {text} "), Ink::Text);
            if marked(setting) {
                span.with(Emphasis::Reversed)
            } else {
                span
            }
        };
        let left = Line::new(vec![
            Span::new(" new run ", Ink::LiveTitle).with(Emphasis::Bold),
            Span::plain(" "),
            Span::new("▸", Ink::Muted),
            value(Setting::Project, self.project_label(model)),
            Span::plain("   "),
            Span::new("▸", Ink::Muted),
            value(Setting::Client, Self::name(self.client).to_owned()),
        ]);
        Bar {
            left,
            right: Line::of(
                match self.focus {
                    Focus::Prompt => "tab settings ",
                    Focus::Setting(_) | Focus::Picking(_) => "tab back to the prompt ",
                },
                Ink::Muted,
            ),
            fill: Ink::Bar,
            keep: Keep::Left,
        }
    }

    /// An open setting's list, its entry marked.
    fn list(&self, picker: &Picker, model: &Model) -> Vec<Line> {
        let mark = |index: usize, spans: Vec<Span>| {
            let chosen = index == picker.at;
            let mut line = Line::new(vec![Span::new(
                if chosen { " › " } else { "   " },
                Ink::Prompt,
            )]);
            for span in spans {
                line.push(if chosen {
                    span.with(Emphasis::Bold)
                } else {
                    span
                });
            }
            line
        };
        match picker.setting {
            Setting::Project => {
                let projects = Self::matching(&picker.search, model.projects());
                let name = projects
                    .iter()
                    .map(|project| Text(&project.name).width())
                    .max()
                    .unwrap_or(0);
                let path = projects
                    .iter()
                    .map(|project| Text(&project.path).width())
                    .max()
                    .unwrap_or(0);
                let mut lines = vec![Line::new(vec![
                    Span::new(" search  ", Ink::Muted),
                    Span::new(format!("{}▏", picker.search), Ink::Text),
                ])];
                let first = picker.at.saturating_sub(Self::LISTED - 1);
                for (index, project) in projects.iter().enumerate().skip(first).take(Self::LISTED) {
                    lines.push(mark(
                        index,
                        vec![
                            Span::new(format!("{:<name$}  ", project.name), Ink::Text),
                            Span::new(format!("{:<path$}  ", project.path), Ink::Muted),
                            Span::new(Self::branch(&project.git), Ink::Muted),
                        ],
                    ));
                }
                if projects.is_empty() {
                    lines.push(Line::of("   no project matches", Ink::Muted));
                }
                lines
            }
            Setting::Client => model
                .clients()
                .iter()
                .enumerate()
                .map(|(index, client)| {
                    let spans = match client.status {
                        ClientStatus::Ready => {
                            vec![Span::new(Self::name(client.harness), Ink::Text)]
                        }
                        ClientStatus::NotInstalled | ClientStatus::NotDrivable => vec![
                            Span::new(Self::name(client.harness), Ink::Muted),
                            Span::new("  not installed", Ink::Muted),
                        ],
                    };
                    mark(index, spans)
                })
                .collect(),
        }
    }

    /// The key bar: what the keys do where the focus is, or what was said;
    /// the prompt's size on the right.
    fn keys(&self, look: &Look<'_>) -> Bar {
        let left = match look.notice {
            Some(notice) => Line::of(
                format!(" {}", notice.text),
                match notice.tone {
                    super::NoticeTone::Info => Ink::Muted,
                    super::NoticeTone::Refusal => Ink::Error,
                },
            ),
            None => Line::of(
                match &self.focus {
                    Focus::Prompt => {
                        " enter start · ctrl-j new line · ctrl-e your editor · tab settings · esc cancel"
                    }
                    Focus::Setting(_) => {
                        " ←→ project or client · enter change it · tab back to the prompt"
                    }
                    Focus::Picking(picker) => match picker.setting {
                        Setting::Project => {
                            " type to search · ↑↓ choose · enter take it · esc close"
                        }
                        Setting::Client => " ↑↓ choose · enter take it · esc close",
                    },
                },
                Ink::Muted,
            ),
        };
        let (lines, chars) = self.prompt.counts();
        Bar {
            left,
            right: Line::of(
                format!(
                    "{lines} line{} · {chars} char{} ",
                    if lines == 1 { "" } else { "s" },
                    if chars == 1 { "" } else { "s" }
                ),
                Ink::Muted,
            ),
            fill: Ink::Bar,
            keep: Keep::Right,
        }
    }
}
