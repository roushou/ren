//! The deployments of Ren the factory runs (docs/plans/deployments.md):
//! [`Deployments`], each in its slot a build of one commit started
//! on a copy of production's history, and the one step that moves them on
//! ([`Deployments::step`], from an [`Event`] to the [`Action`]s that follow).
//! Today they are the previews (docs/plans/self-build.md, phase 9): each
//! ready pull request wants a deployment of its head
//! ([`Trigger::FollowsPull`]), and each commit the operator chose
//! ([`ChosenCommits`], [`Trigger::Chosen`]) wants one of it ([`Wanted`]); the
//! first wants, as many as the capacity allows, hold the slots. The factory
//! carries out the actions and reports back ([`Event::Started`],
//! [`Event::Failed`]). A failed build is not tried again for the same
//! commit; a build whose outcome is unknown (its pass stopped) is started
//! again. Pull requests' previews follow them, as the GitHub watch sees
//! them move (`crate::factory::Follower`).

mod chosen;
mod deployments;
mod slot;

pub use chosen::ChosenCommits;
pub use deployments::{
    Action, Deployment, DeploymentState, Deployments, Event, Reach, StaleReport, Trigger, Wanted,
};
pub use slot::Slot;
