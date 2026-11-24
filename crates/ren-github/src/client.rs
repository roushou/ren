//! The client: sends a [`Watch`]'s query to GitHub's GraphQL endpoint with
//! a [`Token`], and reads the answer into a [`Snapshot`].

use crate::{
    repository::RepositorySlug,
    watch::{ReadError, Snapshot, Watch},
};
use std::{fmt, net::IpAddr, time::Duration};

/// A GitHub token: sent only to the client's endpoint, and never shown by
/// `Debug` or in an error. One that can read pull requests, commit
/// statuses, and metadata is enough.
#[derive(Clone)]
pub struct Token(String);

/// One of GitHub's APIs (GraphQL, or REST's base), or a stand-in on loopback. HTTPS, or
/// plain HTTP to a loopback host only, so a token never travels in clear
/// text off the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint(reqwest::Url);

/// Why text is not an [`Endpoint`]. The text is not echoed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    #[error("not a URL")]
    NotAUrl,
    #[error("it carries credentials, a query, or a fragment")]
    Embedded,
    #[error("it needs HTTPS, except HTTP to a loopback host")]
    Insecure,
}

/// Reads repositories' pull requests from GitHub.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    endpoint: Endpoint,
    token: Token,
}

/// Why a read saw nothing.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("GitHub could not be reached: {0}")]
    Transport(#[source] reqwest::Error),
    #[error("GitHub answered {0}")]
    Status(reqwest::StatusCode),
    #[error("GitHub's answer is larger than {} bytes", Client::ANSWER_LIMIT)]
    TooLarge,
    #[error(transparent)]
    Read(#[from] ReadError),
}

impl Token {
    /// A token as text; none when blank or not printable ASCII.
    pub fn new(text: &str) -> Option<Self> {
        let text = text.trim();
        let valid = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_graphic());
        valid.then(|| Self(text.to_owned()))
    }
}

impl Token {
    /// The token's text, to hand to the one it is for (git, as a
    /// credential); never to show.
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

impl Endpoint {
    /// GitHub's own GraphQL endpoint.
    pub const GITHUB: &str = "https://api.github.com/graphql";

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn parse(text: &str) -> Result<Self, EndpointError> {
        let url = reqwest::Url::parse(text).map_err(|_| EndpointError::NotAUrl)?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(EndpointError::Embedded);
        }
        let loopback = url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        let secure = url.scheme() == "https" || (url.scheme() == "http" && loopback);
        if url.host_str().is_some() && secure {
            Ok(Self(url))
        } else {
            Err(EndpointError::Insecure)
        }
    }
}

impl std::str::FromStr for Endpoint {
    type Err = EndpointError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Client {
    /// The most of an answer read: 50 pull requests with 50 checks each
    /// and their descriptions fit well within it.
    const ANSWER_LIMIT: usize = 16 * 1024 * 1024;

    /// A client of `endpoint` with `token`.
    pub fn new(endpoint: Endpoint, token: Token) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("ren-github/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            http,
            endpoint,
            token,
        })
    }

    /// `repository`'s default branch and open pull requests, now.
    pub async fn snapshot(&self, repository: &RepositorySlug) -> Result<Snapshot, FetchError> {
        let watch = Watch::new(repository.clone());
        let mut response = self
            .http
            .post(self.endpoint.0.clone())
            .bearer_auth(&self.token.0)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(watch.query())
            .send()
            .await
            .map_err(FetchError::Transport)?;
        let status = response.status();
        if !status.is_success() {
            return Err(FetchError::Status(status));
        }
        let mut answer = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(FetchError::Transport)? {
            if answer.len().saturating_add(chunk.len()) > Self::ANSWER_LIMIT {
                return Err(FetchError::TooLarge);
            }
            answer.extend_from_slice(&chunk);
        }
        Ok(watch.read(&answer)?)
    }
}

#[cfg(test)]
mod tests {
    use super::{Endpoint, EndpointError, Token};

    #[test]
    fn an_endpoint_is_https_or_loopback() {
        assert!(Endpoint::parse("https://api.github.com/graphql").is_ok());
        assert!(Endpoint::parse("http://127.0.0.1:8080/graphql").is_ok());
        assert_eq!(
            Endpoint::parse("http://github.example/graphql"),
            Err(EndpointError::Insecure)
        );
        assert_eq!(
            Endpoint::parse("https://user:secret@github.example/graphql"),
            Err(EndpointError::Embedded)
        );
        assert_eq!(
            Endpoint::parse(Endpoint::GITHUB).unwrap().0.as_str(),
            Endpoint::GITHUB
        );
    }

    #[test]
    fn a_token_is_never_shown() {
        let token = Token::new(" github_pat_SECRET \n").unwrap();
        assert!(!format!("{token:?}").contains("SECRET"));
        assert!(Token::new("  ").is_none());
    }
}
