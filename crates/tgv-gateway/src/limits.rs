//! How many calls each caller may have running at once: a limit per named
//! user, and one limit shared by every caller that is not a named user.
//! Separate from, and checked before, the gateway's global admission.

use crate::user::Users;
use std::{collections::HashMap, sync::Arc};
use tgv_recorder::caller::{Caller, UserId};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

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
    pub(crate) fn new(users: &Users, unnamed: Option<u32>) -> Self {
        let semaphore =
            |limit: u32| Arc::new(Semaphore::new(usize::try_from(limit).unwrap_or(usize::MAX)));
        Self {
            named: users
                .configs()
                .iter()
                .filter_map(|config| {
                    config
                        .max_concurrent
                        .map(|limit| (config.id.clone(), semaphore(limit.get())))
                })
                .collect(),
            unnamed: unnamed.map(semaphore),
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
    use super::{AtLimit, UserLimits};
    use crate::user::{UserConfig, Users};
    use std::num::NonZeroU32;
    use tgv_recorder::caller::{Caller, Proof, TailscaleLogin, UserId};

    #[test]
    fn each_user_and_the_unnamed_callers_have_their_own_limit() {
        let user = |name: &str, limit: Option<u32>| UserConfig {
            id: UserId::parse(name).unwrap(),
            tailscale_logins: vec![TailscaleLogin::parse(&format!("{name}@example.com")).unwrap()],
            token_blake3: None,
            max_concurrent: limit.and_then(NonZeroU32::new),
        };
        let users = Users::new(vec![user("alice", Some(1)), user("bob", None)]).unwrap();
        let limits = UserLimits::new(&users, Some(1));
        let caller = |name: &str| Caller::User {
            user: UserId::parse(name).unwrap(),
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
