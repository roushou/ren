//! Where a tunnel goes ([`Target`]) and where a policy lets one go
//! ([`Destination`]), each parsed once from its text.

use std::{
    fmt,
    net::{IpAddr, Ipv6Addr},
    str::FromStr,
};

/// A host name: lowercase, without a trailing dot, each label one to 63 of
/// `a-z`, `0-9`, `-`, and `_`, not starting or ending with `-`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostName(Box<str>);

/// Why a host name does not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a host name")]
pub struct NotAHostName;

impl HostName {
    const MAX: usize = 253;
    const MAX_LABEL: usize = 63;

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this name is `parent` or a name under it.
    fn is_under(&self, parent: &Self) -> bool {
        self.0
            .strip_suffix(parent.as_str())
            .is_some_and(|rest| rest.ends_with('.'))
    }
}

impl FromStr for HostName {
    type Err = NotAHostName;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let name = text.strip_suffix('.').unwrap_or(text).to_ascii_lowercase();
        let label_reads = |label: &str| {
            (1..=Self::MAX_LABEL).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        };
        // A name of digits and dots alone reads as an address, never a name.
        let numeric = name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.');
        if name.is_empty() || name.len() > Self::MAX || numeric || !name.split('.').all(label_reads)
        {
            return Err(NotAHostName);
        }
        Ok(Self(name.into()))
    }
}

impl fmt::Display for HostName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A host as a run names it: a name, or an address written as one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Host {
    Name(HostName),
    Address(IpAddr),
}

impl Host {
    /// `text`, an address in brackets for IPv6 (`[::1]`) or without, or a
    /// name.
    fn read(text: &str) -> Option<Self> {
        if let Some(inner) = text
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            return inner
                .parse::<Ipv6Addr>()
                .ok()
                .map(|v6| Self::Address(v6.into()));
        }
        match text.parse::<IpAddr>() {
            Ok(IpAddr::V4(v4)) => Some(Self::Address(v4.into())),
            // An IPv6 address names a port only in brackets.
            Ok(IpAddr::V6(_)) => None,
            Err(_) => text.parse().ok().map(Self::Name),
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => name.fmt(f),
            Self::Address(IpAddr::V4(v4)) => v4.fmt(f),
            Self::Address(IpAddr::V6(v6)) => write!(f, "[{v6}]"),
        }
    }
}

/// Splits `host:port`, the host's own colons inside brackets.
fn split_port(text: &str) -> Option<(&str, u16)> {
    let (host, port) = text.rsplit_once(':')?;
    let port = port.parse::<u16>().ok().filter(|port| *port != 0)?;
    Some((host, port))
}

/// Where a tunnel goes: the authority of a `CONNECT`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target {
    host: Host,
    port: u16,
}

/// Why a target does not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a host and port")]
pub struct NotATarget;

impl Target {
    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl FromStr for Target {
    type Err = NotATarget;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (host, port) = split_port(text).ok_or(NotATarget)?;
        let host = Host::read(host).ok_or(NotATarget)?;
        Ok(Self { host, port })
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// The hosts a destination lets a run reach.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HostPattern {
    /// This host alone.
    Exact(Host),
    /// Any name under this one, not the name itself: `*.example.com`.
    Under(HostName),
}

impl HostPattern {
    fn matches(&self, host: &Host) -> bool {
        match (self, host) {
            (Self::Exact(listed), host) => listed == host,
            (Self::Under(parent), Host::Name(name)) => name.is_under(parent),
            (Self::Under(_), Host::Address(_)) => false,
        }
    }
}

impl fmt::Display for HostPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(host) => host.fmt(f),
            Self::Under(parent) => write!(f, "*.{parent}"),
        }
    }
}

/// Where a policy lets a run connect: hosts and one port, written
/// `host:port` or `*.parent:port`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Destination {
    host: HostPattern,
    port: u16,
}

/// Why a destination does not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a destination: write host:port or *.parent:port")]
pub struct NotADestination;

impl Destination {
    pub fn new(host: HostPattern, port: u16) -> Self {
        Self { host, port }
    }

    pub fn host(&self) -> &HostPattern {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Whether this destination lets a run reach `target`.
    pub(crate) fn admits(&self, target: &Target) -> bool {
        self.port == target.port && self.host.matches(&target.host)
    }

    /// Whether `target` reaches this destination by the address it lists,
    /// so its address is the one listed, not one a name resolved to.
    pub(crate) fn lists_address(&self) -> bool {
        matches!(self.host, HostPattern::Exact(Host::Address(_)))
    }
}

impl FromStr for Destination {
    type Err = NotADestination;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (host, port) = split_port(text).ok_or(NotADestination)?;
        let host = match host.strip_prefix("*.") {
            Some(parent) => HostPattern::Under(parent.parse().map_err(|_| NotADestination)?),
            None => HostPattern::Exact(Host::read(host).ok_or(NotADestination)?),
        };
        Ok(Self { host, port })
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::{Destination, Host, HostName, Target};

    fn target(text: &str) -> Target {
        text.parse().unwrap()
    }

    #[test]
    fn targets_read_names_addresses_and_ports() {
        let named = target("Index.Crates.IO.:443");
        assert_eq!(named.to_string(), "index.crates.io:443");
        assert_eq!(target("10.0.0.1:22").to_string(), "10.0.0.1:22");
        assert_eq!(target("[::1]:8080").to_string(), "[::1]:8080");
        for bad in [
            "crates.io",
            "crates.io:0",
            "crates.io:70000",
            "::1:443",
            ":443",
            "-bad.io:443",
            "a..b:443",
            "sp ace.io:443",
            "1.2.3:443",
        ] {
            assert!(bad.parse::<Target>().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_destination_admits_its_hosts_on_its_port() {
        let exact: Destination = "crates.io:443".parse().unwrap();
        assert!(exact.admits(&target("crates.io:443")));
        assert!(!exact.admits(&target("crates.io:80")));
        assert!(!exact.admits(&target("static.crates.io:443")));

        let under: Destination = "*.crates.io:443".parse().unwrap();
        assert!(under.admits(&target("static.crates.io:443")));
        assert!(under.admits(&target("a.b.crates.io:443")));
        assert!(!under.admits(&target("crates.io:443")));
        assert!(!under.admits(&target("evilcrates.io:443")));
        assert_eq!(under.to_string(), "*.crates.io:443");

        let address: Destination = "127.0.0.1:4000".parse().unwrap();
        assert!(address.admits(&target("127.0.0.1:4000")));
        assert!(address.lists_address() && !exact.lists_address());
        assert!("*.10.0.0.1:443".parse::<Destination>().is_err());
        assert!("*.crates.io".parse::<Destination>().is_err());
    }

    #[test]
    fn host_names_are_lowercase_without_a_trailing_dot() {
        let name: HostName = "GitHub.com.".parse().unwrap();
        assert_eq!(name.as_str(), "github.com");
        assert_eq!(
            Host::Name(name).to_string(),
            "github.com",
            "a name displays as it reads"
        );
        assert!("".parse::<HostName>().is_err());
        assert!(".".parse::<HostName>().is_err());
        assert!(
            format!("{}.io", "a".repeat(64))
                .parse::<HostName>()
                .is_err()
        );
    }
}
