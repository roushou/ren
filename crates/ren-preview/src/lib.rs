//! Which previews of Ren run (docs/plans/self-build.md, phase 9), pure:
//! [`Previews`], the previews in their slots, each a build of one commit
//! started on a copy of production's history, and what to do next. Each
//! open pull request wants a preview of its head, and each commit the
//! operator asks for wants one of it ([`Wanted`]); the first wants, as many
//! as the capacity allows, hold the slots. [`Previews::want`] says which
//! slots to stop and which commits to start ([`Action`]); the factory
//! carries them out and reports back ([`Previews::started`],
//! [`Previews::failed`]). The commits the operator asked for are
//! [`PreviewRequests`]. A failed build is not tried again for the same
//! commit, and a build a pass left unfinished is started again.

mod previews;
mod requests;
mod slot;

#[cfg(test)]
mod purity;

pub use previews::{
    Action, Preview, PreviewKey, PreviewState, Previews, Reach, StaleReport, Wanted,
};
pub use requests::PreviewRequests;
pub use slot::Slot;
