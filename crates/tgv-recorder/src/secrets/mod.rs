//! Credentials in what calls carry: well-known formats only, found and
//! redacted without keeping them.
//!
//! The scanner looks for fixed, documented token formats (provider keys,
//! GitHub and Slack tokens, cloud keys, private key blocks, JWTs), not for
//! text that merely looks random, so a finding is rarely a false alarm. A
//! found secret is never stored: an exposure keeps its kind, a masked hint
//! (its known prefix and last four characters), and a short fingerprint
//! that tells repeats of one secret apart from different ones.

use regex::Regex;
use serde::Serialize;
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::ops::Range;
use strum::{Display, EnumString, IntoStaticStr, VariantArray};
use tgv_protocol::label::Labeled;

/// A credential format.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum SecretKind {
    /// `sk-ant-…`
    AnthropicKey,
    /// `sk-…`, `sk-proj-…`
    OpenaiKey,
    /// `ghp_…`, `gho_…`, `ghu_…`, `ghs_…`, `ghr_…`, `github_pat_…`
    GithubToken,
    /// `AKIA…`, `ASIA…`
    AwsAccessKey,
    /// `AIza…`
    GoogleApiKey,
    /// `xoxb-…` and the other Slack token types
    SlackToken,
    /// `sk_live_…`, `rk_live_…`
    StripeLiveKey,
    /// A PEM `PRIVATE KEY` block.
    PrivateKey,
    /// A JSON Web Token: three base64url parts, the first two JSON. Often a
    /// short-lived session token rather than a long-lived key.
    Jwt,
}

/// One secret found in a text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Detection {
    pub kind: SecretKind,
    /// The known prefix and the last four characters: `sk-ant-…a1b2`.
    pub hint: String,
    /// 16 hex characters of the secret's BLAKE3 hash: equal for the same
    /// secret, and never enough to recover it.
    pub fingerprint: String,
}

/// Finds and redacts credentials. Built once; matching is linear in the
/// text's length.
pub struct SecretScanner {
    patterns: Vec<(Pattern, Regex)>,
}

