//! What a run may reach ([`RelayPolicy`]), the [`Verdict`] on a tunnel it
//! asks for, and the addresses a tunnel may then go to
//! ([`PublicAddress`]).

use ren_destination::{Destination, Host, Target};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// The destinations one holder's tunnels may reach; nothing else.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayPolicy {
    destinations: Vec<Destination>,
}

/// Whether a tunnel is let through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Let through to the destination that lists it.
    Admitted(Destination),
    Refused(Refusal),
}

/// Why a tunnel was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum Refusal {
    /// No destination of the holder's policy lists it.
    Unlisted,
    /// Its name resolved only to addresses on this host or a private
    /// network, which a name never reaches.
    PrivateAddress,
}

impl Refusal {
    pub fn label(self) -> &'static str {
        self.into()
    }
}

impl RelayPolicy {
    pub fn new(destinations: impl IntoIterator<Item = Destination>) -> Self {
        Self {
            destinations: destinations.into_iter().collect(),
        }
    }

    pub fn destinations(&self) -> &[Destination] {
        &self.destinations
    }

    /// The first destination that admits `target`, else the refusal.
    pub fn decide(&self, target: &Target) -> Verdict {
        self.destinations
            .iter()
            .find(|destination| destination.admits(target))
            .map_or(Verdict::Refused(Refusal::Unlisted), |destination| {
                Verdict::Admitted(destination.clone())
            })
    }
}

/// An address an admitted tunnel connects to: a name's public addresses
/// only, or the address a destination lists itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reachable(IpAddr);

impl Reachable {
    pub fn address(self) -> IpAddr {
        self.0
    }

    /// Where a tunnel `destination` admitted to `target` may connect, of
    /// the `resolved` addresses: the listed address itself, or a name's
    /// public ones; refused when a name has none.
    pub fn of(
        destination: &Destination,
        target: &Target,
        resolved: &[IpAddr],
    ) -> Result<Vec<Self>, Refusal> {
        if destination.lists_address() {
            return Ok(match target.host() {
                Host::Address(address) => vec![Self(*address)],
                Host::Name(_) => Vec::new(),
            });
        }
        let public: Vec<Self> = resolved
            .iter()
            .filter_map(|address| PublicAddress::try_from(*address).ok())
            .map(|public| Self(public.0))
            .collect();
        if public.is_empty() && !resolved.is_empty() {
            return Err(Refusal::PrivateAddress);
        }
        Ok(public)
    }
}

/// An address on the public internet: not this host, a private or shared
/// network (a tailnet's `100.64.0.0/10` included), link-local, multicast,
/// or reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicAddress(IpAddr);

/// Why an address is not public.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a public address")]
pub struct NotPublic;

impl PublicAddress {
    fn v4_public(v4: Ipv4Addr) -> bool {
        let [a, b, ..] = v4.octets();
        !(v4.is_unspecified()
            || v4.is_loopback()
            || v4.is_private()
            || v4.is_link_local()
            || v4.is_multicast()
            || v4.is_broadcast()
            || v4.is_documentation()
            || a == 0
            // Shared address space (carrier-grade NAT, Tailscale).
            || (a == 100 && (64..128).contains(&b))
            // Benchmarking.
            || (a == 198 && (18..20).contains(&b))
            // Reserved, 240.0.0.0/4.
            || a >= 240)
    }

    fn v6_public(v6: Ipv6Addr) -> bool {
        if let Some(v4) = v6.to_ipv4_mapped() {
            return Self::v4_public(v4);
        }
        let first = v6.segments()[0];
        !(v6.is_unspecified()
            || v6.is_loopback()
            || v6.is_multicast()
            // Unique local, fc00::/7.
            || (first & 0xfe00) == 0xfc00
            // Link-local, fe80::/10.
            || (first & 0xffc0) == 0xfe80
            // Documentation, 2001:db8::/32.
            || (first == 0x2001 && v6.segments()[1] == 0x0db8))
    }
}

impl TryFrom<IpAddr> for PublicAddress {
    type Error = NotPublic;

    fn try_from(address: IpAddr) -> Result<Self, Self::Error> {
        let public = match address {
            IpAddr::V4(v4) => Self::v4_public(v4),
            IpAddr::V6(v6) => Self::v6_public(v6),
        };
        if public {
            Ok(Self(address))
        } else {
            Err(NotPublic)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PublicAddress, Reachable, Refusal, RelayPolicy, Verdict};
    use ren_destination::{Destination, Target};
    use std::net::IpAddr;

    fn policy() -> RelayPolicy {
        RelayPolicy::new(
            ["crates.io:443", "*.crates.io:443", "127.0.0.1:4000"]
                .iter()
                .map(|text| text.parse::<Destination>().unwrap()),
        )
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn a_policy_admits_what_it_lists_and_refuses_the_rest() {
        let policy = policy();
        let decide = |target: &str| policy.decide(&target.parse::<Target>().unwrap());
        assert_eq!(
            decide("static.crates.io:443"),
            Verdict::Admitted("*.crates.io:443".parse().unwrap())
        );
        assert_eq!(decide("evil.io:443"), Verdict::Refused(Refusal::Unlisted));
        assert_eq!(decide("crates.io:22"), Verdict::Refused(Refusal::Unlisted));
        assert_eq!(
            RelayPolicy::default().decide(&"crates.io:443".parse().unwrap()),
            Verdict::Refused(Refusal::Unlisted),
            "an empty policy reaches nothing"
        );
    }

    #[test]
    fn a_name_reaches_only_public_addresses() {
        let destination: Destination = "crates.io:443".parse().unwrap();
        let target: Target = "crates.io:443".parse().unwrap();
        let reach = |resolved: &[&str]| {
            Reachable::of(
                &destination,
                &target,
                &resolved.iter().map(|text| ip(text)).collect::<Vec<_>>(),
            )
            .map(|all| all.into_iter().map(Reachable::address).collect::<Vec<_>>())
        };
        assert_eq!(reach(&["1.1.1.1"]), Ok(vec![ip("1.1.1.1")]));
        assert_eq!(
            reach(&["127.0.0.1", "1.1.1.1"]),
            Ok(vec![ip("1.1.1.1")]),
            "a private address among public ones is skipped"
        );
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "100.101.102.103",
            "169.254.169.254",
            "192.168.1.1",
            "::1",
            "fd7a:115c:a1e0::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert_eq!(reach(&[private]), Err(Refusal::PrivateAddress), "{private}");
        }
        assert_eq!(
            reach(&[]),
            Ok(Vec::new()),
            "nothing resolved is unreachable, not refused"
        );
    }

    #[test]
    fn a_listed_address_is_reached_as_listed() {
        let destination: Destination = "127.0.0.1:4000".parse().unwrap();
        let target: Target = "127.0.0.1:4000".parse().unwrap();
        let reached = Reachable::of(&destination, &target, &[]).unwrap();
        assert_eq!(reached[0].address(), ip("127.0.0.1"));
    }

    #[test]
    fn public_addresses_are_the_internets() {
        for public in ["1.1.1.1", "140.82.112.3", "2606:4700::1111"] {
            assert!(PublicAddress::try_from(ip(public)).is_ok(), "{public}");
        }
        for not in [
            "0.0.0.0",
            "255.255.255.255",
            "240.0.0.1",
            "198.18.0.1",
            "2001:db8::1",
        ] {
            assert!(PublicAddress::try_from(ip(not)).is_err(), "{not}");
        }
        assert_eq!(Refusal::PrivateAddress.label(), "private_address");
    }
}
