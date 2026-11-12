//! Starting a run: a full-screen composer. It opens on where the run will
//! work: the folder is always chosen, never assumed, from a list with the
//! folders recent runs worked in first. Then the prompt owns the screen, as
//! long as it needs to be. Two header lines name where the run works and how
//! it is set up, each setting beside the key that changes it, which works
//! while typing: Ctrl-O the folder; Ctrl-L the client (the last one used,
//! until changed); Ctrl-K the model, from the models the client itself lists
//! on the daemon's host, with ← and → choosing the effort of the one
//! highlighted; Shift-Tab cycles what the client may do without asking.
//! Each left as `default` is the client's own choice. Below the
//! prompt a line is kept for the files a prompt carries, which Ren cannot add
//! yet.
//!
//! The prompt takes typing, pasting, and moving through its wrapped rows;
//! Enter starts the run (or, with no folder yet, opens the folder list),
//! Ctrl-J adds a line, Ctrl-E opens the prompt in the operator's editor.

use super::composer::Composer;
use super::{Action, Ctx, Look, Notice};
use crate::Key;
use crate::model::Offered;
use crate::model::desk::Run;
use crate::model::{Model, Read};
use crate::ui::{
    Anchor, Bar, Emphasis, Ink, Keep, Line, Place, Rule, Size as Fit, Span, Text, View,
};
use ren_harness::Harness;
use ren_harness::drive::{
    self, ApprovalPolicy, DefaultChoice, Effort, ModelId, Prompt, RunSettings,
};
use ren_supervisor::{Client, ClientStatus, Project, ProjectGit};

/// The composer, as written so far.
#[derive(Debug)]
pub(crate) struct NewRun {
    prompt: Composer,
    /// The project's folder, once chosen; until then the last one used, or
    /// the first offered.
    project: Option<String>,
    client: Harness,
    /// The model; none is the client's own default.
    model: Option<ModelId>,
    /// The effort; none is the model's own.
    effort: Option<Effort>,
    /// What the client may do without asking; none is its own setting.
    approvals: Option<ApprovalPolicy>,
    focus: Focus,
}

/// What has the keys.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Focus {
    Prompt,
    /// A setting's list, open under the header.
    Picking(Picker),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setting {
    Project,
    Client,
    Model,
}

/// A setting's list: what is typed to narrow it, and the entry chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Picker {
    setting: Setting,
    search: String,
    at: usize,
    /// The effort being chosen in the model list, for the model taken.
    effort: Option<Effort>,
}

impl NewRun {
    /// Rows a setting's list shows at once.
    const LISTED: usize = 10;
    /// The rows around the prompt: the two header lines, rules, files, and
    /// key bar.
    const CHROME: u16 = 7;
    /// Where a setting's list opens under the header.
    const LIST_AT: u16 = 2;
    /// The header's height.
    const HEADER: u16 = 2;

    /// The composer, open on choosing where the run works.
    /// The widest a model's description is shown in its list.
    const DESCRIBED: usize = 56;

    pub fn new(model: &Model) -> Self {
        let mut screen = Self {
            prompt: Composer::default(),
            project: None,
            client: model.client,
            model: None,
            effort: None,
            approvals: None,
            focus: Self::picking(Setting::Project),
        };
        screen.recall(model);
        screen
    }

    /// The settings last started with on this client.
    fn recall(&mut self, model: &Model) {
        let saved = model.chosen.get(&self.client).cloned().unwrap_or_default();
        self.model = saved.model;
        self.effort = saved.effort;
        self.approvals = saved
            .approvals
            .filter(|policy| ApprovalPolicy::offered(self.client).contains(policy));
    }

    fn picking(setting: Setting) -> Focus {
        Focus::Picking(Picker {
            setting,
            search: String::new(),
            at: 0,
            effort: None,
        })
    }

    /// The model list, open on the model and effort chosen.
    fn picking_model(&self, model: &Model) -> Focus {
        let at = self
            .model_choices("", model)
            .iter()
            .position(|choice| choice.map(|offered| &offered.id) == self.kept(model).as_ref())
            .unwrap_or(0);
        Focus::Picking(Picker {
            setting: Setting::Model,
            search: String::new(),
            at,
            effort: self.effort,
        })
    }

