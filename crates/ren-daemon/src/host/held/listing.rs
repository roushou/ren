//! Where a relayed run may go, and why each place is listed: Ren's
//! configuration, the run's client, its project's manifest, or its grant.
//! A run's [`Listing`] makes the policy its pass holds, travels with the
//! pass as its holder ([`RelayedRun`]), and says of each passage the relay
//! reports which list let it through. Pure: what each list holds is handed
//! to it.

use ren_github::RepositorySlug;
use ren_recorder::{
    id::RunId,
    passage::{PassageRecord, Purpose},
};
use ren_relay::{Course, Destination, RelayPolicy};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A run's destinations in the order the relay tries them, each with the
/// list it is on. A destination on two lists is admitted, and labelled, by
/// the first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Listing(Vec<Listed>);

/// One destination, and the list it is on.
#[serde_with::serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listed {
    #[serde_as(as = "serde_with::DisplayFromStr")]
    destination: Destination,
    purpose: Purpose,
}

/// Who holds a relayed run's pass: the run, its listing, so a passage
/// reported after the run ended still says why it went through, and the
/// repository it may push to, which its git's ask is answered for.
#[derive(Debug, Clone)]
pub struct RelayedRun {
    run: RunId,
    listing: Arc<Listing>,
    grant: Option<RepositorySlug>,
}

impl Listing {
    /// This listing, then `destinations` on the list `purpose` names.
    #[must_use]
    pub fn list(
        mut self,
        purpose: Purpose,
        destinations: impl IntoIterator<Item = Destination>,
    ) -> Self {
        self.0
            .extend(destinations.into_iter().map(|destination| Listed {
                destination,
                purpose,
            }));
        self
    }

    /// What the relay admits: every destination, in order.
    pub fn policy(&self) -> RelayPolicy {
        RelayPolicy::new(self.0.iter().map(|listed| listed.destination.clone()))
    }

    /// Why `through` is listed: the first list it is on.
    pub fn purpose(&self, through: &Destination) -> Option<Purpose> {
        self.0
            .iter()
            .find(|listed| &listed.destination == through)
            .map(|listed| listed.purpose)
    }
}

impl RelayedRun {
    pub fn new(run: RunId, listing: Listing, grant: Option<RepositorySlug>) -> Self {
        Self {
            run,
            listing: Arc::new(listing),
            grant,
        }
    }

    pub fn run(&self) -> RunId {
        self.run
    }

    /// The repository the run may push to, when it is granted one.
    pub fn grant(&self) -> Option<&RepositorySlug> {
        self.grant.as_ref()
    }

    /// The record of `course`, one of this run's passages.
    pub fn record(&self, course: &Course) -> PassageRecord {
        PassageRecord::of(course, |through| self.listing.purpose(through))
    }
}

#[cfg(test)]
mod tests {
    use super::{Listing, RelayedRun};
    use ren_recorder::{id::RunId, passage::Purpose};
    use ren_relay::{Course, Destination, TunnelEnd};

    fn destination(text: &str) -> Destination {
        text.parse().unwrap()
    }

    fn listing() -> Listing {
        Listing::default()
            .list(Purpose::Configuration, [destination("*.crates.io:443")])
            .list(Purpose::Client, [destination("auth.openai.com:443")])
            .list(
                Purpose::Project,
                [
                    destination("registry.npmjs.org:443"),
                    destination("*.crates.io:443"),
                ],
            )
            .list(Purpose::Grant, [destination("github.com:443")])
    }

    #[test]
    fn the_relay_admits_every_listed_destination_in_order() {
        assert_eq!(
            listing()
                .policy()
                .destinations()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            [
                "*.crates.io:443",
                "auth.openai.com:443",
                "registry.npmjs.org:443",
                "*.crates.io:443",
                "github.com:443",
            ]
        );
    }

    #[test]
    fn a_passage_says_which_list_let_it_through_the_first_one_on_two() {
        let run = RelayedRun::new(RunId::restore(uuid::Uuid::nil()), listing(), None);
        let through = |target: &str, through: &str| Course::Tunnel {
            target: target.parse().unwrap(),
            end: TunnelEnd::Broken {
                through: destination(through),
            },
        };
        for (target, via, purpose) in [
            (
                "index.crates.io:443",
                "*.crates.io:443",
                Purpose::Configuration,
            ),
            (
                "auth.openai.com:443",
                "auth.openai.com:443",
                Purpose::Client,
            ),
            (
                "registry.npmjs.org:443",
                "registry.npmjs.org:443",
                Purpose::Project,
            ),
            ("github.com:443", "github.com:443", Purpose::Grant),
        ] {
            assert_eq!(run.record(&through(target, via)).listed_by(), Some(purpose));
        }
        assert_eq!(
            run.record(&through("elsewhere.dev:443", "elsewhere.dev:443"))
                .listed_by(),
            None
        );
    }

    #[test]
    fn a_listing_is_kept_in_a_runs_spec_as_it_reads() {
        let json = serde_json::to_value(listing()).unwrap();
        assert_eq!(
            json[2],
            serde_json::json!({ "destination": "registry.npmjs.org:443", "purpose": "project" })
        );
        assert_eq!(serde_json::from_value::<Listing>(json).unwrap(), listing());
    }
}
