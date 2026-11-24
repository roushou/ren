//! Which checkout holds which dev instance (docs/plans/instances.md, phase
//! 5): `cargo xtask dev up` in a checkout claims the lowest free
//! [`DevIndex`], and every other `dev` command there acts on that one, so
//! each worktree has an instance of its own. Pure: the shell reads and
//! writes [`DevClaims::FILE`] under a lock.

use super::index::DevIndex;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Each checkout's instance, each index held at most once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DevClaims {
    claims: BTreeMap<PathBuf, DevIndex>,
}

/// Every instance is held by a checkout that still exists.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "all {} dev instances are claimed: stop one with `cargo xtask dev down --release` in its checkout",
    DevIndex::COUNT
)]
pub(crate) struct AllClaimed;

impl DevClaims {
    /// Its file, in the user's state folder beside the instances'.
    pub const FILE: &str = "ren-dev-claims.json";

    /// `checkout`'s instance, if it holds one.
    pub fn of(&self, checkout: &Path) -> Option<DevIndex> {
        self.claims.get(checkout).copied()
    }

    /// Each claim, by index.
    pub fn all(&self) -> Vec<(DevIndex, &Path)> {
        let mut all: Vec<(DevIndex, &Path)> = self
            .claims
            .iter()
            .map(|(checkout, index)| (*index, checkout.as_path()))
            .collect();
        all.sort();
        all
    }

    /// `checkout`'s instance, claimed if it holds none: the lowest index no
    /// checkout holds, once the claims of checkouts that are gone
    /// (`exists` says not) are let go.
    pub fn claim(
        mut self,
        checkout: &Path,
        exists: impl Fn(&Path) -> bool,
    ) -> Result<(Self, DevIndex), AllClaimed> {
        if let Some(index) = self.of(checkout) {
            return Ok((self, index));
        }
        self.claims.retain(|held, _| exists(held));
        let free = (0..DevIndex::COUNT)
            .filter_map(|index| DevIndex::try_from(index).ok())
            .find(|index| !self.claims.values().any(|held| held == index))
            .ok_or(AllClaimed)?;
        self.claims.insert(checkout.to_path_buf(), free);
        Ok((self, free))
    }

    /// Without `checkout`'s claim.
    #[must_use]
    pub fn release(mut self, checkout: &Path) -> Self {
        self.claims.remove(checkout);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{AllClaimed, DevClaims, DevIndex};
    use std::path::{Path, PathBuf};

    fn index(index: u8) -> DevIndex {
        DevIndex::try_from(index).unwrap()
    }

    #[test]
    fn each_checkout_claims_the_lowest_free_instance_once() {
        let all = |_: &Path| true;
        let (claims, main) = DevClaims::default()
            .claim(Path::new("/src/ren"), all)
            .unwrap();
        assert_eq!(main, DevIndex::FIRST);
        let (claims, one) = claims.claim(Path::new("/src/ren-1"), all).unwrap();
        assert_eq!(one, index(1));
        // Claimed again, the same one.
        let (claims, again) = claims.claim(Path::new("/src/ren-1"), all).unwrap();
        assert_eq!(again, index(1));
        // A released index is the next taken.
        let claims = claims.release(Path::new("/src/ren"));
        assert_eq!(claims.of(Path::new("/src/ren")), None);
        let (claims, two) = claims.claim(Path::new("/src/ren-2"), all).unwrap();
        assert_eq!(two, DevIndex::FIRST);
        assert_eq!(
            claims.all(),
            [
                (DevIndex::FIRST, Path::new("/src/ren-2")),
                (index(1), Path::new("/src/ren-1"))
            ]
        );
    }

    #[test]
    fn a_gone_checkouts_claim_is_let_go_and_a_full_host_refuses() {
        let mut claims = DevClaims::default();
        for n in 0..DevIndex::COUNT {
            claims = claims
                .claim(&PathBuf::from(format!("/src/ren-{n}")), |_| true)
                .unwrap()
                .0;
        }
        assert_eq!(
            claims.clone().claim(Path::new("/src/new"), |_| true),
            Err(AllClaimed)
        );
        // /src/ren-3 was removed: its instance is free again.
        let (claims, taken) = claims
            .claim(Path::new("/src/new"), |held| {
                held != Path::new("/src/ren-3")
            })
            .unwrap();
        assert_eq!(taken, index(3));
        assert_eq!(claims.of(Path::new("/src/ren-3")), None);
    }

    #[test]
    fn the_file_keeps_its_shape() {
        let (claims, _) = DevClaims::default()
            .claim(Path::new("/src/ren"), |_| true)
            .unwrap();
        let text = serde_json::to_string(&claims).unwrap();
        assert_eq!(text, r#"{"claims":{"/src/ren":0}}"#);
        assert_eq!(serde_json::from_str::<DevClaims>(&text).unwrap(), claims);
        assert!(serde_json::from_str::<DevClaims>(r#"{"claims":{"/a":8}}"#).is_err());
    }
}
