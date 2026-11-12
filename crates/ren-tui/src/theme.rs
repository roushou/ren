//! Rendering preferences. Terminal defaults stay symbolic, so the terminal
//! owns their actual colors, including when its palette changes.

use crate::wall::Tone;
use ratatui::style::{Color, Modifier, Style};
use ren_harness::Harness;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// A selectable, locally saved theme.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr,
)]
pub enum ThemeName {
    #[default]
    Terminal,
    Dark,
    Light,
}

impl ThemeName {
    pub fn theme(self) -> Theme {
        match self {
            Self::Terminal => Theme::default(),
            Self::Dark => Theme::dark(),
            Self::Light => Theme::light(),
        }
    }

    /// The next theme when previewing with the theme key.
    pub fn next(self) -> Self {
        match self {
            Self::Terminal => Self::Dark,
            Self::Dark => Self::Light,
            Self::Light => Self::Terminal,
        }
    }
}

/// Resolved styles for a wall. No environment, terminal queries, or I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub(crate) ground: Style,
    pub(crate) pane: Style,
    pub(crate) bar: Style,
    pub(crate) rule: Style,
    pub(crate) text: Style,
    pub(crate) muted: Style,
    pub(crate) code: Style,
    pub(crate) prompt: Style,
    pub(crate) tool: Style,
    pub(crate) success: Style,
    pub(crate) error: Style,
    pub(crate) warning: Style,
    pub(crate) focus: Style,
    pub(crate) responding_title: Style,
    pub(crate) error_title: Style,
    pub(crate) warning_title: Style,
    claude_code: Style,
    codex: Style,
    pi: Style,
    open_code: Style,
}

impl Default for Theme {
    fn default() -> Self {
        let text = Style::new().fg(Color::Reset);
        Self {
            ground: Style::reset(),
            pane: text.bg(Color::Reset),
            bar: text.bg(Color::Reset),
            rule: text,
            text,
            muted: text.add_modifier(Modifier::DIM),
            code: text,
            prompt: text.fg(Color::Magenta),
            tool: text.fg(Color::Blue),
            success: text.fg(Color::Green),
            error: text.fg(Color::Red),
            warning: text.fg(Color::Yellow),
            focus: text
                .bg(Color::Reset)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD)
                .remove_modifier(Modifier::DIM),
            responding_title: text
                .bg(Color::Reset)
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            error_title: text
                .bg(Color::Reset)
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD),
            warning_title: text
                .bg(Color::Reset)
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            claude_code: text.fg(Color::Yellow),
            codex: text.fg(Color::Blue),
            pi: text.fg(Color::Cyan),
            open_code: text.fg(Color::Magenta),
        }
    }
}

/// What a style is for, so views outside this crate draw with the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    /// The whole screen's background.
    Ground,
    /// A bar: the status line, an overlay.
    Bar,
    Rule,
    Text,
    Muted,
    Code,
    /// The operator's own words.
    Prompt,
    Tool,
    Success,
    Error,
    Warning,
    /// What has the keyboard.
    Focus,
    WarningTitle,
}

impl Theme {
    /// The style for `ink`.
    pub fn ink(&self, ink: Ink) -> Style {
        match ink {
            Ink::Ground => self.ground,
            Ink::Bar => self.bar,
            Ink::Rule => self.rule,
            Ink::Text => self.text,
            Ink::Muted => self.muted,
            Ink::Code => self.code,
            Ink::Prompt => self.prompt,
            Ink::Tool => self.tool,
            Ink::Success => self.success,
            Ink::Error => self.error,
            Ink::Warning => self.warning,
            Ink::Focus => self.focus,
            Ink::WarningTitle => self.warning_title,
        }
    }

    /// Terminal defaults with emphasis and labels, without color accents.
    pub fn monochrome() -> Self {
        let text = Style::new().fg(Color::Reset);
        let strong = text.add_modifier(Modifier::BOLD);
        Self {
            prompt: strong,
            tool: strong,
            success: text,
            error: strong,
            warning: strong,
            responding_title: strong.bg(Color::Reset),
            error_title: strong.bg(Color::Reset),
            warning_title: strong.bg(Color::Reset),
            claude_code: text,
            codex: text,
            pi: text,
            open_code: text,
            ..Self::default()
        }
    }

