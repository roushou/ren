use super::{
    Admission, AuthKind, CallFacts, EntryUsers, Policy, PolicyConfig, PolicyError, Rejection,
    RequestJson, StepOutcome, UpstreamSummary,
};
use crate::{
    operator::{OperatorCommand, OperatorState, RetryAfter},
    retry::{Delivery, RetryDecision, WireFact},
};
use http::HeaderMap;
use ren_protocol::{
    native::NativeSessionId,
    route::{Endpoint, Provider, Transport},
};
use ren_recorder::{caller::UserId, routing::UpstreamId, time::UnixMillis};
use uuid::Uuid;

fn upstreams() -> Vec<UpstreamSummary> {
    let mut entries: Vec<UpstreamSummary> =
        [Provider::Anthropic, Provider::Codex, Provider::OpenAi]
            .into_iter()
            .map(|provider| UpstreamSummary {
                id: UpstreamId::default_for(provider),
                provider,
                auth: AuthKind::ClientOwned,
                users: EntryUsers::Anyone,
            })
            .collect();
    entries.push(UpstreamSummary {
        id: UpstreamId::parse("anthropic-key").unwrap(),
        provider: Provider::Anthropic,
        auth: AuthKind::OperatorKey,
        users: EntryUsers::Anyone,
    });
    entries
}

fn policy(toml: &str) -> Result<Policy, PolicyError> {
    Policy::new(toml::from_str::<PolicyConfig>(toml).unwrap(), &upstreams())
}

fn facts(provider: Provider, model: Option<&str>) -> CallFacts<'_> {
    CallFacts {
        transport: Transport::Http,
        provider,
        // Each test's calls go first to their provider's native entry.
        upstream: Box::leak(Box::new(UpstreamId::default_for(provider))),
        endpoint: Endpoint::Messages,
        model,
        session: None,
        user: None,
        now: UnixMillis::from_millis(1_000),
    }
}

const RULES: &str = r#"
[[block]]
id = "no-opus"
match = { provider = "anthropic", model = "fixture-opus" }
reason = "too expensive"

[[transform]]
id = "drop-temperature"
match = { endpoint = "messages" }
steps = [
  { op = "json_remove", path = "/temperature" },
  { op = "set_header", name = "x-fixture", value = "1" },
]
reason = "fixture"

[[fallback]]
id = "plan-limit"
provider = "anthropic"
to = "anthropic-key"
on_status = [429]
"#;

#[test]
fn plans_admit_block_transform_and_fall_over() {
    let policy = policy(RULES).unwrap();
    let operator = OperatorState::default();

    let plan = policy.plan(
        &facts(Provider::Anthropic, Some("fixture-sonnet")),
        &operator,
    );
    assert_eq!(plan.admission, Admission::Admit);
    assert_eq!(plan.candidates.len(), 2);
    assert_eq!(plan.candidates.first().as_str(), "anthropic");
    assert_eq!(plan.transforms.len(), 1);
    assert_eq!(
        plan.retry
            .eligible(WireFact::Responded { status: 429 }, Delivery::Nothing),
        RetryDecision::NextCandidate
    );

    let blocked = policy.plan(&facts(Provider::Anthropic, Some("fixture-opus")), &operator);
    assert!(matches!(
        blocked.admission,
        Admission::Reject(Rejection::Blocked { ref rule, .. }) if rule.as_str() == "no-opus"
    ));

    // Other providers have no fallback; WebSocket calls never do.
    assert_eq!(
        policy
            .plan(&facts(Provider::OpenAi, None), &operator)
            .candidates
            .len(),
        1
    );
    let websocket = CallFacts {
        transport: Transport::WebSocket,
        ..facts(Provider::Anthropic, None)
    };
    let plan = policy.plan(&websocket, &operator);
    assert_eq!((plan.candidates.len(), plan.transforms.len()), (1, 0));
}

