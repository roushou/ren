//! A proxy request's head, read from the bytes a client sends
//! ([`Head::read`]), and the [`Answer`] the relay sends back. Pure: it
//! reads bytes and writes bytes, and never touches a socket.

use crate::{policy::Refusal, ticket::Ticket};
use base64::Engine as _;
use ren_destination::Target;

/// What a client asked the relay for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// A tunnel to this target (`CONNECT host:port`).
    Tunnel(Target),
    /// Anything else, which the relay refuses: plain HTTP through a proxy
    /// would let the relay read and change what a run sends.
    Other { method: Box<str> },
}

/// A request's head: what it asks for, the ticket it presents, and how
/// many bytes the head took (what follows is the tunnel's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    ask: Ask,
    ticket: Option<Ticket>,
    len: usize,
}

/// What the bytes read so far are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// Not a whole head yet: read more.
    Incomplete,
    Complete(Head),
    Invalid(HeadError),
}

/// Why a request's head does not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HeadError {
    #[error("the request head is longer than {} bytes", Head::LIMIT)]
    TooLong,
    #[error("the request head is not text")]
    NotText,
    #[error("the request line is not `METHOD target HTTP/1.x`")]
    RequestLine,
    #[error("the tunnel's target is not a host and port")]
    Target,
}

impl Head {
    /// The most a head may take, its blank line included.
    pub const LIMIT: usize = 8192;
    const END: &[u8] = b"\r\n\r\n";

    pub fn ask(&self) -> &Ask {
        &self.ask
    }

    /// The ticket its `Proxy-Authorization` presents (`Basic`, the
    /// password), if it presents one that reads.
    pub fn ticket(&self) -> Option<&Ticket> {
        self.ticket.as_ref()
    }

    /// How many of the bytes read the head took.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the head took no bytes, which a whole head never does.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The head at the start of `bytes`, everything the client sent so far.
    pub fn read(bytes: &[u8]) -> Reading {
        let Some(end) = bytes
            .windows(Self::END.len())
            .position(|window| window == Self::END)
        else {
            return if bytes.len() >= Self::LIMIT {
                Reading::Invalid(HeadError::TooLong)
            } else {
                Reading::Incomplete
            };
        };
        let len = end + Self::END.len();
        if len > Self::LIMIT {
            return Reading::Invalid(HeadError::TooLong);
        }
        let Ok(text) = std::str::from_utf8(&bytes[..end]) else {
            return Reading::Invalid(HeadError::NotText);
        };
        let mut lines = text.split("\r\n");
        let line = lines.next().unwrap_or_default();
        let mut parts = line.split(' ');
        let (Some(method), Some(target), Some(version), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Reading::Invalid(HeadError::RequestLine);
        };
        if method.is_empty() || !version.starts_with("HTTP/1.") {
            return Reading::Invalid(HeadError::RequestLine);
        }
        let ask = if method == "CONNECT" {
            match target.parse() {
                Ok(target) => Ask::Tunnel(target),
                Err(_) => return Reading::Invalid(HeadError::Target),
            }
        } else {
            Ask::Other {
                method: method.into(),
            }
        };
        let ticket = lines
            .filter_map(|header| header.split_once(':'))
            .filter(|(name, _)| name.trim().eq_ignore_ascii_case("proxy-authorization"))
            .find_map(|(_, value)| Self::basic_password(value.trim()));
        Reading::Complete(Self { ask, ticket, len })
    }

    /// The password of a `Basic` credential, as a ticket.
    fn basic_password(value: &str) -> Option<Ticket> {
        let (scheme, encoded) = value.split_once(' ')?;
        if !scheme.eq_ignore_ascii_case("basic") {
            return None;
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .ok()?;
        let decoded = String::from_utf8(decoded).ok()?;
        let (_, password) = decoded.split_once(':')?;
        Ticket::new(password).ok()
    }
}

/// What the relay answers a request's head with, before a tunnel's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// The tunnel is open; its bytes follow.
    Established,
    /// No ticket the relay holds, so it asks for one.
    TicketNeeded,
    Refused(Refusal),
    /// It asked for something other than a tunnel.
    NotATunnel,
    /// Admitted, but its target did not resolve or connect.
    Unreachable,
    BadRequest,
}

