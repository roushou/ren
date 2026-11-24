//! One entry of a manifest's `reach`: where the project's relayed runs may
//! connect, as `host:port` or `*.parent:port`.

use std::{fmt, str::FromStr};

/// A `reach` entry: a host name, or `*.` and the name every host under it
/// shares, and a port. Lowercase, without a trailing dot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachEntry {
    /// The host, with its `*.` when it names every host under a parent.
    host: String,
    port: u16,
}

/// Why a `reach` entry does not read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not `host:port` or `*.parent:port`")]
pub struct NotAReachEntry(pub String);

impl ReachEntry {
    const MAX_HOST: usize = 253;
    const MAX_LABEL: usize = 63;

    pub fn port(&self) -> u16 {
        self.port
    }

    fn is_label(label: &str) -> bool {
        !label.is_empty()
            && label.len() <= Self::MAX_LABEL
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
            })
    }
}

impl FromStr for ReachEntry {
    type Err = NotAReachEntry;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refused = || NotAReachEntry(text.to_owned());
        let (host, port) = text.rsplit_once(':').ok_or_else(refused)?;
        let port = port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(refused)?;
        let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
        let name = host.strip_prefix("*.").unwrap_or(&host);
        if name.len() > Self::MAX_HOST || !name.split('.').all(Self::is_label) {
            return Err(refused());
        }
        Ok(Self { host, port })
    }
}

impl fmt::Display for ReachEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::ReachEntry;

    #[test]
    fn an_entry_is_a_host_or_a_parents_hosts_and_a_port() {
        for (text, read) in [
            ("registry.npmjs.org:443", "registry.npmjs.org:443"),
            ("*.Example.com.:443", "*.example.com:443"),
            ("crates_io.example:8443", "crates_io.example:8443"),
        ] {
            assert_eq!(text.parse::<ReachEntry>().unwrap().to_string(), read);
        }
        for text in [
            "crates.io",
            "crates.io:0",
            "crates.io:https",
            "https://crates.io:443",
            "*.:443",
            "-bad.io:443",
            ":443",
            "a b.io:443",
        ] {
            assert!(text.parse::<ReachEntry>().is_err(), "{text}");
        }
    }
}