#[test]
fn a_call_sent_first_to_an_operator_key_entry_never_falls_over() {
    let policy = policy(RULES).unwrap();
    let operator = OperatorState::default();
    let direct = UpstreamId::parse("anthropic-key").unwrap();
    let call = CallFacts {
        upstream: &direct,
        ..facts(Provider::Anthropic, Some("fixture-sonnet"))
    };
    let plan = policy.plan(&call, &operator);
    assert_eq!(plan.candidates.len(), 1);
    assert_eq!(plan.candidates.first(), &direct);
    assert_eq!(plan.fallback, None);
    // Block and transform rules still apply to it.
    assert_eq!(plan.transforms.len(), 1);
}

#[test]
fn an_entry_for_some_users_refuses_others_and_never_takes_their_fallback() {
    let alice = UserId::parse("alice").unwrap();
    let bob = UserId::parse("bob").unwrap();
    let mut entries = upstreams();
    for entry in &mut entries {
        if entry.auth == AuthKind::OperatorKey {
            entry.users = EntryUsers::Only(vec![alice.clone()]);
        }
    }
    let policy = Policy::new(toml::from_str(RULES).unwrap(), &entries).unwrap();
    let operator = OperatorState::default();
    let key = UpstreamId::parse("anthropic-key").unwrap();
    for (user, admitted) in [(Some(&alice), true), (Some(&bob), false), (None, false)] {
        let direct = CallFacts {
            upstream: &key,
            user,
            ..facts(Provider::Anthropic, Some("fixture-sonnet"))
        };
        let plan = policy.plan(&direct, &operator);
        if admitted {
            assert_eq!(plan.admission, Admission::Admit, "{user:?}");
        } else {
            assert_eq!(
                plan.admission,
                Admission::Reject(Rejection::UserNotAllowed {
                    upstream: key.clone()
                }),
                "{user:?}"
            );
        }
        // A fallback to the entry is offered only to its users.
        let native = CallFacts {
            user,
            ..facts(Provider::Anthropic, Some("fixture-sonnet"))
        };
        let plan = policy.plan(&native, &operator);
        assert_eq!(plan.admission, Admission::Admit);
        assert_eq!(
            plan.candidates.len(),
            if admitted { 2 } else { 1 },
            "{user:?}"
        );
        assert_eq!(plan.fallback.is_some(), admitted, "{user:?}");
    }
}

#[test]
fn rules_match_the_calling_user_and_the_first_upstream() {
    let rules = policy(
        r#"
[[block]]
id = "not-bob"
match = { user = "bob" }
reason = "fixture"

[[block]]
id = "no-direct-opus"
match = { upstream = "anthropic-key", model = "fixture-opus" }
reason = "fixture"
"#,
    )
    .unwrap();
    let operator = OperatorState::default();
    let bob = UserId::parse("bob").unwrap();
    let key = UpstreamId::parse("anthropic-key").unwrap();
    let blocked = |facts: CallFacts<'_>| match rules.plan(&facts, &operator).admission {
        Admission::Reject(Rejection::Blocked { rule, .. }) => Some(rule.as_str().to_owned()),
        Admission::Admit | Admission::Reject(_) => None,
    };
    assert_eq!(
        blocked(CallFacts {
            user: Some(&bob),
            ..facts(Provider::OpenAi, None)
        }),
        Some("not-bob".to_owned())
    );
    assert_eq!(blocked(facts(Provider::OpenAi, None)), None);
    assert_eq!(
        blocked(CallFacts {
            upstream: &key,
            ..facts(Provider::Anthropic, Some("fixture-opus"))
        }),
        Some("no-direct-opus".to_owned())
    );
    assert_eq!(
        blocked(facts(Provider::Anthropic, Some("fixture-opus"))),
        None
    );
    assert_eq!(
        policy(
            r#"[[block]]
id = "b"
match = { upstream = "nowhere" }
reason = "r""#
        )
        .unwrap_err()
        .to_string(),
        "policy rule b matches an unknown upstream"
    );
}

