//! The instances of Ren the factory runs (docs/plans/instances.md, phase
//! 1), pure: [`Instances`], each in its slot a build of one commit started
//! on a copy of production's history, and the one step that moves them on
//! ([`Instances::step`], from an [`Event`] to the [`Action`]s that follow).
//! Today they are the previews (docs/plans/self-build.md, phase 9): each
//! open pull request wants an instance of its head, and each commit the
//! operator asks for ([`CommitAsks`]) wants one of it ([`Wanted`]); the
//! first wants, as many as the capacity allows, hold the slots. The factory
//! carries out the actions and reports back ([`Event::Started`],
//! [`Event::Failed`]). A failed build is not tried again for the same
//! commit; a build whose outcome is unknown (its pass stopped) is started
//! again. Pull requests' previews follow them, as the GitHub watch sees
//! them move (`crate::factory::Follower`).

mod asks;
mod instances;
mod slot;

pub use asks::CommitAsks;
pub use instances::{
    Action, Event, Instance, InstanceState, Instances, Reach, Source, StaleReport, Wanted,
};
pub use slot::Slot;
