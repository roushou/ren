//! Which of the host's dev instances (docs/plans/instances.md, phase 5):
//! each checkout claims one, and its index places it, so several run side
//! by side. Index 0 is where the one dev slot was: its ports, folders, and
//! units are unchanged.

use serde::{Deserialize, Serialize};
use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
};

/// One dev instance's index, below [`DevIndex::COUNT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub(crate) struct DevIndex(u8);

/// Why a number is not a [`DevIndex`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a dev instance's index is from 0 to 7: {0}")]
pub(crate) struct NotADevIndex(pub String);

impl DevIndex {
    /// How many can run: each takes ports in steps of ten on loopback and a
    /// hundred on the tailnet.
    pub const COUNT: u8 = 8;
    /// The first: the one dev slot's places.
    #[cfg(test)]
    pub const FIRST: Self = Self(0);

    /// `base`'s port for this instance, on loopback.
    fn loopback(self, base: u16) -> SocketAddr {
        SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            base + 10 * u16::from(self.0),
        )
    }

    /// The fake provider's address.
    pub fn provider(self) -> SocketAddr {
        self.loopback(18_999)
    }

    /// The dev daemon's address.
    pub fn daemon(self) -> SocketAddr {
        self.loopback(18_787)
    }

    /// The dev UI's address.
    pub fn ui(self) -> SocketAddr {
        self.loopback(13_000)
    }

    /// The traffic generator's control API, which the dev UI's toolbar
    /// reaches through the UI server; never published.
    pub fn control(self) -> SocketAddr {
        self.loopback(13_001)
    }

    /// The tailnet HTTPS ports of its gateway and UI, beside the installed
    /// service's, the canary's, and the previews'.
    pub fn tailnet(self) -> (u16, u16) {
        let step = 100 * u16::from(self.0);
        (9443 + step, 9444 + step)
    }

    /// The name its units, folders, and containers share: `ren-dev` for
    /// the first, `ren-dev-<index>` after.
    pub fn name(self) -> String {
        format!("ren-{}", self.label())
    }

    /// Its name without `ren-`, as containers are named (`ren-<label>-ui`).
    pub fn label(self) -> String {
        match self.0 {
            0 => "dev".to_owned(),
            index => format!("dev-{index}"),
        }
    }
}

impl TryFrom<u8> for DevIndex {
    type Error = NotADevIndex;

    fn try_from(index: u8) -> Result<Self, Self::Error> {
        if index < Self::COUNT {
            Ok(Self(index))
        } else {
            Err(NotADevIndex(index.to_string()))
        }
    }
}

impl From<DevIndex> for u8 {
    fn from(index: DevIndex) -> Self {
        index.0
    }
}

impl FromStr for DevIndex {
    type Err = NotADevIndex;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse::<u8>()
            .ok()
            .and_then(|index| Self::try_from(index).ok())
            .ok_or_else(|| NotADevIndex(text.to_owned()))
    }
}

impl fmt::Display for DevIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::DevIndex;
    use std::collections::HashSet;

    #[test]
    fn the_first_keeps_the_dev_slots_places() {
        let first = DevIndex::FIRST;
        assert_eq!(first.provider().to_string(), "127.0.0.1:18999");
        assert_eq!(first.daemon().to_string(), "127.0.0.1:18787");
        assert_eq!(first.ui().to_string(), "127.0.0.1:13000");
        assert_eq!(first.control().to_string(), "127.0.0.1:13001");
        assert_eq!(first.tailnet(), (9443, 9444));
        assert_eq!(first.name(), "ren-dev");
    }

    #[test]
    fn no_two_instances_share_a_port_or_a_name() {
        let mut ports = HashSet::new();
        let mut names = HashSet::new();
        for index in (0..DevIndex::COUNT).map(DevIndex) {
            for port in [
                index.provider().port(),
                index.daemon().port(),
                index.ui().port(),
                index.control().port(),
                index.tailnet().0,
                index.tailnet().1,
            ] {
                assert!(ports.insert(port), "{index}: {port}");
                // Clear of the previews' (18100.., 9450..9465), the
                // canary's (17000, 17787, 9445, 9446), and the worktrees'
                // (20000..).
                assert!(!(18_100..18_120).contains(&port) && !(9_445..9_466).contains(&port));
                assert!(port < 20_000 && port != 17_000 && port != 17_787);
            }
            assert!(names.insert(index.name()));
        }
        let last: DevIndex = (DevIndex::COUNT - 1).to_string().parse().unwrap();
        assert_eq!(last.name(), "ren-dev-7");
        assert!(DevIndex::COUNT.to_string().parse::<DevIndex>().is_err());
    }
}