#[test]
fn operator_pause_and_session_blocks_come_first() {
    let policy = policy(RULES).unwrap();
    let session = NativeSessionId::new(Uuid::from_u128(9)).unwrap();
    let now = UnixMillis::from_millis(1_000);
    let blocked =
        OperatorState::default().apply(OperatorCommand::block_session(session, None, now));
    let call = CallFacts {
        session: Some(session),
        ..facts(Provider::Anthropic, Some("fixture-opus"))
    };
    let plan = policy.plan(&call, &blocked);
    assert_eq!(plan.admission, Admission::Reject(Rejection::SessionBlocked));
    assert_eq!(plan.operator_revision.get(), 1);
    let paused = blocked.apply(OperatorCommand::Pause {
        retry_after: RetryAfter::seconds(9).unwrap(),
    });
    assert!(matches!(
        policy.plan(&call, &paused).admission,
        Admission::Reject(Rejection::Paused { .. })
    ));
}

#[test]
fn protected_paths_headers_and_bad_rules_are_refused_at_load() {
    for (rules, expected) in [
        (
            r#"[[transform]]
id = "t"
match = { provider = "anthropic" }
steps = [{ op = "json_set", path = "/system/0/text", value = "x" }]
reason = "r""#,
            "protected body path",
        ),
        (
            r#"[[transform]]
id = "t"
match = { endpoint = "responses" }
steps = [{ op = "json_remove", path = "/previous_response_id" }]
reason = "r""#,
            "protected body path",
        ),
        (
            r#"[[transform]]
id = "t"
match = { endpoint = "chat_completions" }
steps = [{ op = "json_set", path = "/user", value = "someone-else" }]
reason = "r""#,
            "protected body path",
        ),
        (
            r#"[[transform]]
id = "t"
match = { provider = "codex" }
steps = [{ op = "set_header", name = "Authorization", value = "x" }]
reason = "r""#,
            "protected header",
        ),
        (
            r#"[[transform]]
id = "t"
match = { provider = "anthropic" }
steps = [{ op = "remove_header", name = "anthropic-beta" }]
reason = "r""#,
            "protected header",
        ),
        (
            r#"[[block]]
id = "b"
match = {}
reason = "r""#,
            "matches every call",
        ),
        (
            r#"[[fallback]]
id = "f"
provider = "anthropic"
to = "anthropic"
on_status = [429]"#,
            "operator-key upstream",
        ),
        (
            r#"[[fallback]]
id = "f"
provider = "codex"
to = "anthropic-key"
on_status = [429]"#,
            "Codex backend",
        ),
    ] {
        let error = policy(rules).err().unwrap().to_string();
        assert!(error.contains(expected), "{error}");
    }
    // A model-scoped rule may still not reach a protected path on any endpoint.
    assert!(
        policy(
            r#"[[transform]]
id = "t"
match = { model = "m" }
steps = [{ op = "json_remove", path = "/metadata/user_id" }]
reason = "r""#
        )
        .is_err()
    );
}

#[test]
fn transforms_change_only_what_they_name_and_keep_key_order() {
    let policy = policy(RULES).unwrap();
    let plan = policy.plan(&facts(Provider::Anthropic, None), &OperatorState::default());
    let rule = &plan.transforms[0];
    let original = br#"{"model":"m","temperature":1,"system":[{"type":"text","text":"b"},{"type":"text","text":"a"}],"z":0}"#;
    let mut headers = HeaderMap::new();
    let mut body = RequestJson::new(original);
    let outcomes: Vec<StepOutcome> = rule
        .steps
        .iter()
        .map(|step| step.apply(&mut headers, &mut body))
        .collect();
    assert_eq!(outcomes, [StepOutcome::Applied, StepOutcome::Applied]);
    assert_eq!(headers["x-fixture"], "1");
    assert_eq!(
        String::from_utf8(body.changed().unwrap()).unwrap(),
        r#"{"model":"m","system":[{"type":"text","text":"b"},{"type":"text","text":"a"}],"z":0}"#
    );

    // Nothing to remove: the original bytes stand.
    let untouched = br#"{ "model": "m" }"#;
    let mut body = RequestJson::new(untouched);
    assert_eq!(
        rule.steps[0].apply(&mut headers, &mut body),
        StepOutcome::Unchanged
    );
    assert!(body.changed().is_none());
    let mut body = RequestJson::new(b"not json");
    assert_eq!(
        rule.steps[0].apply(&mut headers, &mut body),
        StepOutcome::NotJson
    );
}
