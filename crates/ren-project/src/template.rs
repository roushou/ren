//! Text that names the place a step runs for: `{port}`, `{port+N}`,
//! `{data}`, and `{worktree}`, parsed once into literal pieces and
//! placeholders. `{{` and `}}` are a literal brace.

use crate::Place;

/// A value or argument, with the placeholders it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pieces: Vec<Piece>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Place(Placeholder),
}

/// What a placeholder names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placeholder {
    /// The place's first port, plus this many.
    Port(u16),
    Data,
    Worktree,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error(
        "{0:?} names no placeholder Ren knows (`{{port}}`, `{{port+N}}`, `{{data}}`, `{{worktree}}`)"
    )]
    Placeholder(String),
    #[error("an unclosed or stray brace in {0:?}; write `{{{{` or `}}}}` for a brace")]
    Brace(String),
    #[error("port +{0} is past the place's ports")]
    PortRange(u16),
    #[error("a port is named outside a worktree, which has none")]
    NoPorts,
    #[error("the place's folder is not text: {0}")]
    Path(String),
}

impl std::str::FromStr for Template {
    type Err = TemplateError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let brace = || TemplateError::Brace(text.to_owned());
        let mut pieces = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars().peekable();
        while let Some(char) = chars.next() {
            match char {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    literal.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    literal.push('}');
                }
                '}' => return Err(brace()),
                '{' => {
                    let mut name = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some(char) => name.push(char),
                            None => return Err(brace()),
                        }
                    }
                    if !literal.is_empty() {
                        pieces.push(Piece::Text(std::mem::take(&mut literal)));
                    }
                    pieces.push(Piece::Place(Placeholder::read(&name)?));
                }
                char => literal.push(char),
            }
        }
        if !literal.is_empty() {
            pieces.push(Piece::Text(literal));
        }
        Ok(Self { pieces })
    }
}

impl Placeholder {
    fn read(name: &str) -> Result<Self, TemplateError> {
        let unknown = || TemplateError::Placeholder(format!("{{{name}}}"));
        match name {
            "port" => Ok(Self::Port(0)),
            "data" => Ok(Self::Data),
            // `{workspace}`, its name before the rename, is read the same
            // for contracts written before it.
            "worktree" | "workspace" => Ok(Self::Worktree),
            other => other
                .strip_prefix("port+")
                .and_then(|offset| offset.parse().ok())
                .map(Self::Port)
                .ok_or_else(unknown),
        }
    }
}

impl Template {
    /// The text, for `place`.
    pub fn expand(&self, place: &Place) -> Result<String, TemplateError> {
        let path = |path: &std::path::Path| {
            path.to_str()
                .map(str::to_owned)
                .ok_or_else(|| TemplateError::Path(path.display().to_string()))
        };
        let mut text = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Text(literal) => text.push_str(literal),
                Piece::Place(Placeholder::Port(offset)) => {
                    let (first, last) = place.ports.ok_or(TemplateError::NoPorts)?;
                    let port = first
                        .checked_add(*offset)
                        .filter(|port| *port <= last)
                        .ok_or(TemplateError::PortRange(*offset))?;
                    text.push_str(&port.to_string());
                }
                Piece::Place(Placeholder::Data) => text.push_str(&path(&place.data)?),
                Piece::Place(Placeholder::Worktree) => text.push_str(&path(&place.worktree)?),
            }
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::{Template, TemplateError};
    use crate::Place;
    use std::path::PathBuf;

    #[test]
    fn placeholders_expand_and_braces_escape() {
        let place = Place {
            worktree: PathBuf::from("/w/acme-1"),
            data: PathBuf::from("/w/.data/acme-1"),
            ports: Some((20_000, 20_099)),
        };
        let expand = |text: &str| text.parse::<Template>().unwrap().expand(&place).unwrap();
        assert_eq!(expand("plain"), "plain");
        assert_eq!(expand("{port}:{port+2}"), "20000:20002");
        assert_eq!(
            expand("{worktree}/x and {data}"),
            "/w/acme-1/x and /w/.data/acme-1"
        );
        assert_eq!(expand("{workspace}/x"), expand("{worktree}/x"));
        assert_eq!(expand("{{\"a\": 1}}"), "{\"a\": 1}");
        for text in ["{home}", "{port+}", "{port-1}", "{port", "a}b"] {
            assert!(text.parse::<Template>().is_err(), "{text}");
        }
        assert_eq!(
            "{port+100}".parse::<Template>().unwrap().expand(&place),
            Err(TemplateError::PortRange(100))
        );
    }
}