impl Answer {
    /// The response's bytes: its status line, headers, and a short body
    /// that names a refusal by its label.
    pub fn bytes(self) -> Vec<u8> {
        let (status, extra, body) = match self {
            Self::Established => ("200 Connection Established", "", ""),
            Self::TicketNeeded => (
                "407 Proxy Authentication Required",
                "Proxy-Authenticate: Basic realm=\"ren\"\r\n",
                "ticket_needed\n",
            ),
            Self::NotATunnel => (
                "405 Method Not Allowed",
                "X-Ren-Refusal: not_a_tunnel\r\n",
                "not_a_tunnel\n",
            ),
            Self::Refused(Refusal::Unlisted) => {
                ("403 Forbidden", "X-Ren-Refusal: unlisted\r\n", "unlisted\n")
            }
            Self::Refused(Refusal::PrivateAddress) => (
                "403 Forbidden",
                "X-Ren-Refusal: private_address\r\n",
                "private_address\n",
            ),
            Self::Unreachable => ("502 Bad Gateway", "", "unreachable\n"),
            Self::BadRequest => ("400 Bad Request", "", "bad_request\n"),
        };
        let mut head = format!("HTTP/1.1 {status}\r\n{extra}");
        if !matches!(self, Self::Established) {
            head.push_str(&format!(
                "Content-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            ));
        }
        head.push_str("\r\n");
        head.push_str(body);
        head.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::{Answer, Ask, Head, HeadError, Reading};
    use crate::{policy::Refusal, ticket::Ticket};
    use proptest::prelude::*;

    /// `run:0123456789abcdef` in Basic.
    const AUTH: &str = "Proxy-Authorization: Basic cnVuOjAxMjM0NTY3ODlhYmNkZWY=";

    fn connect(extra: &str) -> String {
        format!("CONNECT index.crates.io:443 HTTP/1.1\r\nHost: index.crates.io:443\r\n{extra}\r\n")
    }

    fn complete(bytes: &[u8]) -> Head {
        match Head::read(bytes) {
            Reading::Complete(head) => head,
            other @ (Reading::Incomplete | Reading::Invalid(_)) => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_connect_reads_its_target_and_ticket() {
        let text = connect(&format!("{AUTH}\r\n"));
        let mut bytes = text.clone().into_bytes();
        bytes.extend_from_slice(b"\x16\x03\x01tls");
        let head = complete(&bytes);
        assert_eq!(
            head.ask(),
            &Ask::Tunnel("index.crates.io:443".parse().unwrap())
        );
        assert_eq!(
            head.ticket(),
            Some(&Ticket::new("0123456789abcdef").unwrap())
        );
        assert_eq!(
            head.len(),
            text.len(),
            "the tunnel's bytes are not the head's"
        );
    }

    #[test]
    fn a_head_without_a_ticket_that_reads_presents_none() {
        assert_eq!(complete(connect("").as_bytes()).ticket(), None);
        for header in [
            "Proxy-Authorization: Bearer 0123456789abcdef",
            "Proxy-Authorization: Basic !!!",
            // `run:short`: a password that is no ticket.
            "Proxy-Authorization: Basic cnVuOnNob3J0",
        ] {
            let head = complete(connect(&format!("{header}\r\n")).as_bytes());
            assert_eq!(head.ticket(), None, "{header}");
        }
    }

    #[test]
    fn anything_but_connect_is_another_ask() {
        let head = complete(b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n");
        assert_eq!(
            head.ask(),
            &Ask::Other {
                method: "GET".into()
            }
        );
    }

    #[test]
    fn a_head_that_cannot_be_one_is_invalid() {
        let invalid = |bytes: &[u8]| match Head::read(bytes) {
            Reading::Invalid(error) => error,
            other @ (Reading::Incomplete | Reading::Complete(_)) => panic!("{other:?}"),
        };
        assert_eq!(
            invalid(b"CONNECT crates.io HTTP/1.1\r\n\r\n"),
            HeadError::Target
        );
        assert_eq!(
            invalid(b"CONNECT a:1 SPDY/3\r\n\r\n"),
            HeadError::RequestLine
        );
        assert_eq!(
            invalid(b"CONNECT a:1 HTTP/1.1 x\r\n\r\n"),
            HeadError::RequestLine
        );
        assert_eq!(
            invalid(b"\xff\xfe a:1 HTTP/1.1\r\n\r\n"),
            HeadError::NotText
        );
        assert_eq!(invalid(&vec![b'a'; Head::LIMIT]), HeadError::TooLong);
        assert_eq!(Head::read(b"CONNECT a:1 HTTP/1.1\r\n"), Reading::Incomplete);
    }

    #[test]
    fn answers_name_their_refusal() {
        let text = |answer: Answer| String::from_utf8(answer.bytes()).unwrap();
        assert_eq!(
            text(Answer::Established),
            "HTTP/1.1 200 Connection Established\r\n\r\n"
        );
        let refused = text(Answer::Refused(Refusal::Unlisted));
        assert!(refused.starts_with("HTTP/1.1 403 Forbidden\r\n"));
        assert!(refused.contains("X-Ren-Refusal: unlisted\r\n"));
        assert!(refused.ends_with("\r\n\r\nunlisted\n"));
        assert!(text(Answer::TicketNeeded).contains("Proxy-Authenticate: Basic realm=\"ren\"\r\n"));
    }

    proptest! {
        /// However the client's bytes arrive, the head reads the same once
        /// it is whole, and is incomplete before.
        #[test]
        fn a_head_reads_the_same_however_it_is_split(split in 0usize..200) {
            let text = connect(&format!("{AUTH}\r\n"));
            let bytes = text.as_bytes();
            let split = split.min(bytes.len());
            let whole = complete(bytes);
            match Head::read(&bytes[..split]) {
                Reading::Incomplete => prop_assert!(split < bytes.len()),
                Reading::Complete(head) => prop_assert_eq!(head, whole),
                Reading::Invalid(error) => prop_assert!(false, "{error}"),
            }
        }
    }
}
