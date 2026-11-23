//! A holder's [`Ticket`]: the secret its proxy URL carries, by which the
//! relay knows whose tunnel it is.

use std::{fmt, net::SocketAddr};

/// A secret one holder presents as the password of its proxy URL. It is
/// 16 to 128 of `A-Z`, `a-z`, `0-9`, `-`, and `_`, so it needs no escaping
/// in a URL, and it never appears in `Debug` output.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Ticket(Box<str>);

/// Why a ticket was refused. The text is never echoed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a ticket is 16 to 128 of A-Z, a-z, 0-9, -, and _")]
pub struct InvalidTicket;

impl Ticket {
    const MIN: usize = 16;
    const MAX: usize = 128;

    pub fn new(secret: impl Into<String>) -> Result<Self, InvalidTicket> {
        let secret = secret.into();
        let reads = (Self::MIN..=Self::MAX).contains(&secret.len())
            && secret
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        if reads {
            Ok(Self(secret.into()))
        } else {
            Err(InvalidTicket)
        }
    }

    /// The proxy URL a holder named `user` is given for the relay at
    /// `relay`: `http://<user>:<ticket>@<relay>`. The user name is only a
    /// label; the relay reads the ticket.
    pub fn proxy_url(&self, user: &str, relay: SocketAddr) -> String {
        let user: String = user
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        format!("http://{user}:{}@{relay}", self.0)
    }
}

impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Ticket(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::Ticket;

    #[test]
    fn a_ticket_is_url_safe_and_never_shown() {
        let ticket = Ticket::new("0123456789abcdef-_XYZ").unwrap();
        assert_eq!(format!("{ticket:?}"), "Ticket(<redacted>)");
        assert_eq!(
            ticket.proxy_url("run 7/x", "10.88.0.1:3128".parse().unwrap()),
            "http://run7x:0123456789abcdef-_XYZ@10.88.0.1:3128"
        );
        for bad in [
            "short",
            "has a space in it, so no",
            &"x".repeat(129),
            "with:colon_0123456789",
        ] {
            assert!(Ticket::new(bad).is_err(), "{bad}");
        }
    }
}