    /// The efforts a choice in the model list takes: the default's are its
    /// model's, when the client says which that is.
    fn efforts<'a>(&self, choice: Option<&'a drive::Model>, model: &'a Model) -> &'a [Effort] {
        choice
            .or_else(|| self.fallback(model))
            .map_or(&[], |offered| offered.efforts.as_slice())
    }

    /// `effort` moved one step along `efforts` (none, the model's own,
    /// comes first), or kept where it ends.
    fn step(effort: Option<Effort>, efforts: &[Effort], forward: bool) -> Option<Effort> {
        let at = effort.and_then(|effort| efforts.iter().position(|offered| *offered == effort));
        match (at, forward) {
            (None, true) => efforts.first().copied(),
            (None, false) => None,
            (Some(at), true) => efforts.get(at + 1).or(efforts.get(at)).copied(),
            (Some(0), false) => None,
            (Some(at), false) => efforts.get(at - 1).copied(),
        }
    }

    /// The next approval policy the client offers after the one chosen,
    /// back to its own setting after the last.
    fn cycle_approvals(&mut self) -> Vec<Action> {
        let offered = ApprovalPolicy::offered(self.client);
        if offered.is_empty() {
            return vec![Action::Say(Notice::info(format!(
                "{} asks for nothing: it runs every tool itself",
                Self::name(self.client)
            )))];
        }
        let at = self
            .approvals
            .and_then(|policy| offered.iter().position(|offered| *offered == policy));
        self.approvals = match at {
            None => offered.first().copied(),
            Some(at) => offered.get(at + 1).copied(),
        };
        Vec::new()
    }

    pub fn shown(&mut self, model: &mut Model) -> Vec<Action> {
        let mut reads = vec![Action::Read(Read::Projects), Action::Read(Read::Clients)];
        reads.extend(model.ask_models(self.client).map(Action::Read));
        reads
    }

