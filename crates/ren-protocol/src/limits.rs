//! The response headers a provider sends about its limits, picked from a
//! response head by name. They are the raw evidence for a subscription's
//! allowance (Anthropic's `anthropic-ratelimit-*`, Codex's
//! `x-codex-primary-*` and `x-codex-secondary-*`, the OpenAI API's
//! `x-ratelimit-*`, and `retry-after`), kept as sent so their names and
//! values can be read before anything is built on them. Allowance is not
//! token usage, and neither is cost.
//!
//! Only names on the allowlist are kept, and a value is kept only when it
//! is plain: short, and made of the characters numbers, timestamps,
//! durations, and status words use. Any other value is withheld, its name
//! kept, so a header that would carry an identifier or a token is never
//! recorded.

use http::HeaderMap;
use serde::{Deserialize, Serialize};

/// The limit headers of one response head, in the order they came.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LimitHeaders(Vec<LimitHeader>);

/// One limit header: its name, lowercase as HTTP/2 sends it, and its value
/// when it is plain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitHeader {
    name: String,
    /// `None`: withheld, because it was not plain.
    value: Option<String>,
}

impl LimitHeaders {
    /// A head with no limit headers.
    pub const NONE: Self = Self(Vec::new());
    /// The most headers one response head keeps; the rest are left out.
    pub const MAX: usize = 32;
    /// The longest value kept; a longer one is withheld.
    pub const MAX_VALUE_BYTES: usize = 64;

    /// Name prefixes of limit headers.
    const PREFIXES: [&'static str; 5] = [
        "anthropic-ratelimit-",
        "x-ratelimit-",
        "x-codex-primary-",
        "x-codex-secondary-",
        "x-codex-credits-",
    ];
    /// Limit headers named whole.
    const NAMES: [&'static str; 1] = ["retry-after"];

    /// The limit headers among `headers`.
    pub fn from_headers(headers: &HeaderMap) -> Self {
        Self(
            headers
                .iter()
                .filter(|(name, _)| Self::is_limit(name.as_str()))
                .take(Self::MAX)
                .map(|(name, value)| LimitHeader {
                    name: name.as_str().to_owned(),
                    value: value
                        .to_str()
                        .ok()
                        .filter(|value| Self::is_plain(value))
                        .map(str::to_owned),
                })
                .collect(),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &LimitHeader> {
        self.0.iter()
    }

    /// The same headers, with every value `keep` refuses withheld.
    #[must_use]
    pub fn withholding(self, keep: impl Fn(&str) -> bool) -> Self {
        Self(
            self.0
                .into_iter()
                .map(|header| LimitHeader {
                    value: header.value.filter(|value| keep(value)),
                    ..header
                })
                .collect(),
        )
    }

    fn is_limit(name: &str) -> bool {
        Self::NAMES.contains(&name) || Self::PREFIXES.iter().any(|prefix| name.starts_with(prefix))
    }

    /// Short, and only what numbers, timestamps, durations, and status
    /// words are written with.
    fn is_plain(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= Self::MAX_VALUE_BYTES
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b" .:,_+-".contains(&byte))
    }
}

impl LimitHeader {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The value as sent; `None` when it was withheld.
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::LimitHeaders;
    use http::{HeaderMap, HeaderName, HeaderValue};

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        pairs
            .iter()
            .map(|(name, value)| {
                (
                    HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    HeaderValue::from_str(value).unwrap(),
                )
            })
            .collect()
    }

    fn pairs(limits: &LimitHeaders) -> Vec<(&str, Option<&str>)> {
        limits
            .iter()
            .map(|header| (header.name(), header.value()))
            .collect()
    }

    #[test]
    fn only_limit_headers_are_kept_with_their_values() {
        let limits = LimitHeaders::from_headers(&headers(&[
            ("content-type", "text/event-stream"),
            ("anthropic-ratelimit-unified-5h-utilization", "0.42"),
            ("anthropic-ratelimit-unified-5h-reset", "1795140000"),
            ("anthropic-ratelimit-unified-status", "allowed"),
            ("x-codex-primary-used-percent", "62.5"),
            ("x-codex-secondary-reset-after-seconds", "86400"),
            ("x-codex-turn-state", "opaque"),
            ("x-ratelimit-remaining-tokens", "39000"),
            ("x-ratelimit-reset-requests", "1s"),
            ("retry-after", "30"),
            ("request-id", "req_011CXyz"),
            ("set-cookie", "session=abc"),
        ]));
        assert_eq!(
            pairs(&limits),
            [
                ("anthropic-ratelimit-unified-5h-utilization", Some("0.42")),
                ("anthropic-ratelimit-unified-5h-reset", Some("1795140000")),
                ("anthropic-ratelimit-unified-status", Some("allowed")),
                ("x-codex-primary-used-percent", Some("62.5")),
                ("x-codex-secondary-reset-after-seconds", Some("86400")),
                ("x-ratelimit-remaining-tokens", Some("39000")),
                ("x-ratelimit-reset-requests", Some("1s")),
                ("retry-after", Some("30")),
            ]
        );
    }

    #[test]
    fn a_value_that_is_not_plain_is_withheld_and_its_name_kept() {
        let long = "1".repeat(LimitHeaders::MAX_VALUE_BYTES + 1);
        let limits = LimitHeaders::from_headers(&headers(&[
            ("anthropic-ratelimit-unified-reset", "2026-11-20T14:00:00Z"),
            ("anthropic-ratelimit-unified-fallback", "a/b=c"),
            ("x-ratelimit-limit-tokens", &long),
            ("retry-after", ""),
        ]));
        assert_eq!(
            pairs(&limits),
            [
                (
                    "anthropic-ratelimit-unified-reset",
                    Some("2026-11-20T14:00:00Z")
                ),
                ("anthropic-ratelimit-unified-fallback", None),
                ("x-ratelimit-limit-tokens", None),
                ("retry-after", None),
            ]
        );
        let withheld = limits.withholding(|value| !value.starts_with("2026"));
        assert_eq!(withheld.iter().next().unwrap().value(), None);
    }

    #[test]
    fn at_most_max_headers_are_kept() {
        let names: Vec<String> = (0..LimitHeaders::MAX + 5)
            .map(|index| format!("x-ratelimit-test-{index}"))
            .collect();
        let many: Vec<(&str, &str)> = names.iter().map(|name| (name.as_str(), "1")).collect();
        assert_eq!(
            LimitHeaders::from_headers(&headers(&many)).iter().count(),
            LimitHeaders::MAX
        );
        assert!(LimitHeaders::from_headers(&HeaderMap::new()).is_empty());
    }

    #[test]
    fn they_store_as_names_and_values_and_read_back() {
        let limits = LimitHeaders::from_headers(&headers(&[
            ("anthropic-ratelimit-unified-status", "allowed"),
            ("anthropic-ratelimit-unified-fallback", "a/b"),
        ]));
        let json = serde_json::to_string(&limits).unwrap();
        assert_eq!(
            json,
            r#"[{"name":"anthropic-ratelimit-unified-status","value":"allowed"},{"name":"anthropic-ratelimit-unified-fallback","value":null}]"#
        );
        assert_eq!(serde_json::from_str::<LimitHeaders>(&json).unwrap(), limits);
    }
}
