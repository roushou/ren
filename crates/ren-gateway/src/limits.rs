//! How many calls each caller may have running at once: a limit per named
//! user, and one limit shared by every caller that is not a named user.
//! Separate from, and checked before, the gateway's global admission.

use crate::user::Users;
use ren_recorder::caller::{Caller, UserId};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// How many calls or channels may run at once: 1 to 1024. Read from
/// configuration as a number, so a value out of range is refused as it loads.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "u64", into = "u64")]
pub struct ConcurrencyLimit(u16);

/// A concurrency limit outside 1 to 1024.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a concurrency limit must be between 1 and {max}", max = ConcurrencyLimit::MAX.0)]
pub struct ConcurrencyOutOfRange;

impl ConcurrencyLimit {
    pub const MIN: Self = Self(1);
    pub const MAX: Self = Self(1024);

    /// `limit`, brought into range: for defaults and tests, which name
    /// their value in code.
    pub const fn saturating(limit: u16) -> Self {
        if limit < Self::MIN.0 {
            Self::MIN
        } else if limit > Self::MAX.0 {
            Self::MAX
        } else {
            Self(limit)
        }
    }

    pub fn get(self) -> usize {
        self.0.into()
    }

    /// A semaphore with this many permits.
    pub(crate) fn semaphore(self) -> Arc<Semaphore> {
        Arc::new(Semaphore::new(self.get()))
    }
}

impl TryFrom<u64> for ConcurrencyLimit {
    type Error = ConcurrencyOutOfRange;

    fn try_from(limit: u64) -> Result<Self, ConcurrencyOutOfRange> {
        u16::try_from(limit)
            .ok()
            .filter(|limit| (Self::MIN.0..=Self::MAX.0).contains(limit))
            .map(Self)
            .ok_or(ConcurrencyOutOfRange)
    }
}

impl From<ConcurrencyLimit> for u64 {
    fn from(limit: ConcurrencyLimit) -> Self {
        limit.0.into()
    }
}

/// The configured per-caller limits.
#[derive(Debug, Default)]
pub(crate) struct UserLimits {
    named: HashMap<UserId, Arc<Semaphore>>,
    unnamed: Option<Arc<Semaphore>>,
}

/// A caller is at its limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AtLimit;

impl UserLimits {
    pub(crate) fn new(users: &Users, unnamed: Option<ConcurrencyLimit>) -> Self {
        Self {
            named: users
                .configs()
                .iter()
                .filter_map(|config| {
                    config
                        .max_concurrent
                        .map(|limit| (config.id.clone(), limit.semaphore()))
                })
                .collect(),
            unnamed: unnamed.map(ConcurrencyLimit::semaphore),
        }
    }

    /// A slot for one more of `caller`'s calls, held until the call ends;
    /// `None` when its kind of caller has no limit.
    pub(crate) fn admit(&self, caller: &Caller) -> Result<Option<OwnedSemaphorePermit>, AtLimit> {
        let limit = match caller.user() {
            Some(user) => self.named.get(user),
            None => self.unnamed.as_ref(),
        };
        limit
            .map(|semaphore| semaphore.clone().try_acquire_owned().map_err(|_| AtLimit))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::{AtLimit, ConcurrencyLimit, UserLimits};
    use crate::user::{UserConfig, Users};
    use ren_recorder::caller::{Caller, Proof, TailscaleLogin, UserId};

    #[test]
    fn each_user_and_the_unnamed_callers_have_their_own_limit() {
        let user = |name: &str, limit: Option<u16>| UserConfig {
            id: name.parse::<UserId>().unwrap(),
            tailscale_logins: vec![
                format!("{name}@example.com")
                    .parse::<TailscaleLogin>()
                    .unwrap(),
            ],
            token_blake3: None,
            max_concurrent: limit.map(ConcurrencyLimit::saturating),
        };
        let users = Users::new(vec![user("alice", Some(1)), user("bob", None)]).unwrap();
        let limits = UserLimits::new(&users, Some(ConcurrencyLimit::saturating(1)));
        let caller = |name: &str| Caller::User {
            user: name.parse::<UserId>().unwrap(),
            proof: Proof::Token { login: None },
        };
        let alice = limits.admit(&caller("alice")).unwrap();
        assert!(alice.is_some());
        assert_eq!(limits.admit(&caller("alice")).err(), Some(AtLimit));
        // Bob has no limit; unnamed callers share theirs.
        assert!(limits.admit(&caller("bob")).unwrap().is_none());
        let unnamed = limits.admit(&Caller::Unidentified).unwrap();
        assert_eq!(limits.admit(&Caller::Unidentified).err(), Some(AtLimit));
        drop((alice, unnamed));
        assert!(limits.admit(&caller("alice")).unwrap().is_some());
        assert!(limits.admit(&Caller::Unidentified).unwrap().is_some());
    }
}
