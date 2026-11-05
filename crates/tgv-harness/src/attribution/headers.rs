//! Attribution from request headers: the bounded, allowlisted observations a
//! harness's requests carry. These are hints, never authentication or proof of
//! client identity. Raw headers are not persisted.

use crate::Harness;
use crate::attribution::{Attribution, HintIssue, HintSource, SessionHint};
use http::HeaderMap;
use serde::Deserialize;
use tgv_protocol::native::{NativeSessionId, NativeThreadId, NativeTurnId};
use uuid::Uuid;

impl Harness {
    const PI_SESSION: &'static str = "x-tgv-pi-session-id";
    const PI_TURN: &'static str = "x-tgv-pi-turn-id";
    const MAX_TURN_METADATA: usize = 8192;

    /// What `headers` say about attribution, on a route that expects `route`'s
    /// headers (`None`: a route no harness owns, such as the public OpenAI API).
    /// Pi's extension headers are recognised on every route and take
    /// precedence, since Pi reaches TGV through the other harnesses' routes. A
    /// turn without a session carries no grouping; every discarded hint leaves
    /// an issue.
    pub fn attribution(route: Option<Self>, headers: &HeaderMap) -> Attribution {
        let mut reading = HeaderReading::new(headers);
        let claim = if headers.contains_key(Self::PI_SESSION) || headers.contains_key(Self::PI_TURN)
        {
            Claim {
                source: Some(HintSource::PiExtensionHeaders),
                session: reading.uuid(Self::PI_SESSION),
                turn: reading.uuid(Self::PI_TURN),
                thread: None,
            }
        } else {
            match route {
                // Public Responses headers do not establish a Codex client
                // identity, and Pi has no headers but its extension's.
                None | Some(Self::Pi) => Claim::default(),
                Some(Self::ClaudeCode) => {
                    let session = reading.uuid("x-claude-code-session-id");
                    let turn = reading.uuid("x-claude-code-prompt-id");
                    Claim {
                        source: (session.is_some() || turn.is_some())
                            .then_some(HintSource::ClaudeCodeHeaders),
                        session,
                        turn,
                        thread: None,
                    }
                }
                Some(Self::Codex) => reading.codex(),
            }
        };
        reading.finish(claim)
    }
}

/// Identifiers read so far, before a turn is checked against its session.
#[derive(Default)]
struct Claim {
    source: Option<HintSource>,
    session: Option<Uuid>,
    turn: Option<Uuid>,
    thread: Option<Uuid>,
}

/// One pass over a request's headers, collecting issues as it reads.
struct HeaderReading<'a> {
    headers: &'a HeaderMap,
    issues: Vec<HintIssue>,
}

impl<'a> HeaderReading<'a> {
    fn new(headers: &'a HeaderMap) -> Self {
        Self {
            headers,
            issues: Vec::new(),
        }
    }

    fn codex(&mut self) -> Claim {
        let mut session = self.uuid("session-id");
        let mut thread = self.uuid("thread-id");
        let mut turn = None;
        if let Some(raw) = self.header("x-codex-turn-metadata", Harness::MAX_TURN_METADATA) {
            match serde_json::from_str::<TurnMetadata>(raw) {
                Ok(metadata) => {
                    turn = self.metadata_id(metadata.turn_id);
                    let claimed_session = self.metadata_id(metadata.session_id);
                    let claimed_thread = self.metadata_id(metadata.thread_id);
                    // Do not silently prefer one source when identifiers disagree.
                    if Self::conflicts(session, claimed_session)
                        || Self::conflicts(thread, claimed_thread)
                    {
                        self.issue(HintIssue::ConflictingIdentifiers);
                        session = None;
                        thread = None;
                        turn = None;
                    }
                }
                Err(_) => self.issue(HintIssue::InvalidTurnMetadata),
            }
        }
        Claim {
            source: session.is_some().then_some(HintSource::CodexHeaders),
            session,
            turn,
            thread,
        }
    }

    fn finish(mut self, claim: Claim) -> Attribution {
        let session = claim.session.and_then(NativeSessionId::new);
        if session.is_none() && claim.turn.is_some() {
            self.issue(HintIssue::TurnWithoutSession);
        }
        Attribution {
            hint: claim
                .source
                .zip(session)
                .map(|(source, session)| SessionHint {
                    source,
                    session,
                    turn: claim.turn.and_then(NativeTurnId::new),
                    thread: claim.thread.and_then(NativeThreadId::new),
                }),
            issues: self.issues,
        }
    }

    fn issue(&mut self, issue: HintIssue) {
        if !self.issues.contains(&issue) {
            self.issues.push(issue);
        }
    }

    fn header(&mut self, name: &str, max: usize) -> Option<&'a str> {
        let mut values = self.headers.get_all(name).iter();
        let value = values.next()?;
        if values.next().is_some() || value.as_bytes().len() > max {
            self.issue(HintIssue::InvalidOrDuplicateHeader);
            return None;
        }
        match value.to_str() {
            Ok(value) => Some(value),
            Err(_) => {
                self.issue(HintIssue::InvalidOrDuplicateHeader);
                None
            }
        }
    }

    fn uuid(&mut self, name: &str) -> Option<Uuid> {
        let value = self.header(name, 36)?;
        let id = Self::identifier(value);
        if id.is_none() {
            self.issue(HintIssue::InvalidOrDuplicateHeader);
        }
        id
    }

    fn metadata_id(&mut self, value: Option<String>) -> Option<Uuid> {
        let id = Self::identifier(&value?);
        if id.is_none() {
            self.issue(HintIssue::InvalidTurnMetadata);
        }
        id
    }

    /// A hyphenated, non-nil UUID; nothing else is an identifier.
    fn identifier(value: &str) -> Option<Uuid> {
        if value.len() != 36 {
            return None;
        }
        Uuid::parse_str(value).ok().filter(|id| !id.is_nil())
    }

    fn conflicts(left: Option<Uuid>, right: Option<Uuid>) -> bool {
        matches!((left, right), (Some(a), Some(b)) if a != b)
    }
}

