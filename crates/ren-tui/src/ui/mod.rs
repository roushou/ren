//! What screens draw with. Styled text ([`Span`], [`Line`]) names what each
//! piece is for ([`Ink`]), never a color, and a small layout tree
//! ([`View`]) places it. A screen builds a view without a terminal;
//! [`View::render`] (`draw`) is the one renderer, which puts any view on a
//! ratatui buffer with a resolved theme.

mod draw;
mod text;

pub use text::Text;

use ren_harness::Harness;

/// What a piece of text is for; the theme gives each its style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    /// Whatever is under it: no style of its own.
    Same,
    /// The whole screen's background.
    Ground,
    /// A pane's background.
    Pane,
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
    /// A title bar lit while a response streams, or by a problem.
    LiveTitle,
    ErrorTitle,
    WarningTitle,
    /// A client's own color; muted when the client is not known.
    Client(Option<Harness>),
}

/// How something reads at a glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Live,
    Error,
    Warning,
    Success,
    Neutral,
}

impl Ink {
    /// The ink a tone is written in.
    pub fn of(tone: Tone) -> Self {
        match tone {
            Tone::Live => Self::Text,
            Tone::Success => Self::Success,
            Tone::Error => Self::Error,
            Tone::Warning => Self::Warning,
            Tone::Neutral => Self::Muted,
        }
    }
}

/// How a span's text is set, beside its ink.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Emphasis {
    #[default]
    Plain,
    Bold,
    Italic,
    /// Colors swapped: the cursor.
    Reversed,
}

/// Text in one ink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub ink: Ink,
    pub emphasis: Emphasis,
}

impl Span {
    pub fn new(text: impl Into<String>, ink: Ink) -> Self {
        Self {
            text: text.into(),
            ink,
            emphasis: Emphasis::Plain,
        }
    }

    /// Text with no style of its own.
    pub fn plain(text: impl Into<String>) -> Self {
        Self::new(text, Ink::Same)
    }

    #[must_use]
    pub fn with(mut self, emphasis: Emphasis) -> Self {
        self.emphasis = emphasis;
        self
    }

    pub fn width(&self) -> usize {
        Text(&self.text).width()
    }
}

/// One row of spans.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    pub spans: Vec<Span>,
}

impl Line {
    pub fn new(spans: Vec<Span>) -> Self {
        Self { spans }
    }

    /// One span's line.
    pub fn of(text: impl Into<String>, ink: Ink) -> Self {
        Self::new(vec![Span::new(text, ink)])
    }

    pub fn push(&mut self, span: Span) {
        self.spans.push(span);
    }

    pub fn width(&self) -> usize {
        self.spans.iter().map(Span::width).sum()
    }

    /// Its text without styles.
    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    /// `prefix` then `text` wrapped to `width`, later rows indented under
    /// the text.
    pub fn hanging(prefix: &str, text: &str, width: usize, ink: Ink) -> Vec<Self> {
        let indent = Text(prefix).width();
        Text(text)
            .wrap(width.saturating_sub(indent))
            .into_iter()
            .enumerate()
            .map(|(row, part)| {
                let lead = if row == 0 {
                    prefix.to_owned()
                } else {
                    " ".repeat(indent)
                };
                Self::of(format!("{lead}{part}"), ink)
            })
            .collect()
    }
}

impl From<Span> for Line {
    fn from(span: Span) -> Self {
        Self::new(vec![span])
    }
}

/// A layout: what goes where in the area it is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Empty,
    Lines(Lines),
    Bar(Bar),
    Rule(Rule),
    /// Rows top-down: fixed heights, and what is left shared by `Fill`s.
    Rows(Vec<(Size, View)>),
    /// Columns left to right, with a one-cell rule between each two.
    Columns(Vec<(Size, View)>),
    /// Equal cells, across then down, with one-cell rules between them.
    Grid(Grid),
    /// The area filled with an ink, then the view on it.
    Fill(Ink, Box<View>),
    /// The view with columns left free on each side.
    Pad {
        left: u16,
        right: u16,
        view: Box<View>,
    },
    /// `over` laid on `under`, in a place of its own.
    Layer {
        under: Box<View>,
        over: Box<View>,
        place: Place,
    },
    /// A bordered box with a title, filled with an ink.
    Framed {
        title: String,
        ink: Ink,
        body: Box<View>,
    },
}

