//! Pi's attribution headers: the session and turn ids Ren's extension sends,
//! read on every route, since Pi reaches Ren through the other clients'.

use crate::attribution::HintSource;
use crate::attribution::headers::{Claim, HeaderReading};

impl HeaderReading<'_> {
    const PI_SESSION: &'static str = "x-ren-pi-session-id";
    const PI_TURN: &'static str = "x-ren-pi-turn-id";

    /// Pi's extension headers, on any route.
    pub(crate) fn pi(&mut self) -> Option<Claim> {
        if !self.headers.contains_key(Self::PI_SESSION) && !self.headers.contains_key(Self::PI_TURN)
        {
            return None;
        }
        Some(Claim {
            source: Some(HintSource::PiExtensionHeaders),
            session: self.uuid(Self::PI_SESSION),
            turn: self.uuid(Self::PI_TURN),
            thread: None,
        })
    }
}
