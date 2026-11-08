use super::{
    Admission, AuthKind, CallFacts, Policy, PolicyConfig, PolicyError, Rejection, RequestJson,
    StepOutcome, UpstreamSummary,
};
use crate::{
    operator::{OperatorCommand, OperatorState, RetryAfter},
    retry::{Delivery, RetryDecision, WireFact},
};
use http::HeaderMap;
use tgv_protocol::{
    native::NativeSessionId,
    route::{Endpoint, Provider, Transport},
};
use tgv_recorder::{routing::UpstreamId, time::UnixMillis};
use uuid::Uuid;

fn upstreams() -> Vec<UpstreamSummary> {
    let mut entries: Vec<UpstreamSummary> =
        [Provider::Anthropic, Provider::Codex, Provider::OpenAi]
            .into_iter()
            .map(|provider| UpstreamSummary {
                id: UpstreamId::default_for(provider),
                provider,
                auth: AuthKind::ClientOwned,
            })
            .collect();
    entries.push(UpstreamSummary {
        id: UpstreamId::parse("anthropic-key").unwrap(),
        provider: Provider::Anthropic,
        auth: AuthKind::OperatorKey,
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