/// A row's height or a column's width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Exact(u16),
    /// A share of what the exact ones leave.
    Fill,
}

/// Lines of text, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lines {
    pub lines: Vec<Line>,
    pub anchor: Anchor,
    /// Lines scrolled up from the newest, for bottom-anchored lines; a
    /// note says how many are below.
    pub scroll: usize,
}

/// Where lines sit when there is more room than lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Top,
    /// The newest lines show, against the bottom.
    Bottom,
    /// The newest lines show, from the top: a conversation that has not
    /// filled the screen yet starts at its top.
    Newest,
    /// Centered both ways.
    Middle,
}

/// One row: `left`, and `right` against the right edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bar {
    pub left: Line,
    pub right: Line,
    /// The row's ink, under both.
    pub fill: Ink,
    pub keep: Keep,
}

/// Which side of a bar stays when both do not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// `right` shows only with two free cells between them.
    Left,
    /// `right` always shows; `left` is cut short of it.
    Right,
}

/// A horizontal rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// `─` in the rule's ink.
    Plain,
    /// `▀` in the color of a lit title bar above it, so the two read as one.
    Edge(Ink),
    /// `─` meeting a column's `│` `at` cells in: `┬` above the columns, `┴`
    /// below them.
    Joined { at: u16, joint: Joint },
}

/// Where a rule meets a column's rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joint {
    /// The rule is above the columns: `┬`.
    Above,
    /// The rule is below them: `┴`.
    Below,
}

/// Equal cells with rules between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    pub columns: u16,
    pub rows: u16,
    /// Across, then down.
    pub cells: Vec<View>,
}

/// Where a layer goes over what is under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Centered, at most this big.
    Center { width: u16, height: u16 },
    /// Against the bottom, `inset` cells from the left, at most this big.
    Bottom { inset: u16, width: u16, height: u16 },
}

impl View {
    pub fn lines(lines: Vec<Line>, anchor: Anchor) -> Self {
        Self::Lines(Lines {
            lines,
            anchor,
            scroll: 0,
        })
    }

    pub fn bar(left: Line, right: Line, fill: Ink, keep: Keep) -> Self {
        Self::Bar(Bar {
            left,
            right,
            fill,
            keep,
        })
    }

    pub fn fill(ink: Ink, view: Self) -> Self {
        Self::Fill(ink, Box::new(view))
    }

    pub fn pad(left: u16, right: u16, view: Self) -> Self {
        Self::Pad {
            left,
            right,
            view: Box::new(view),
        }
    }

    pub fn layer(under: Self, over: Self, place: Place) -> Self {
        Self::Layer {
            under: Box::new(under),
            over: Box::new(over),
            place,
        }
    }
}

impl Grid {
    /// Where the `part`-th of `parts` equal shares of `total` starts.
    pub fn start(total: u16, parts: u16, part: u16) -> u16 {
        let total = u32::from(total);
        u16::try_from(total * u32::from(part) / u32::from(parts.max(1))).unwrap_or(u16::MAX)
    }

    /// A cell's width and height in an area `width` by `height`.
    pub fn cell(&self, width: u16, height: u16, index: usize) -> (u16, u16) {
        let columns = self.columns.max(1);
        let rows = self.rows.max(1);
        let index = u16::try_from(index).unwrap_or(u16::MAX);
        let (column, row) = (index % columns, index / columns);
        let width = width.saturating_sub(columns - 1);
        let height = height.saturating_sub(rows - 1);
        (
            Self::start(width, columns, column + 1) - Self::start(width, columns, column),
            Self::start(height, rows, row + 1) - Self::start(height, rows, row),
        )
    }
}
