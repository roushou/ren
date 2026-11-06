//! Terminal styling for human-readable output. Whether to color is decided
//! once per stream, from the environment and whether that stream is a
//! terminal; machine output (`--json`, pipes) stays plain.
use anstyle::{AnsiColor, Style};
use std::io::IsTerminal as _;
use tgv_harness::host::HostEnv;

/// How each output stream is styled. Stdout and stderr are decided apart:
/// either can be piped while the other is a terminal.
#[derive(Clone, Copy, Debug)]
pub struct Terminal {
    pub stdout: Styling,
    pub stderr: Styling,
}

impl Terminal {
    pub fn detect(env: &HostEnv) -> Self {
        Self {
            stdout: Styling::detect(env, std::io::stdout().is_terminal()),
            stderr: Styling::detect(env, std::io::stderr().is_terminal()),
        }
    }
}

/// Whether output is colored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Styling {
    Color,
    Plain,
}

/// The roles text plays in output, each with one style.
#[derive(Clone, Copy)]
pub enum Tone {
    Good,
    Warn,
    Bad,
    Muted,
    Strong,
}

impl Styling {
    /// Color only on a terminal, unless `NO_COLOR` is set or `TERM` is
    /// `dumb`; `CLICOLOR_FORCE` (other than `0`) colors even when piped.
    /// See <https://no-color.org> and <https://bixense.com/clicolors>.
    pub fn detect(env: &HostEnv, terminal: bool) -> Self {
        let forced = env
            .get("CLICOLOR_FORCE")
            .is_some_and(|value| value.trim() != "0");
        let dumb = env.get("TERM").is_some_and(|term| term.trim() == "dumb");
        if env.is_set("NO_COLOR") {
            Self::Plain
        } else if forced || (terminal && !dumb) {
            Self::Color
        } else {
            Self::Plain
        }
    }

    pub fn is_color(self) -> bool {
        match self {
            Self::Color => true,
            Self::Plain => false,
        }
    }

    /// `text` in `tone`, or unchanged when plain.
    pub fn paint(self, tone: Tone, text: &str) -> String {
        match self {
            Self::Plain => text.to_owned(),
            Self::Color => {
                let style = tone.style();
                format!("{}{text}{}", style.render(), style.render_reset())
            }
        }
    }
}

impl Tone {
    fn style(self) -> Style {
        let color = |color: AnsiColor| Style::new().fg_color(Some(color.into()));
        match self {
            Self::Good => color(AnsiColor::Green).bold(),
            Self::Warn => color(AnsiColor::Yellow).bold(),
            Self::Bad => color(AnsiColor::Red).bold(),
            Self::Muted => Style::new().dimmed(),
            Self::Strong => Style::new().bold(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Styling, Tone};
    use tgv_harness::host::HostEnv;

    #[test]
    fn color_follows_the_terminal_and_the_conventions() {
        let none = HostEnv::default();
        assert_eq!(Styling::detect(&none, true), Styling::Color);
        assert_eq!(Styling::detect(&none, false), Styling::Plain);
        let no_color = HostEnv::from_pairs([("NO_COLOR", "1"), ("CLICOLOR_FORCE", "1")]);
        assert_eq!(Styling::detect(&no_color, true), Styling::Plain);
        let forced = HostEnv::from_pairs([("CLICOLOR_FORCE", "1")]);
        assert_eq!(Styling::detect(&forced, false), Styling::Color);
        let unforced = HostEnv::from_pairs([("CLICOLOR_FORCE", "0")]);
        assert_eq!(Styling::detect(&unforced, false), Styling::Plain);
        let dumb = HostEnv::from_pairs([("TERM", "dumb")]);
        assert_eq!(Styling::detect(&dumb, true), Styling::Plain);
    }

    #[test]
    fn plain_text_has_no_escapes() {
        assert_eq!(Styling::Plain.paint(Tone::Bad, "FAIL"), "FAIL");
        assert!(Styling::Color.paint(Tone::Bad, "FAIL").contains("\u{1b}["));
    }
}
