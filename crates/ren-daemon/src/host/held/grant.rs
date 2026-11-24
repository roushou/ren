//! Grants: a relayed run pushes to its repository with a token scoped to
//! that repository alone, minted outside the run by the GitHub App and
//! handed to git when it asks. Inside the run, git's credential helper
//! (`ren credential`) passes git's ask, with the run's proxy URL, to the
//! grants socket; [`Grants`] answers a run whose ticket was granted its
//! repository, and nothing else.

use ren_github::{Installations, MintError, RepositorySlug, Token};
use ren_recorder::time::{Clock, UnixMillis};
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, SystemTime},
};

/// What git asks a credential helper for, read once from git's credential
/// format (`key=value` lines) with the run's proxy URL the helper adds: the
/// run's ticket, and what git wants a credential for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CredentialAsk {
    /// From the run's proxy URL, whose password it is.
    ticket: Option<ren_relay::Ticket>,
    protocol: Option<Protocol>,
    host: Option<GitHost>,
    path: AskedPath,
}

/// The protocol git asks a credential for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Protocol {
    Https,
    Other(String),
}

/// The host git asks a credential for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GitHost {
    GitHub,
    Other(String),
}

/// The repository's path git sends with its ask (`credential.useHttpPath`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum AskedPath {
    #[default]
    Unsaid,
    Repository(RepositorySlug),
    Other(String),
}

/// The GitHub App's tokens for the repositories runs may be granted.
pub struct Grants {
    installations: Arc<Installations>,
    repositories: BTreeSet<RepositorySlug>,
    clock: Arc<dyn Clock>,
    minted: Mutex<HashMap<RepositorySlug, Minted>>,
}

#[derive(Clone)]
struct Minted {
    token: Token,
    at: UnixMillis,
}

impl CredentialAsk {
    /// The most of an ask read.
    pub const LIMIT: usize = 8 * 1024;

    /// The ask in `text`: git's lines up to a blank one. A key it does
    /// not take is left out; a value is never echoed.
    pub fn read(text: &str) -> Self {
        let mut ask = Self::default();
        for line in text.lines() {
            if line.is_empty() {
                break;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "proxy" => ask.ticket = Self::ticket_of(value),
                "protocol" => {
                    ask.protocol = Some(match value {
                        "https" => Protocol::Https,
                        other => Protocol::Other(other.to_owned()),
                    });
                }
                "host" => {
                    ask.host = Some(match value {
                        "github.com" => GitHost::GitHub,
                        other => GitHost::Other(other.to_owned()),
                    });
                }
                "path" => {
                    ask.path =
                        match RepositorySlug::of_remote(&format!("https://github.com/{value}")) {
                            Some(repository) => AskedPath::Repository(repository),
                            None => AskedPath::Other(value.to_owned()),
                        };
                }
                _ => {}
            }
        }
        ask
    }

    /// The ticket in a proxy URL, `http://<user>:<ticket>@<relay>`.
    fn ticket_of(proxy: &str) -> Option<ren_relay::Ticket> {
        let (credentials, _) = proxy.rsplit_once('@')?;
        let (_, secret) = credentials.rsplit_once(':')?;
        ren_relay::Ticket::new(secret).ok()
    }

    /// The run's ticket, from its proxy URL.
    pub fn ticket(&self) -> Option<&ren_relay::Ticket> {
        self.ticket.as_ref()
    }

    /// Whether it asks for `repository` on github.com over HTTPS: its path,
    /// when git sends one, must name it.
    pub fn is_for(&self, repository: &RepositorySlug) -> bool {
        let https = matches!(self.protocol, Some(Protocol::Https));
        let github = matches!(self.host, Some(GitHost::GitHub));
        let path = match &self.path {
            AskedPath::Unsaid => true,
            AskedPath::Repository(asked) => asked == repository,
            AskedPath::Other(_) => false,
        };
        https && github && path
    }

    /// Git's answer granting `token`: the user GitHub expects with an
    /// installation token.
    pub fn answer(token: &Token) -> String {
        format!("username=x-access-token\npassword={}\n\n", token.reveal())
    }
}

impl Grants {
    /// The variable a run's git helper finds the grants socket in.
    pub const SOCKET_VARIABLE: &str = "REN_GRANTS";
    /// How long a minted token is handed out: GitHub's last an hour, and
    /// a push that starts with one must finish before it ends.
    const REUSE: Duration = Duration::from_secs(45 * 60);

    pub fn new(
        installations: Arc<Installations>,
        repositories: impl IntoIterator<Item = RepositorySlug>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            installations,
            repositories: repositories.into_iter().collect(),
            clock,
            minted: Mutex::new(HashMap::new()),
        }
    }

    /// Whether a run working in `repository` is granted it: the operator
    /// listed it under `[[github.repository]]`.
    pub fn covers(&self, repository: &RepositorySlug) -> bool {
        self.repositories.contains(repository)
    }

    /// A token for `repository`: the one minted last while it is fresh,
    /// else a new one.
    pub async fn token(&self, repository: &RepositorySlug) -> Result<Token, MintError> {
        let now = self.clock.now().wall;
        let fresh = |minted: &Minted| {
            now.as_millis().saturating_sub(minted.at.as_millis())
                < u64::try_from(Self::REUSE.as_millis()).unwrap_or(u64::MAX)
        };
        if let Some(minted) = self
            .minted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(repository)
            .filter(|minted| fresh(minted))
        {
            return Ok(minted.token.clone());
        }
        let at = SystemTime::UNIX_EPOCH + Duration::from_millis(now.as_millis());
        let minted = self
            .installations
            .repository_token(repository, at)
            .await?
            .token;
        self.minted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                repository.clone(),
                Minted {
                    token: minted.clone(),
                    at: now,
                },
            );
        Ok(minted)
    }
}

impl std::fmt::Debug for Grants {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grants")
            .field("repositories", &self.repositories)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::CredentialAsk;

    fn ask(text: &str) -> CredentialAsk {
        CredentialAsk::read(text)
    }

    #[test]
    fn an_ask_is_read_from_gits_lines_and_its_ticket_from_the_proxy() {
        let read = ask(
            "proxy=http://run:abcdef0123456789abcdef0123456789@127.0.0.1:3128\nprotocol=https\nhost=github.com\npath=acme/web.git\nwwwauth[]=Basic\n\nprotocol=ignored\n",
        );
        assert!(read.ticket().is_some());
        assert!(read.is_for(&"acme/web".parse().unwrap()));
        // Read up to the blank line: what follows is not the ask.
        assert!(!format!("{read:?}").contains("ignored"));
        // The ticket is never shown.
        assert!(!format!("{read:?}").contains("abcdef"));
        assert!(ask("protocol=https\n").ticket().is_none());
    }

    #[test]
    fn an_ask_is_for_its_repository_on_github_over_https_only() {
        let web = "acme/web".parse().unwrap();
        assert!(ask("protocol=https\nhost=github.com\npath=acme/web.git\n").is_for(&web));
        assert!(ask("protocol=https\nhost=github.com\npath=acme/web\n").is_for(&web));
        assert!(ask("protocol=https\nhost=github.com\n").is_for(&web));
        assert!(!ask("protocol=https\nhost=github.com\npath=acme/other.git\n").is_for(&web));
        assert!(!ask("protocol=http\nhost=github.com\n").is_for(&web));
        assert!(!ask("protocol=https\nhost=gitlab.com\npath=acme/web.git\n").is_for(&web));
    }
}