    fn dark() -> Self {
        let ground = Color::Rgb(0x0b, 0x0e, 0x12);
        let pane = Color::Rgb(0x0e, 0x12, 0x17);
        let text = Style::new().fg(Color::Rgb(0xc9, 0xd1, 0xd9));
        let success = Style::new().fg(Color::Rgb(0x7e, 0xe7, 0x87));
        let error = Style::new().fg(Color::Rgb(0xff, 0x7b, 0x72));
        let warning = Style::new().fg(Color::Rgb(0xe3, 0xb3, 0x41));
        Self {
            ground: Style::reset().patch(text).bg(ground),
            pane: text.bg(pane),
            bar: text.bg(Color::Rgb(0x16, 0x1b, 0x22)),
            rule: Style::new().fg(Color::Rgb(0x2a, 0x30, 0x38)),
            text,
            muted: Style::new().fg(Color::Rgb(0x6e, 0x76, 0x81)),
            code: Style::new().fg(Color::Rgb(0xe6, 0xed, 0xf3)),
            prompt: Style::new().fg(Color::Rgb(0xd2, 0xa8, 0xff)),
            tool: Style::new().fg(Color::Rgb(0x79, 0xc0, 0xff)),
            success,
            error,
            warning,
            focus: Style::new()
                .fg(ground)
                .bg(Color::Rgb(0xc9, 0xd1, 0xd9))
                .add_modifier(Modifier::BOLD),
            responding_title: Style::new().fg(ground).bg(Color::Rgb(0x7e, 0xe7, 0x87)),
            error_title: Style::new().fg(ground).bg(Color::Rgb(0xff, 0x7b, 0x72)),
            warning_title: Style::new().fg(ground).bg(Color::Rgb(0xe3, 0xb3, 0x41)),
            claude_code: Style::new().fg(Color::Rgb(0xf0, 0xa5, 0x81)),
            codex: Style::new().fg(Color::Rgb(0x9a, 0xb8, 0xe8)),
            pi: Style::new().fg(Color::Rgb(0x7f, 0xd4, 0xd9)),
            open_code: Style::new().fg(Color::Rgb(0xf7, 0x9c, 0xc7)),
        }
    }

    fn light() -> Self {
        let ground = Color::Rgb(0xff, 0xff, 0xff);
        let text = Style::new().fg(Color::Rgb(0x1f, 0x23, 0x28));
        let success = Style::new().fg(Color::Rgb(0x11, 0x63, 0x29));
        let error = Style::new().fg(Color::Rgb(0xa4, 0x0e, 0x26));
        let warning = Style::new().fg(Color::Rgb(0x7a, 0x4b, 0x00));
        Self {
            ground: Style::reset().patch(text).bg(ground),
            pane: text.bg(ground),
            bar: text.bg(Color::Rgb(0xea, 0xee, 0xf2)),
            rule: Style::new().fg(Color::Rgb(0x8c, 0x95, 0x9f)),
            text,
            muted: Style::new().fg(Color::Rgb(0x57, 0x60, 0x6a)),
            code: text,
            prompt: Style::new().fg(Color::Rgb(0x66, 0x39, 0xba)),
            tool: Style::new().fg(Color::Rgb(0x05, 0x50, 0xae)),
            success,
            error,
            warning,
            focus: Style::new()
                .fg(ground)
                .bg(Color::Rgb(0x1f, 0x23, 0x28))
                .add_modifier(Modifier::BOLD),
            responding_title: Style::new().fg(ground).bg(Color::Rgb(0x11, 0x63, 0x29)),
            error_title: Style::new().fg(ground).bg(Color::Rgb(0xa4, 0x0e, 0x26)),
            warning_title: Style::new().fg(ground).bg(Color::Rgb(0x7a, 0x4b, 0x00)),
            claude_code: Style::new().fg(Color::Rgb(0x95, 0x38, 0x00)),
            codex: Style::new().fg(Color::Rgb(0x05, 0x50, 0xae)),
            pi: Style::new().fg(Color::Rgb(0x0e, 0x63, 0x70)),
            open_code: Style::new().fg(Color::Rgb(0x82, 0x1b, 0x70)),
        }
    }

    pub(crate) fn tone(&self, tone: Tone) -> Style {
        match tone {
            Tone::Live => self.text,
            Tone::Success => self.success,
            Tone::Error => self.error,
            Tone::Warning => self.warning,
            Tone::Neutral => self.muted,
        }
    }

    pub(crate) fn harness(&self, harness: Option<Harness>) -> Style {
        match harness {
            Some(Harness::ClaudeCode) => self.claude_code,
            Some(Harness::Codex) => self.codex,
            Some(Harness::Pi) => self.pi,
            Some(Harness::OpenCode) => self.open_code,
            None => self.muted,
        }
    }
}
