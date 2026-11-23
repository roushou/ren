//! The network Ren is reached through, and what it vouches for: which
//! login a request comes from.

pub mod tailscale;

/// A network that carries requests to Ren's listeners and names the login
/// each one comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    /// A tailnet, with Tailscale Serve in front of the listeners.
    Tailscale,
}

impl Network {
    /// The request header in which the network names the caller's login.
    pub const fn login_header(self) -> &'static str {
        match self {
            Self::Tailscale => "tailscale-user-login",
        }
    }
}
