//! A repository on GitHub, by its owner and name.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{fmt, str::FromStr};

/// A repository on GitHub, written `owner/name` (`acme/web`). Each part is
/// 1 to 100 ASCII letters, digits, `-`, `_`, or `.`, and neither is `.` or
/// `..`, as GitHub allows.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, SerializeDisplay, DeserializeFromStr,
)]
pub struct RepositorySlug {
    owner: String,
    name: String,
}

/// Why text is not a [`RepositorySlug`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a GitHub repository is `owner/name`, each of letters, digits, `-`, `_`, or `.`: {0:?}")]
pub struct SlugError(String);

impl RepositorySlug {
    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The repository a git remote URL names on github.com: `https://`,
    /// `ssh://git@`, or `git@github.com:`, with or without `.git`. None for
    /// another host or a URL that names no repository.
    pub fn of_remote(url: &str) -> Option<Self> {
        let path = url
            .strip_prefix("https://github.com/")
            .or_else(|| url.strip_prefix("ssh://git@github.com/"))
            .or_else(|| url.strip_prefix("git@github.com:"))?;
        let path = path.trim_end_matches('/');
        path.strip_suffix(".git").unwrap_or(path).parse().ok()
    }

    fn part(text: &str) -> bool {
        (1..=100).contains(&text.len())
            && text != "."
            && text != ".."
            && text
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    }
}

impl FromStr for RepositorySlug {
    type Err = SlugError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.split_once('/') {
            Some((owner, name)) if Self::part(owner) && Self::part(name) => Ok(Self {
                owner: owner.to_owned(),
                name: name.to_owned(),
            }),
            Some(_) | None => Err(SlugError(text.to_owned())),
        }
    }
}

impl fmt::Display for RepositorySlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_github_remote_names_its_repository() {
        use super::RepositorySlug;
        for url in [
            "https://github.com/acme/web.git",
            "https://github.com/acme/web",
            "git@github.com:acme/web.git",
            "ssh://git@github.com/acme/web.git",
        ] {
            assert_eq!(
                RepositorySlug::of_remote(url),
                Some("acme/web".parse().unwrap()),
                "{url}"
            );
        }
        for url in [
            "https://gitlab.com/acme/web.git",
            "https://github.com/acme",
            "https://github.com/acme/web/tree/main",
            "/srv/git/web.git",
        ] {
            assert_eq!(RepositorySlug::of_remote(url), None, "{url}");
        }
    }

    use super::RepositorySlug;

    #[test]
    fn a_slug_is_owner_and_name() {
        let slug: RepositorySlug = "acme/web.site".parse().unwrap();
        assert_eq!((slug.owner(), slug.name()), ("acme", "web.site"));
        assert_eq!(slug.to_string(), "acme/web.site");
    }

    #[test]
    fn other_text_is_refused() {
        for text in [
            "",
            "acme",
            "acme/",
            "/web",
            "acme/web/x",
            "acme/..",
            "ac me/web",
        ] {
            assert!(text.parse::<RepositorySlug>().is_err(), "{text:?}");
        }
    }
}
