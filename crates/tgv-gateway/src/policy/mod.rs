//! Call policy: a pure plan over typed call facts and the operator's
//! state. It decides admission, the ordered upstream candidates, the request
//! transforms, and which responses may fall over. It performs no I/O; the
//! gateway records the plan and carries it out.
//!
//! v1 rules are small: block rules matched on call facts, the operator's pause
//! and session blocks, request transforms, and one fallback rule per provider
//! (a plan-limit status falls over to an operator-key upstream, opt-in).
//! Transform rules may not touch protected headers or body paths; a rule that
//! tries is rejected when the configuration loads.

mod transform;

pub use transform::{RequestJson, StepOutcome, StepRecord, TransformStep};

use crate::{
    operator::{OperatorState, Pause, RetryAfter},
    retry::RetryPolicy,
};
use std::{collections::HashSet, sync::Arc};
use tgv_protocol::{
    native::NativeSessionId,
    route::{Endpoint, Provider, Transport},
};
use tgv_recorder::{
    capture::CallRejection,
    routing::{OperatorRevision, UpstreamId},
    time::UnixMillis,
};

/// A rule's identifier, written in configuration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct RuleId(Box<str>);

impl<'de> serde::Deserialize<'de> for RuleId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Rule ids follow the upstream id format.
        let id = UpstreamId::deserialize(deserializer)?;
        Ok(Self(id.as_str().into()))
    }
}

impl RuleId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Which calls a rule applies to. Every field given must match; at least one
/// must be given.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Matcher {
    pub provider: Option<Provider>,
    pub endpoint: Option<Endpoint>,
    /// The request body's top-level `model`, exactly.
    pub model: Option<String>,
}

/// Reject matching calls with a non-retryable `403`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockRule {
    pub id: RuleId,
    #[serde(rename = "match")]
    pub matcher: Matcher,
    pub reason: String,
}

/// Change matching HTTP requests before dispatch. Recorded with the original
/// and resulting bodies.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformRule {
    pub id: RuleId,
    #[serde(rename = "match")]
    pub matcher: Matcher,
    pub steps: Vec<TransformStep>,
    pub reason: String,
}

/// Fall over from a provider's default upstream to an operator-key upstream
/// when it answers one of these statuses before anything reached the client.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FallbackRule {
    pub id: RuleId,
    pub provider: Provider,
    pub to: UpstreamId,
    pub on_status: Vec<u16>,
}

/// The `[policy]` configuration section.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PolicyConfig {
    pub block: Vec<BlockRule>,
    pub transform: Vec<TransformRule>,
    pub fallback: Vec<FallbackRule>,
}

/// How an upstream entry authenticates, as far as policy needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    /// The client's own credentials pass through.
    ClientOwned,
    /// TGV replaces the credentials with an operator API key.
    OperatorKey,
}

/// An upstream entry, as policy sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamSummary {
    pub id: UpstreamId,
    pub provider: Provider,
    pub auth: AuthKind,
}

/// Why a policy configuration was refused. Values are named, never echoed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("policy rule id {0} is used twice")]
    DuplicateRule(String),
    #[error("policy rule {0} matches every call; give a provider, endpoint, or model")]
    UnscopedRule(String),
    #[error("transform rule {0} has no steps")]
    EmptyTransform(String),
    #[error("transform rule {rule}: {problem}")]
    Step {
        rule: String,
        problem: transform::StepError,
    },
    #[error("fallback rule {0} names an unknown upstream")]
    UnknownUpstream(String),
    #[error("fallback rule {0} must go to an operator-key upstream of the same provider")]
    FallbackTarget(String),
    #[error("fallback rule {0}: the Codex backend accepts no operator keys")]
    CodexFallback(String),
    #[error("provider {0} has more than one fallback rule")]
    DuplicateFallback(Provider),
    #[error("fallback rule {0} needs statuses between 400 and 599")]
    FallbackStatus(String),
}

/// What is known about a call when it is planned.
#[derive(Debug, Clone, Copy)]
pub struct CallFacts<'a> {
    pub transport: Transport,
    pub provider: Provider,
    /// The entry the call goes to first: its route's entry.
    pub upstream: &'a UpstreamId,
    pub endpoint: Endpoint,
    pub model: Option<&'a str>,
    pub session: Option<NativeSessionId>,
    pub now: UnixMillis,
}