    /// The client's models, as listed so far.
    fn listed<'a>(&self, model: &'a Model) -> &'a [drive::Model] {
        match model.models(self.client) {
            Some(Offered::Listed(models)) => models,
            Some(Offered::Asked | Offered::Failed(_)) | None => &[],
        }
    }

    /// The models whose name, id, provider, or description holds `search`,
    /// ignoring case, by provider in the order the client lists them; after
    /// the default, which is always first.
    fn model_choices<'a>(&self, search: &str, model: &'a Model) -> Vec<Option<&'a drive::Model>> {
        let search = search.to_lowercase();
        let matching: Vec<&drive::Model> = self
            .listed(model)
            .iter()
            .filter(|offered| {
                [
                    Some(offered.name.as_str()),
                    Some(offered.id.as_str()),
                    offered.provider.as_deref(),
                    offered.description.as_deref(),
                ]
                .into_iter()
                .flatten()
                .any(|text| text.to_lowercase().contains(&search))
            })
            .collect();
        let mut providers: Vec<Option<&str>> = Vec::new();
        for offered in &matching {
            if !providers.contains(&offered.provider.as_deref()) {
                providers.push(offered.provider.as_deref());
            }
        }
        let mut choices = vec![None];
        for provider in providers {
            choices.extend(
                matching
                    .iter()
                    .filter(|offered| offered.provider.as_deref() == provider)
                    .map(|offered| Some(*offered)),
            );
        }
        choices
    }

    /// The model the client uses when none is chosen, when it says.
    fn fallback<'a>(&self, model: &'a Model) -> Option<&'a drive::Model> {
        self.listed(model)
            .iter()
            .find(|offered| offered.choice == DefaultChoice::Default)
    }

    /// The chosen model, while the client still lists it (or has not
    /// listed yet).
    fn kept(&self, model: &Model) -> Option<ModelId> {
        let chosen = self.model.as_ref()?;
        match model.models(self.client) {
            Some(Offered::Listed(models)) if !models.is_empty() => models
                .iter()
                .any(|offered| &offered.id == chosen)
                .then(|| chosen.clone()),
            Some(Offered::Listed(_) | Offered::Asked | Offered::Failed(_)) | None => {
                Some(chosen.clone())
            }
        }
    }

    /// The prompt's width in a screen `width` wide: a margin each side.
    fn text_width(width: u16) -> usize {
        usize::from(width).saturating_sub(2)
    }

    fn text_height(height: u16) -> usize {
        usize::from(height.saturating_sub(Self::CHROME)).max(1)
    }

    /// The folder chosen, while it is offered.
    fn project<'a>(&self, projects: &'a [Project]) -> Option<&'a Project> {
        self.project
            .as_ref()
            .and_then(|path| projects.iter().find(|project| &project.path == path))
    }

    /// The folders whose name or path holds `search`, ignoring case: those
    /// recent runs worked in first, newest first, then the rest; and how
    /// many are recent.
    fn choices<'a>(search: &str, model: &'a Model) -> (Vec<&'a Project>, usize) {
        let search = search.to_lowercase();
        let matching: Vec<&Project> = model
            .projects()
            .iter()
            .filter(|project| {
                project.name.to_lowercase().contains(&search)
                    || project.path.to_lowercase().contains(&search)
            })
            .collect();
        let mut chosen: Vec<&Project> = model
            .desk
            .recent_folders()
            .into_iter()
            .filter_map(|folder| {
                matching
                    .iter()
                    .copied()
                    .find(|project| project.path == folder)
            })
            .take(5)
            .collect();
        let recent = chosen.len();
        chosen.extend(
            matching
                .into_iter()
                .filter(|project| !chosen.iter().any(|seen| seen.path == project.path))
                .collect::<Vec<_>>(),
        );
        (chosen, recent)
    }

    pub fn paste(&mut self, text: &str) -> Vec<Action> {
        match &mut self.focus {
            Focus::Picking(picker) => picker
                .search
                .push_str(text.lines().next().unwrap_or_default()),
            Focus::Prompt => self.prompt.paste(text),
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
            Focus::Picking(picker) => self.pick(picker, key, ctx),
        }
    }

    fn write(&mut self, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let width = Self::text_width(ctx.size.width);
        match key {
            Key::Enter => return self.start(ctx),
            Key::Esc | Key::Interrupt => return vec![Action::Pop],
            Key::Folder | Key::Tab => self.focus = Self::picking(Setting::Project),
            Key::Client => self.focus = Self::picking(Setting::Client),
            Key::BackTab => return self.cycle_approvals(),
            Key::Model => {
                self.focus = self.picking_model(ctx.model);
                return ctx
                    .model
                    .ask_models(self.client)
                    .map(Action::Read)
                    .into_iter()
                    .collect();
            }
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

    fn pick(&mut self, mut picker: Picker, key: Key, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let count = match picker.setting {
            Setting::Project => Self::choices(&picker.search, ctx.model).0.len(),
            Setting::Client => ctx.model.clients().len(),
            Setting::Model => self.model_choices(&picker.search, ctx.model).len(),
        };
        match key {
            Key::Up => picker.at = picker.at.saturating_sub(1),
            Key::Down => picker.at = (picker.at + 1).min(count.saturating_sub(1)),
            Key::Char(ch) if picker.setting != Setting::Client => {
                picker.search.push(ch);
                picker.at = 0;
            }
            Key::Backspace if picker.setting != Setting::Client => {
                picker.search.pop();
                picker.at = 0;
            }
            // The highlighted model's effort, one step down or up.
            Key::Left | Key::Right if picker.setting == Setting::Model => {
                let choices = self.model_choices(&picker.search, ctx.model);
                let choice = choices.get(picker.at).copied().flatten();
                let efforts = self.efforts(choice, ctx.model);
                picker.effort = Self::step(picker.effort, efforts, key == Key::Right);
            }
            Key::Enter | Key::Tab => return self.choose(&picker, ctx.model),
            // Back to the prompt, as it was: no folder is assumed.
            Key::Esc => {
                self.focus = Focus::Prompt;
                return Vec::new();
            }
            Key::Folder => {
                self.focus = Self::picking(Setting::Project);
                return Vec::new();
            }
            Key::Client => {
                self.focus = Self::picking(Setting::Client);
                return Vec::new();
            }
            Key::Model => {
                self.focus = self.picking_model(ctx.model);
                return ctx
                    .model
                    .ask_models(self.client)
                    .map(Action::Read)
                    .into_iter()
                    .collect();
            }
            Key::BackTab => {
                let said = self.cycle_approvals();
                self.focus = Focus::Picking(picker);
                return said;
            }
            Key::Interrupt => return vec![Action::Pop],
            Key::Char(_)
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
    fn choose(&mut self, picker: &Picker, model: &mut Model) -> Vec<Action> {
        match picker.setting {
            Setting::Project => {
                if let Some(project) = Self::choices(&picker.search, model).0.get(picker.at) {
                    self.project = Some(project.path.clone());
                }
            }
            Setting::Client => match model.clients().get(picker.at) {
                Some(Client {
                    harness,
                    status: ClientStatus::Ready,
                    ..
                }) => {
                    let harness = *harness;
                    self.focus = Focus::Prompt;
                    if harness == self.client {
                        return Vec::new();
                    }
                    // Another client's settings are its own.
                    self.client = harness;
                    self.recall(model);
                    return model
                        .ask_models(harness)
                        .map(Action::Read)
                        .into_iter()
                        .collect();
                }
                Some(Client { harness, .. }) => {
                    return vec![Action::Say(Notice::refusal(format!(
                        "{} is not installed on the daemon's host",
                        Self::name(*harness)
                    )))];
                }
                None => {}
            },
            Setting::Model => {
                if let Some(choice) = self.model_choices(&picker.search, model).get(picker.at) {
                    self.model = choice.map(|offered| offered.id.clone());
                    // Kept only where the model takes it.
                    self.effort = picker
                        .effort
                        .filter(|effort| self.efforts(*choice, model).contains(effort));
                }
            }
        }
        self.focus = Focus::Prompt;
        Vec::new()
    }

    fn start(&mut self, ctx: &mut Ctx<'_>) -> Vec<Action> {
        let Some(project) = self.project(ctx.model.projects()) else {
            self.focus = Self::picking(Setting::Project);
            return vec![Action::Say(Notice::info(
                "choose the folder the run works in, then enter starts it",
            ))];
        };
        let ready =
            ctx.model.clients().iter().any(|client| {
                client.harness == self.client && client.status == ClientStatus::Ready
            });
        if !ready {
            self.focus = Self::picking(Setting::Client);
            return vec![Action::Say(Notice::refusal(format!(
                "{} is not installed on the daemon's host",
                Self::name(self.client)
            )))];
        }
        let dir = project.path.clone();
        let prompt = Prompt::parse(self.prompt.text()).ok();
        let settings = RunSettings {
            model: self.kept(ctx.model),
            effort: self.effort,
            approvals: self.approvals,
        };
        ctx.model.client = self.client;
        ctx.model.chosen.insert(self.client, settings.clone());
        let command = ctx.model.start(self.client, dir, None, prompt, settings);
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
        let (place, setup) = self.header(look.model);
        let screen = View::fill(
            Ink::Pane,
            View::Rows(vec![
                (Fit::Exact(1), View::Bar(place)),
                (Fit::Exact(1), View::Bar(setup)),
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
                            Setting::Project => " folder ".to_owned(),
                            Setting::Client => " client ".to_owned(),
                            Setting::Model => format!(" {} models ", Self::name(self.client)),
                        },
                        ink: Ink::Bar,
                        body: Box::new(View::lines(lines, Anchor::Top)),
                    },
                    Place::Under {
                        top: Self::HEADER,
                        inset: Self::LIST_AT,
                        width: u16::try_from(widest + 4).unwrap_or(u16::MAX),
                        height: u16::try_from(tall + 2).unwrap_or(u16::MAX),
                    },
                )
            }
            Focus::Prompt => screen,
        }
    }

    fn project_label(&self, model: &Model) -> String {
        match self.project(model.projects()) {
            Some(project) => format!("{} · {}", project.name, Self::branch(&project.git)),
            None => "none yet".to_owned(),
        }
    }

    /// The header: the screen's name, then each setting with the key that
    /// changes it; the one being changed is marked, and a folder not yet
    /// chosen says so.
    fn header(&self, model: &Model) -> (Bar, Bar) {
        let open = match &self.focus {
            Focus::Picking(picker) => Some(picker.setting),
            Focus::Prompt => None,
        };
        let value = |setting: Option<Setting>, text: String, ink: Ink| {
            let span = Span::new(format!(" {text} "), ink);
            if setting.is_some() && open == setting {
                span.with(Emphasis::Reversed)
            } else {
                span
            }
        };
        let folder_ink = if self.project(model.projects()).is_some() {
            Ink::Text
        } else {
            Ink::Error
        };
        // Where: the folder, its full path at the right where there is room.
        let place = Line::new(vec![
            Span::new(" new run ", Ink::LiveTitle).with(Emphasis::Bold),
            Span::plain("  "),
            Span::new("ctrl-o", Ink::Tool),
            Span::new(" folder", Ink::Muted),
            value(
                Some(Setting::Project),
                self.project_label(model),
                folder_ink,
            ),
        ]);
        let path = match self.project(model.projects()) {
            Some(project) => Line::of(format!("{} ", project.path), Ink::Muted),
            None => Line::default(),
        };
        // How: the client, its model and effort, and what it may do alone.
        let approvals = if ApprovalPolicy::offered(self.client).is_empty() {
            "never asks".to_owned()
        } else {
            self.approvals.map_or("default", Run::policy).to_owned()
        };
        let setup = Line::new(vec![
            Span::plain(" ".repeat(Text(" new run ").width() + 2)),
            Span::new("ctrl-l", Ink::Tool),
            Span::new(" client", Ink::Muted),
            value(
                Some(Setting::Client),
                Self::name(self.client).to_owned(),
                Ink::Text,
            ),
            Span::plain("  "),
            Span::new("ctrl-k", Ink::Tool),
            Span::new(" model", Ink::Muted),
            value(Some(Setting::Model), self.model_label(model), Ink::Text),
            Span::plain("  "),
            Span::new("shift-tab", Ink::Tool),
            Span::new(" approvals", Ink::Muted),
            value(None, approvals, Ink::Text),
        ]);
        let bar = |left, right| Bar {
            left,
            right,
            fill: Ink::Bar,
            keep: Keep::Left,
        };
        (bar(place, path), bar(setup, Line::default()))
    }

    /// The model as the header names it: the chosen one's name, or the
    /// default and what it is now.
    fn model_label(&self, model: &Model) -> String {
        let name = self.model_name(model);
        match self.effort {
            Some(effort) => format!("{name} · {effort} effort"),
            None => name,
        }
    }

    fn model_name(&self, model: &Model) -> String {
        match self.kept(model) {
            Some(chosen) => self
                .listed(model)
                .iter()
                .find(|offered| offered.id == chosen)
                .map_or_else(|| chosen.to_string(), |offered| offered.name.clone()),
            None => match self.fallback(model) {
                Some(fallback) => format!("default · {}", fallback.name),
                None => "default".to_owned(),
            },
        }
    }

    /// The model list: the default first, then each provider's models,
    /// each with its description; what is still being asked, or why
    /// nothing is listed, at the foot.
    fn model_list(
        &self,
        picker: &Picker,
        model: &Model,
        mark: impl Fn(usize, Vec<Span>) -> Line,
    ) -> Vec<Line> {
        let choices = self.model_choices(&picker.search, model);
        let chosen = self.kept(model);
        let name = choices
            .iter()
            .map(|choice| choice.map_or(7, |offered| Text(&offered.name).width()))
            .max()
            .unwrap_or(0);
        let mut lines = Vec::new();
        if !picker.search.is_empty() || choices.len() > Self::LISTED {
            lines.push(Line::new(vec![
                Span::new(" search  ", Ink::Muted),
                Span::new(format!("{}▏", picker.search), Ink::Text),
            ]));
        }
        let first = picker.at.saturating_sub(Self::LISTED - 1);
        let mut provider: Option<Option<&str>> = None;
        for (index, choice) in choices.iter().enumerate().skip(first).take(Self::LISTED) {
            let current = choice.map(|offered| &offered.id) == chosen.as_ref();
            let tick = Span::new(if current { "● " } else { "  " }, Ink::Prompt);
            match choice {
                None => {
                    let about = match self.fallback(model) {
                        Some(fallback) => format!(
                            "{}'s own choice, now {}",
                            Self::name(self.client),
                            fallback.name
                        ),
                        None => format!("{}'s own choice", Self::name(self.client)),
                    };
                    lines.push(mark(
                        index,
                        vec![
                            tick,
                            Span::new(format!("{:<name$}  ", "default"), Ink::Text),
                            Span::new(about, Ink::Muted),
                        ],
                    ));
                }
                Some(offered) => {
                    let group = offered.provider.as_deref();
                    if provider != Some(group) {
                        provider = Some(group);
                        if let Some(group) = group {
                            lines.push(Line::of(format!(" {group}"), Ink::Muted));
                        }
                    }
                    let mut spans = vec![
                        tick,
                        Span::new(format!("{:<name$}  ", offered.name), Ink::Text),
                        Span::new(
                            Text(
                                offered
                                    .description
                                    .as_deref()
                                    .unwrap_or(offered.id.as_str()),
                            )
                            .summary(Self::DESCRIBED),
                            Ink::Muted,
                        ),
                    ];
                    if offered.choice == DefaultChoice::Default {
                        spans.push(Span::new("  default", Ink::Tool));
                    }
                    lines.push(mark(index, spans));
                }
            }
        }
        // The highlighted model's efforts, the one to take marked.
        let highlighted = choices.get(picker.at).copied().flatten();
        let efforts = self.efforts(highlighted, model);
        if !efforts.is_empty() {
            let mut line = Line::new(vec![Span::new("   ←→ effort  ", Ink::Muted)]);
            let own = highlighted
                .or_else(|| self.fallback(model))
                .and_then(|offered| offered.effort);
            let default = match own {
                Some(own) => format!("default ({own})"),
                None => "default".to_owned(),
            };
            for (label, effort) in std::iter::once((default, None))
                .chain(efforts.iter().map(|e| (e.to_string(), Some(*e))))
            {
                let span = Span::new(format!(" {label} "), Ink::Text);
                line.push(if effort == picker.effort {
                    span.with(Emphasis::Reversed)
                } else {
                    span
                });
            }
            lines.push(line);
        }
        let note = match model.models(self.client) {
            Some(Offered::Asked) | None => Some(format!(
                "asking {} for its models…",
                Self::name(self.client)
            )),
            Some(Offered::Failed(why)) => Some(why.clone()),
            Some(Offered::Listed(models)) if models.is_empty() => Some(format!(
                "{} lists no models here; is it signed in on the daemon's host?",
                Self::name(self.client)
            )),
            Some(Offered::Listed(_)) if choices.len() == 1 && !picker.search.is_empty() => {
                Some("no model matches".to_owned())
            }
            Some(Offered::Listed(_)) => None,
        };
        if let Some(note) = note {
            lines.push(Line::of(format!("   {note}"), Ink::Muted));
        }
        lines
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
                let (projects, recent) = Self::choices(&picker.search, model);
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
                    if index == 0 && recent > 0 {
                        lines.push(Line::of(" recent", Ink::Muted));
                    }
                    if index == recent && recent > 0 {
                        lines.push(Line::of(" all folders", Ink::Muted));
                    }
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
                    lines.push(Line::of("   no folder matches", Ink::Muted));
                }
                lines
            }
            Setting::Model => self.model_list(picker, model, mark),
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
                        " enter start · ctrl-j new line · ctrl-e your editor · esc cancel"
                    }
                    Focus::Picking(picker) => match picker.setting {
                        Setting::Project => {
                            " where should the run work? type to search · ↑↓ choose · enter take it · esc back to the prompt"
                        }
                        Setting::Client => " ↑↓ choose · enter take it · esc back to the prompt",
                        Setting::Model => {
                            " type to search · ↑↓ choose · ←→ effort · enter take it · esc back to the prompt"
                        }
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