// Serde rejects duplicate known fields. Ignore all other fields: account,
// installation, machine, tool policy, and user-defined names never enter storage.
#[derive(Deserialize)]
struct TurnMetadata {
    turn_id: Option<String>,
    session_id: Option<String>,
    thread_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{Harness, HintIssue, HintSource};
    use crate::attribution::Attribution;
    use http::HeaderMap;
    use uuid::Uuid;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn header(id: Uuid) -> http::HeaderValue {
        id.to_string().parse().unwrap()
    }

    #[test]
    fn pi_hints_are_scoped_separately_and_invalid_hints_cannot_fall_back() {
        let (session, turn) = (id(1), id(2));
        for route in [
            Some(Harness::ClaudeCode),
            Some(Harness::Codex),
            Some(Harness::Pi),
            None,
        ] {
            let mut headers = HeaderMap::new();
            headers.insert("x-tgv-pi-session-id", header(session));
            headers.insert("x-tgv-pi-turn-id", header(turn));
            headers.insert("session-id", header(id(3)));
            let attribution = Harness::attribution(route, &headers);
            let hint = attribution.hint.unwrap();
            assert_eq!(hint.source, HintSource::PiExtensionHeaders);
            assert_eq!(hint.session.as_uuid(), session);
            assert_eq!(hint.turn.map(|t| t.as_uuid()), Some(turn));
            assert!(attribution.issues.is_empty());
            headers.append("x-tgv-pi-session-id", header(session));
            let attribution = Harness::attribution(route, &headers);
            assert!(attribution.hint.is_none());
            assert!(
                attribution
                    .issues
                    .contains(&HintIssue::InvalidOrDuplicateHeader)
            );
        }
    }

    #[test]
    fn extracts_only_ids_and_preserves_headers() {
        let (session, turn) = (id(1), id(2));
        let mut headers = HeaderMap::new();
        headers.insert("session-id", header(session));
        headers.insert("authorization", "Bearer SECRET".parse().unwrap());
        headers.insert(
            "x-codex-turn-metadata",
            format!(
                r#"{{"session_id":"{session}","turn_id":"{turn}","installation_id":"PRIVATE"}}"#
            )
            .parse()
            .unwrap(),
        );
        let original = headers.clone();
        let attribution = Harness::attribution(Some(Harness::Codex), &headers);
        let hint = attribution.hint.unwrap();
        assert_eq!(hint.source, HintSource::CodexHeaders);
        assert_eq!(hint.session.as_uuid(), session);
        assert_eq!(hint.turn.map(|t| t.as_uuid()), Some(turn));
        assert!(attribution.issues.is_empty());
        let json = serde_json::to_string(&attribution).unwrap();
        assert!(!json.contains("SECRET") && !json.contains("PRIVATE"));
        assert_eq!(headers, original);
    }

    #[test]
    fn ambiguity_and_invalid_values_cannot_create_groups() {
        let mut headers = HeaderMap::new();
        headers.append("x-claude-code-session-id", header(id(1)));
        headers.append("x-claude-code-session-id", header(id(1)));
        headers.insert("x-claude-code-prompt-id", header(id(1)));
        let attribution = Harness::attribution(Some(Harness::ClaudeCode), &headers);
        assert!(attribution.hint.is_none());
        assert!(attribution.issues.contains(&HintIssue::TurnWithoutSession));
        headers.clear();
        headers.insert("session-id", header(id(1)));
        headers.insert(
            "x-codex-turn-metadata",
            format!(r#"{{"session_id":"{}","turn_id":"{}"}}"#, id(2), id(1))
                .parse()
                .unwrap(),
        );
        let attribution = Harness::attribution(Some(Harness::Codex), &headers);
        assert!(attribution.hint.is_none());
        assert!(
            attribution
                .issues
                .contains(&HintIssue::ConflictingIdentifiers)
        );
        headers.insert("session-id", "TOKEN_OR_USER_NAME".parse().unwrap());
        let attribution = Harness::attribution(Some(Harness::Codex), &headers);
        assert!(attribution.hint.is_none());
        assert!(
            !serde_json::to_string(&attribution)
                .unwrap()
                .contains("TOKEN_OR_USER_NAME")
        );
    }

    #[test]
    fn ignores_cache_keys_and_request_ids_and_bounds_metadata() {
        let mut headers = HeaderMap::new();
        for (n, name) in ["x-client-request-id", "prompt-cache-key", "session_id"]
            .into_iter()
            .enumerate()
        {
            headers.insert(name, header(id(u128::try_from(n).unwrap() + 1)));
        }
        assert_eq!(
            Harness::attribution(Some(Harness::Codex), &headers),
            Attribution::default()
        );
        headers.insert("x-codex-turn-metadata", "x".repeat(8193).parse().unwrap());
        assert_eq!(
            Harness::attribution(Some(Harness::Codex), &headers).issues,
            [HintIssue::InvalidOrDuplicateHeader]
        );
    }

    #[test]
    fn a_nil_session_is_no_session() {
        let mut headers = HeaderMap::new();
        headers.insert("x-claude-code-session-id", header(Uuid::nil()));
        let attribution = Harness::attribution(Some(Harness::ClaudeCode), &headers);
        assert!(attribution.hint.is_none());
        assert_eq!(attribution.issues, [HintIssue::InvalidOrDuplicateHeader]);
    }
}