/// Why the scanner could not be built: its patterns are constants, so this
/// is a defect, not an input problem.
#[derive(Debug, thiserror::Error)]
#[error("a secret pattern does not compile: {0}")]
pub struct ScannerError(#[from] regex::Error);

/// A pattern and how its matches are named.
#[derive(Debug, Clone, Copy)]
enum Pattern {
    /// `sk-…`: Anthropic when it continues `ant-`, else OpenAI.
    SkKey,
    Kind(SecretKind),
}

impl SecretScanner {
    const SOURCES: [(Pattern, &str); 8] = [
        (Pattern::SkKey, r"sk-[A-Za-z0-9_-]{20,}"),
        (
            Pattern::Kind(SecretKind::GithubToken),
            r"gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,}",
        ),
        (
            Pattern::Kind(SecretKind::AwsAccessKey),
            r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
        ),
        (
            Pattern::Kind(SecretKind::GoogleApiKey),
            r"AIza[0-9A-Za-z_-]{35}",
        ),
        (
            Pattern::Kind(SecretKind::SlackToken),
            r"xox[abposr]-[A-Za-z0-9-]{10,}",
        ),
        (
            Pattern::Kind(SecretKind::StripeLiveKey),
            r"[sr]k_live_[A-Za-z0-9]{20,}",
        ),
        (
            // The block, or its header alone when the text stops before
            // the end marker. Matching stays linear in the text's length.
            Pattern::Kind(SecretKind::PrivateKey),
            r"-----BEGIN (?:[A-Z]+ )*PRIVATE KEY-----(?:[\s\S]*?-----END (?:[A-Z]+ )*PRIVATE KEY-----)?",
        ),
        (
            Pattern::Kind(SecretKind::Jwt),
            r"eyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
        ),
    ];

    pub fn new() -> Result<Self, ScannerError> {
        Ok(Self {
            patterns: Self::SOURCES
                .iter()
                .map(|(pattern, source)| Ok((*pattern, Regex::new(source)?)))
                .collect::<Result<_, ScannerError>>()?,
        })
    }

    /// Every secret in `text`, in order and never overlapping: where two
    /// formats overlap, the one starting first (then the longer) wins.
    pub fn find(&self, text: &str) -> Vec<(Range<usize>, SecretKind)> {
        let mut found: Vec<(Range<usize>, SecretKind)> = self
            .patterns
            .iter()
            .flat_map(|(pattern, regex)| {
                regex.find_iter(text).map(move |found| {
                    let kind = match pattern {
                        Pattern::SkKey if found.as_str().starts_with("sk-ant-") => {
                            SecretKind::AnthropicKey
                        }
                        Pattern::SkKey => SecretKind::OpenaiKey,
                        Pattern::Kind(kind) => *kind,
                    };
                    (found.range(), kind)
                })
            })
            .collect();
        found.sort_by_key(|(range, _)| (range.start, std::cmp::Reverse(range.end)));
        let mut kept: Vec<(Range<usize>, SecretKind)> = Vec::with_capacity(found.len());
        for (range, kind) in found {
            if kept.last().is_none_or(|(last, _)| range.start >= last.end) {
                kept.push((range, kind));
            }
        }
        kept
    }

    /// What `text` carries, without redacting it.
    pub fn detect(&self, text: &str) -> Vec<Detection> {
        self.find(text)
            .into_iter()
            .map(|(range, kind)| Self::detection(kind, text.get(range).unwrap_or_default()))
            .collect()
    }

    /// `text` with each secret replaced by `«redacted: kind»`, and what was
    /// found.
    pub fn redact(&self, text: &str) -> (String, Vec<Detection>) {
        let found = self.find(text);
        if found.is_empty() {
            return (text.to_owned(), Vec::new());
        }
        let mut redacted = String::with_capacity(text.len());
        let mut detections = Vec::with_capacity(found.len());
        let mut next = 0;
        for (range, kind) in found {
            let secret = text.get(range.clone()).unwrap_or_default();
            redacted.push_str(text.get(next..range.start).unwrap_or_default());
            redacted.push_str(&format!("«redacted: {}»", kind.label()));
            next = range.end;
            detections.push(Self::detection(kind, secret));
        }
        redacted.push_str(text.get(next..).unwrap_or_default());
        (redacted, detections)
    }

    fn detection(kind: SecretKind, secret: &str) -> Detection {
        Detection {
            kind,
            hint: Self::hint(kind, secret),
            fingerprint: Self::fingerprint(secret),
        }
    }

    /// The format's known prefix and the last four characters.
    fn hint(kind: SecretKind, secret: &str) -> String {
        let prefix = match kind {
            SecretKind::AnthropicKey => "sk-ant-",
            SecretKind::OpenaiKey => {
                if secret.starts_with("sk-proj-") {
                    "sk-proj-"
                } else {
                    "sk-"
                }
            }
            SecretKind::GithubToken => {
                if secret.starts_with("github_pat_") {
                    "github_pat_"
                } else {
                    secret.get(..4).unwrap_or_default()
                }
            }
            SecretKind::AwsAccessKey | SecretKind::GoogleApiKey => {
                secret.get(..4).unwrap_or_default()
            }
            SecretKind::SlackToken => secret.get(..5).unwrap_or_default(),
            SecretKind::StripeLiveKey => secret.get(..8).unwrap_or_default(),
            // The block's content has no useful tail to show.
            SecretKind::PrivateKey => return "-----BEGIN … PRIVATE KEY-----".to_owned(),
            SecretKind::Jwt => "eyJ",
        };
        let tail: String = secret
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{prefix}…{tail}")
    }

    fn fingerprint(secret: &str) -> String {
        blake3::hash(secret.as_bytes())
            .to_hex()
            .chars()
            .take(16)
            .collect()
    }
}

#[cfg(test)]
mod tests;