/// Whether the call goes ahead.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "admission")]
pub enum Admission {
    Admit,
    Reject(Rejection),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "rejection")]
pub enum Rejection {
    Paused { retry_after: RetryAfter },
    SessionBlocked,
    Blocked { rule: RuleId, reason: String },
}

/// Upstreams to try, in order; never empty.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Candidates(Vec<UpstreamId>);

impl Candidates {
    fn one(id: UpstreamId) -> Self {
        Self(vec![id])
    }

    pub fn first(&self) -> &UpstreamId {
        // Built non-empty and never shrunk.
        &self.0[0]
    }

    pub fn get(&self, index: usize) -> Option<&UpstreamId> {
        self.0.get(index)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The decisions for one call, recorded before dispatch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CallPlan {
    pub admission: Admission,
    pub candidates: Candidates,
    /// Rules whose steps run on the request, in order.
    pub transforms: Vec<Arc<TransformRule>>,
    pub retry: RetryPolicy,
    pub operator_revision: OperatorRevision,
    /// The fallback rule that added candidates, if any.
    pub fallback: Option<RuleId>,
}

impl Rejection {
    pub fn label(&self) -> CallRejection {
        match self {
            Self::Paused { .. } => CallRejection::Paused,
            Self::SessionBlocked => CallRejection::SessionBlocked,
            Self::Blocked { .. } => CallRejection::Blocked,
        }
    }
}

/// A plan as it is recorded with its call.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PlanRecord<'a> {
    #[serde(flatten)]
    pub admission: &'a Admission,
    pub candidates: &'a Candidates,
    /// Transform rules applied, in order.
    pub transforms: Vec<&'a RuleId>,
    pub fallback: Option<&'a RuleId>,
    pub retry: &'a RetryPolicy,
}

impl CallPlan {
    pub fn record(&self) -> PlanRecord<'_> {
        PlanRecord {
            admission: &self.admission,
            candidates: &self.candidates,
            transforms: self.transforms.iter().map(|rule| &rule.id).collect(),
            fallback: self.fallback.as_ref(),
            retry: &self.retry,
        }
    }
}

/// A validated policy. Immutable; a reload builds a new one.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    block: Vec<BlockRule>,
    transform: Vec<Arc<TransformRule>>,
    fallback: Vec<FallbackRule>,
    /// Upstreams with client-owned credentials: the only ones a fallback
    /// rule falls over from.
    client_owned: HashSet<UpstreamId>,
}

impl Matcher {
    fn is_empty(&self) -> bool {
        self.provider.is_none() && self.endpoint.is_none() && self.model.is_none()
    }

    fn matches(&self, facts: &CallFacts<'_>) -> bool {
        self.provider
            .is_none_or(|provider| provider == facts.provider)
            && self
                .endpoint
                .is_none_or(|endpoint| endpoint == facts.endpoint)
            && self
                .model
                .as_deref()
                .is_none_or(|model| facts.model == Some(model))
    }

    /// Endpoints this matcher can apply to, for protected-path checks.
    fn endpoints(&self) -> Vec<Endpoint> {
        match self.endpoint {
            Some(endpoint) => vec![endpoint],
            None => <Endpoint as tgv_protocol::label::Labeled>::ALL.to_vec(),
        }
    }
}

