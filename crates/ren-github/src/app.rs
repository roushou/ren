//! A GitHub App, and the tokens it mints for one repository at a time:
//! the [`App`] signs a short-lived token of its own (RS256) with its
//! private key, and an [`Installations`] client trades it for an
//! installation token scoped to one repository, with only the
//! permissions to push and open pull requests ([`RepositoryToken`]).

use crate::{
    client::{Endpoint, FetchError, Token},
    repository::RepositorySlug,
};
use aws_lc_rs::{rand::SystemRandom, signature};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use std::{
    fmt,
    time::{Duration, SystemTime},
};

/// A GitHub App's id, as its settings page shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppId(u64);

/// One installation of an App, on an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InstallationId(u64);

/// An App's private key: never shown by `Debug` or in an error.
pub struct AppKey(signature::RsaKeyPair);

/// Why text is not an [`AppKey`]. The text is not echoed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AppKeyError {
    #[error("not a PEM private key")]
    NotPem,
    #[error("not an RSA private key")]
    NotRsa,
}

/// A GitHub App: its id and its private key.
#[derive(Debug)]
pub struct App {
    id: AppId,
    key: AppKey,
}

/// A token an [`App`] signed for itself, good for a few minutes.
#[derive(Clone)]
pub struct AppToken(String);

/// Why an [`App`] could not sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the App's key could not sign")]
pub struct SignError;

/// An installation token scoped to one repository, and when it stops
/// working.
#[derive(Debug, Clone)]
pub struct RepositoryToken {
    pub token: Token,
    /// As GitHub wrote it: ISO 8601, in UTC.
    pub expires_at: String,
}

/// Mints installation tokens for an [`App`] from GitHub's REST API.
#[derive(Debug)]
pub struct Installations {
    http: reqwest::Client,
    api: Endpoint,
    app: App,
}

/// Why no token was minted.
#[derive(Debug, thiserror::Error)]
pub enum MintError {
    #[error(transparent)]
    Sign(#[from] SignError),
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("GitHub's answer did not read: {0}")]
    Unread(#[source] serde_json::Error),
    #[error("the token GitHub answered is not one")]
    NotAToken,
}

impl AppId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl InstallationId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl AppKey {
    /// The key in `pem`: PKCS #1 (`RSA PRIVATE KEY`, as GitHub hands it
    /// out) or PKCS #8 (`PRIVATE KEY`).
    pub fn from_pem(pem: &str) -> Result<Self, AppKeyError> {
        let (label, der) = Self::pem(pem).ok_or(AppKeyError::NotPem)?;
        let key = match label {
            "RSA PRIVATE KEY" => signature::RsaKeyPair::from_der(&der),
            "PRIVATE KEY" => signature::RsaKeyPair::from_pkcs8(&der),
            _ => return Err(AppKeyError::NotPem),
        };
        key.map(Self).map_err(|_| AppKeyError::NotRsa)
    }

    /// The label and bytes of the first PEM block in `text`.
    fn pem(text: &str) -> Option<(&str, Vec<u8>)> {
        let begin = text.find("-----BEGIN ")?;
        let rest = &text[begin + "-----BEGIN ".len()..];
        let (label, rest) = rest.split_once("-----")?;
        let end = format!("-----END {label}-----");
        let (body, _) = rest.split_once(end.as_str())?;
        let body: String = body.split_whitespace().collect();
        let der = base64::engine::general_purpose::STANDARD
            .decode(body)
            .ok()?;
        Some((label, der))
    }
}

impl fmt::Debug for AppKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AppKey(<redacted>)")
    }
}

impl App {
    /// How long a token the App signs lasts; GitHub takes at most ten
    /// minutes.
    const LIFETIME: Duration = Duration::from_secs(9 * 60);
    /// How far back a token's start is set, for a clock behind GitHub's.
    const SKEW: Duration = Duration::from_secs(60);

    pub fn new(id: AppId, key: AppKey) -> Self {
        Self { id, key }
    }

    pub fn id(&self) -> AppId {
        self.id
    }

    /// A token of the App's own, signed at `now`.
    pub fn sign(&self, now: SystemTime) -> Result<AppToken, SignError> {
        let seconds = |at: SystemTime| {
            at.duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |since| since.as_secs())
        };
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims = URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                "iat": seconds(now.checked_sub(Self::SKEW).unwrap_or(now)),
                "exp": seconds(now + Self::LIFETIME),
                "iss": self.id.0.to_string(),
            })
            .to_string(),
        );
        let signed = format!("{header}.{claims}");
        let mut signature = vec![0; self.key.0.public_modulus_len()];
        self.key
            .0
            .sign(
                &signature::RSA_PKCS1_SHA256,
                // Unused: PKCS #1 v1.5 signing takes no randomness.
                &SystemRandom::new(),
                signed.as_bytes(),
                &mut signature,
            )
            .map_err(|_| SignError)?;
        Ok(AppToken(format!(
            "{signed}.{}",
            URL_SAFE_NO_PAD.encode(signature)
        )))
    }
}

