//! Token usage over time, by model, and by user, as the read API sends it. Each
//! type converts once from the type it reports (`From`), naming every field.

use super::calls::TokenTotals;
use ren_protocol::label::Stored;
use ren_protocol::route::Provider;
use ren_recorder::time::UnixMillis;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Calls, errors, and tokens over some set of calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UsageCounts {
    pub calls: u64,
    /// The calls the `errors` status filter finds.
    pub errors: u64,
    pub tokens: TokenTotals,
}

impl From<ren_store::view::UsageCounts> for UsageCounts {
    fn from(value: ren_store::view::UsageCounts) -> Self {
        let ren_store::view::UsageCounts {
            calls,
            errors,
            tokens,
        } = value;
        Self {
            calls,
            errors,
            tokens: tokens.into(),
        }
    }
}

/// Usage over a time range, by the hour, by model, and by user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UsageView {
    pub totals: UsageCounts,
    /// Hours that had calls, oldest first, keyed by the hour's start (UTC).
    /// Clients group them into local days.
    pub hours: Vec<UsageBucket>,
    /// Each reported model's usage, most input first. Calls without a
    /// reported model are grouped under `null`.
    pub models: Vec<ModelUsage>,
    /// Each named user's usage, most input first. Calls no named user made
    /// (unlisted or unidentified callers) are grouped under `null`.
    pub users: Vec<UserUsage>,
}

impl From<ren_store::view::UsageView> for UsageView {
    fn from(value: ren_store::view::UsageView) -> Self {
        let ren_store::view::UsageView {
            totals,
            hours,
            models,
            users,
        } = value;
        Self {
            totals: totals.into(),
            hours: hours.into_iter().map(Into::into).collect(),
            models: models.into_iter().map(Into::into).collect(),
            users: users.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UsageBucket {
    #[ts(as = "u64")]
    pub start: UnixMillis,
    #[serde(flatten)]
    pub usage: UsageCounts,
}

impl From<ren_store::view::UsageBucket> for UsageBucket {
    fn from(value: ren_store::view::UsageBucket) -> Self {
        let ren_store::view::UsageBucket { start, usage } = value;
        Self {
            start,
            usage: usage.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelUsage {
    #[ts(as = "String")]
    pub provider: Stored<Provider>,
    pub model: Option<String>,
    #[serde(flatten)]
    pub usage: UsageCounts,
}

impl From<ren_store::view::ModelUsage> for ModelUsage {
    fn from(value: ren_store::view::ModelUsage) -> Self {
        let ren_store::view::ModelUsage {
            provider,
            model,
            usage,
        } = value;
        Self {
            provider,
            model,
            usage: usage.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UserUsage {
    pub user: Option<String>,
    #[serde(flatten)]
    pub usage: UsageCounts,
}

impl From<ren_store::view::UserUsage> for UserUsage {
    fn from(value: ren_store::view::UserUsage) -> Self {
        let ren_store::view::UserUsage { user, usage } = value;
        Self {
            user,
            usage: usage.into(),
        }
    }
}