impl Policy {
    pub fn new(config: PolicyConfig, upstreams: &[UpstreamSummary]) -> Result<Self, PolicyError> {
        let mut ids = HashSet::new();
        let ids_in_use = config
            .block
            .iter()
            .map(|rule| &rule.id)
            .chain(config.transform.iter().map(|rule| &rule.id))
            .chain(config.fallback.iter().map(|rule| &rule.id));
        for id in ids_in_use {
            if !ids.insert(id.clone()) {
                return Err(PolicyError::DuplicateRule(id.as_str().to_owned()));
            }
        }
        for rule in &config.block {
            if rule.matcher.is_empty() {
                return Err(PolicyError::UnscopedRule(rule.id.as_str().to_owned()));
            }
        }
        for rule in &config.transform {
            let id = || rule.id.as_str().to_owned();
            if rule.matcher.is_empty() {
                return Err(PolicyError::UnscopedRule(id()));
            }
            if rule.steps.is_empty() {
                return Err(PolicyError::EmptyTransform(id()));
            }
            for step in &rule.steps {
                step.check(&rule.matcher.endpoints())
                    .map_err(|problem| PolicyError::Step {
                        rule: id(),
                        problem,
                    })?;
            }
        }
        let mut providers = HashSet::new();
        for rule in &config.fallback {
            let id = || rule.id.as_str().to_owned();
            let Some(target) = upstreams.iter().find(|upstream| upstream.id == rule.to) else {
                return Err(PolicyError::UnknownUpstream(id()));
            };
            if rule.provider == Provider::Codex {
                return Err(PolicyError::CodexFallback(id()));
            }
            if target.provider != rule.provider || target.auth != AuthKind::OperatorKey {
                return Err(PolicyError::FallbackTarget(id()));
            }
            if rule.on_status.is_empty()
                || rule
                    .on_status
                    .iter()
                    .any(|status| !(400..=599).contains(status))
            {
                return Err(PolicyError::FallbackStatus(id()));
            }
            if !providers.insert(rule.provider) {
                return Err(PolicyError::DuplicateFallback(rule.provider));
            }
        }
        Ok(Self {
            block: config.block,
            transform: config.transform.into_iter().map(Arc::new).collect(),
            fallback: config.fallback,
            client_owned: upstreams
                .iter()
                .filter(|upstream| match upstream.auth {
                    AuthKind::ClientOwned => true,
                    AuthKind::OperatorKey => false,
                })
                .map(|upstream| upstream.id.clone())
                .collect(),
        })
    }

    /// Plan one call. WebSocket calls are routed once at the upgrade, so they
    /// get one candidate, no transforms, and no fallback; admission is still
    /// checked for every call.
    pub fn plan(&self, facts: &CallFacts<'_>, operator: &OperatorState) -> CallPlan {
        let admission = self.admission(facts, operator);
        let primary = facts.upstream.clone();
        let (candidates, transforms, retry, fallback) = match facts.transport {
            Transport::WebSocket => (
                Candidates::one(primary),
                Vec::new(),
                RetryPolicy::default(),
                None,
            ),
            Transport::Http => {
                let transforms = self
                    .transform
                    .iter()
                    .filter(|rule| rule.matcher.matches(facts))
                    .cloned()
                    .collect();
                // A call sent first to an operator-key entry (one with direct
                // routes) already uses the operator's key: it never falls over.
                let fallback = self
                    .fallback
                    .iter()
                    .filter(|_| self.client_owned.contains(&primary))
                    .find(|rule| rule.provider == facts.provider);
                match fallback {
                    None => (
                        Candidates::one(primary),
                        transforms,
                        RetryPolicy::default(),
                        None,
                    ),
                    Some(rule) => (
                        Candidates(vec![primary, rule.to.clone()]),
                        transforms,
                        RetryPolicy::falling_over_on(rule.on_status.clone()),
                        Some(rule.id.clone()),
                    ),
                }
            }
        };
        CallPlan {
            admission,
            candidates,
            transforms,
            retry,
            operator_revision: operator.revision(),
            fallback,
        }
    }

    fn admission(&self, facts: &CallFacts<'_>, operator: &OperatorState) -> Admission {
        if let Pause::Paused { retry_after } = operator.pause() {
            return Admission::Reject(Rejection::Paused { retry_after });
        }
        if facts
            .session
            .is_some_and(|session| operator.blocks(session, facts.now))
        {
            return Admission::Reject(Rejection::SessionBlocked);
        }
        match self.block.iter().find(|rule| rule.matcher.matches(facts)) {
            Some(rule) => Admission::Reject(Rejection::Blocked {
                rule: rule.id.clone(),
                reason: rule.reason.clone(),
            }),
            None => Admission::Admit,
        }
    }
}

#[cfg(test)]
mod tests;