impl fmt::Debug for AppToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AppToken(<redacted>)")
    }
}

impl Installations {
    /// GitHub's own REST API.
    pub const GITHUB: &str = "https://api.github.com";
    /// The most of an answer read.
    const ANSWER_LIMIT: usize = 1024 * 1024;

    /// A client of the REST API at `api` for `app`.
    pub fn new(api: Endpoint, app: App) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("ren-github/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { http, api, app })
    }

    pub fn app(&self) -> &App {
        &self.app
    }

    /// A token, minted at `now`, that can push to `repository` and open
    /// its pull requests, and nothing else.
    pub async fn repository_token(
        &self,
        repository: &RepositorySlug,
        now: SystemTime,
    ) -> Result<RepositoryToken, MintError> {
        let app = self.app.sign(now)?;
        let installation = self.installation(&app, repository).await?;
        let body = serde_json::json!({
            "repositories": [repository.name()],
            "permissions": { "contents": "write", "pull_requests": "write" },
        });
        let answer = self
            .call(
                self.http
                    .post(self.url(&format!(
                        "app/installations/{}/access_tokens",
                        installation.0
                    )))
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body.to_string()),
                &app,
            )
            .await?;
        #[derive(serde::Deserialize)]
        struct Minted {
            token: String,
            expires_at: String,
        }
        let minted: Minted = serde_json::from_slice(&answer).map_err(MintError::Unread)?;
        Ok(RepositoryToken {
            token: Token::new(&minted.token).ok_or(MintError::NotAToken)?,
            expires_at: minted.expires_at,
        })
    }

    /// The installation of the App that covers `repository`.
    async fn installation(
        &self,
        app: &AppToken,
        repository: &RepositorySlug,
    ) -> Result<InstallationId, MintError> {
        let answer = self
            .call(
                self.http
                    .get(self.url(&format!("repos/{repository}/installation"))),
                app,
            )
            .await?;
        #[derive(serde::Deserialize)]
        struct Installation {
            id: u64,
        }
        let installation: Installation =
            serde_json::from_slice(&answer).map_err(MintError::Unread)?;
        Ok(InstallationId(installation.id))
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.api.as_str().trim_end_matches('/'))
    }

    async fn call(
        &self,
        request: reqwest::RequestBuilder,
        app: &AppToken,
    ) -> Result<Vec<u8>, FetchError> {
        let mut response = request
            .bearer_auth(&app.0)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
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
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::{App, AppId, AppKey, AppKeyError};
    use aws_lc_rs::{
        encoding::AsDer as _,
        rsa::KeySize,
        signature::{self, KeyPair as _},
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use std::time::{Duration, SystemTime};

    /// A key made for the test, as PKCS #8 PEM, and its public key.
    fn key() -> (String, Vec<u8>) {
        let pair = signature::RsaKeyPair::generate(KeySize::Rsa2048).unwrap();
        let der = pair.as_der().unwrap();
        let body = base64::engine::general_purpose::STANDARD.encode(der.as_ref());
        let pem = format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n");
        (pem, pair.public_key().as_ref().to_vec())
    }

    #[test]
    fn an_apps_token_is_signed_by_its_key_and_lasts_minutes() {
        let (pem, public) = key();
        let app = App::new(AppId::new(4242), AppKey::from_pem(&pem).unwrap());
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let token = app.sign(now).unwrap().0;
        let (signed, signature) = token.rsplit_once('.').unwrap();
        signature::UnparsedPublicKey::new(&signature::RSA_PKCS1_2048_8192_SHA256, &public)
            .verify(
                signed.as_bytes(),
                &URL_SAFE_NO_PAD.decode(signature).unwrap(),
            )
            .unwrap();
        let (header, claims) = signed.split_once('.').unwrap();
        assert_eq!(
            URL_SAFE_NO_PAD.decode(header).unwrap(),
            br#"{"alg":"RS256","typ":"JWT"}"#
        );
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).unwrap()).unwrap();
        assert_eq!(
            claims,
            serde_json::json!({ "iat": 1_799_999_940, "exp": 1_800_000_540, "iss": "4242" })
        );
    }

    #[test]
    fn a_key_is_read_from_pem_and_never_shown() {
        let (pem, _) = key();
        let key = AppKey::from_pem(&format!("issued by acme\n{pem}")).unwrap();
        assert_eq!(format!("{key:?}"), "AppKey(<redacted>)");
        assert_eq!(
            AppKey::from_pem("no key here").unwrap_err(),
            AppKeyError::NotPem
        );
        assert_eq!(
            AppKey::from_pem("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----")
                .unwrap_err(),
            AppKeyError::NotRsa
        );
    }
}
