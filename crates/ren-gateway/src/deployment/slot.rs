//! A preview's slot: which ports it runs on.

use serde::{Deserialize, Serialize};

/// A preview's place, `0` up to the capacity: its daemon listens on
/// loopback `18100 + 2 × slot`, its UI on the next port, and the tailnet
/// reaches them on `9450 + 2 × slot` and the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Slot(u8);

impl Slot {
    const LOOPBACK: u16 = 18_100;
    const TAILNET: u16 = 9_450;

    /// The first `count` slots.
    pub fn first(count: usize) -> impl Iterator<Item = Self> {
        (0..=u8::MAX).map(Self).take(count)
    }

    pub fn index(self) -> usize {
        usize::from(self.0)
    }

    /// Its daemon's loopback port, then its UI's.
    pub fn ports(self) -> (u16, u16) {
        let daemon = Self::LOOPBACK + 2 * u16::from(self.0);
        (daemon, daemon + 1)
    }

    /// The tailnet ports its gateway and UI are published on.
    pub fn published(self) -> (u16, u16) {
        let gateway = Self::TAILNET + 2 * u16::from(self.0);
        (gateway, gateway + 1)
    }

    /// The name its instance runs as: `preview-<slot>`.
    pub fn name(self) -> String {
        format!("preview-{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::Slot;

    #[test]
    fn each_slot_has_its_own_ports() {
        let slots: Vec<Slot> = Slot::first(3).collect();
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0].ports(), (18_100, 18_101));
        assert_eq!(slots[2].ports(), (18_104, 18_105));
        assert_eq!(slots[2].published(), (9_454, 9_455));
        assert_eq!(slots[1].name(), "preview-1");
        // The most slots a capacity allows stays clear of the canary's ports.
        let last = Slot::first(crate::release::PreviewCapacity::MOST.get())
            .last()
            .unwrap();
        assert!(last.ports().1 < 17_000 || last.ports().0 > 17_787);
        assert!(last.published().1 < 9_500);
    }
}
