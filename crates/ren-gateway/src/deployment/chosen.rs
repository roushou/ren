//! The commits the operator chose to deploy: each wants a deployment, on
//! the data its seed says, recorded on the work items it previews.

use super::Seed;
use ren_git::Commit;
use ren_recorder::id::WorkItemId;
use serde::{Deserialize, Serialize};

/// The commits the operator chose to deploy, the newest first. The gateway
/// records a choice or a withdrawal (`ren deploy`, the UI); the deployer's
/// next pass reads it, and each one wants a deployment until it is
/// withdrawn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChosenCommits {
    commits: Vec<Chosen>,
}

/// One chosen commit: what its deployment's data starts as, and the work
/// items whose change it previews.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "ChosenEntry", into = "ChosenEntry")]
pub struct Chosen {
    pub commit: Commit,
    pub seed: Seed,
    pub work: Vec<WorkItemId>,
}

/// A chosen commit as the file keeps it: its bare commit when it is a copy
/// of production for no work item, as files written before seeds hold
/// every one, and in full otherwise.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum ChosenEntry {
    Commit(Commit),
    Full {
        commit: Commit,
        #[serde(default)]
        seed: Seed,
        #[serde(default)]
        work: Vec<WorkItemId>,
    },
}

impl From<ChosenEntry> for Chosen {
    fn from(entry: ChosenEntry) -> Self {
        match entry {
            ChosenEntry::Commit(commit) => Self::copy(commit),
            ChosenEntry::Full { commit, seed, work } => Self { commit, seed, work },
        }
    }
}

impl From<Chosen> for ChosenEntry {
    fn from(chosen: Chosen) -> Self {
        if chosen.seed.is_copy() && chosen.work.is_empty() {
            Self::Commit(chosen.commit)
        } else {
            Self::Full {
                commit: chosen.commit,
                seed: chosen.seed,
                work: chosen.work,
            }
        }
    }
}

impl Chosen {
    /// `commit` on a copy of production, for no work item.
    pub fn copy(commit: Commit) -> Self {
        Self {
            commit,
            seed: Seed::Copy,
            work: Vec::new(),
        }
    }
}

impl ChosenCommits {
    /// Its file in production's data folder.
    pub const FILE: &str = "preview-requests.json";

    /// The choices, newest first.
    pub fn chosen(&self) -> &[Chosen] {
        &self.commits
    }

    /// Chooses `chosen`, first; a commit chosen before moves to the front
    /// with what it is chosen with now.
    pub fn choose(&mut self, chosen: Chosen) {
        self.commits.retain(|kept| kept.commit != chosen.commit);
        self.commits.insert(0, chosen);
    }

    /// Withdraws the choice of `commit`; whether there was one.
    pub fn withdraw(&mut self, commit: &Commit) -> bool {
        let before = self.commits.len();
        self.commits.retain(|kept| &kept.commit != commit);
        self.commits.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::{Chosen, ChosenCommits};
    use crate::deployment::{Seed, SeedNumber};
    use ren_git::Commit;

    fn commit(digit: char) -> Commit {
        digit.to_string().repeat(40).parse().unwrap()
    }

    #[test]
    fn the_newest_choice_comes_first_and_once() {
        let mut chosen = ChosenCommits::default();
        chosen.choose(Chosen::copy(commit('a')));
        chosen.choose(Chosen::copy(commit('b')));
        chosen.choose(Chosen {
            seed: Seed::Invented(SeedNumber::new(3)),
            ..Chosen::copy(commit('a'))
        });
        let commits: Vec<_> = chosen
            .chosen()
            .iter()
            .map(|kept| kept.commit.clone())
            .collect();
        assert_eq!(commits, [commit('a'), commit('b')]);
        assert_eq!(chosen.chosen()[0].seed, Seed::Invented(SeedNumber::new(3)));
        assert!(chosen.withdraw(&commit('a')));
        assert!(!chosen.withdraw(&commit('a')));
    }

    #[test]
    fn a_file_of_bare_commits_reads_and_a_copy_is_written_bare() {
        // Written before seeds: every choice a copy of production.
        let old = serde_json::json!({ "commits": ["a".repeat(40)] });
        let read: ChosenCommits = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(read.chosen(), [Chosen::copy(commit('a'))]);
        assert_eq!(serde_json::to_value(&read).unwrap(), old);
        let mut seeded = ChosenCommits::default();
        seeded.choose(Chosen {
            seed: Seed::Invented(SeedNumber::new(7)),
            work: vec!["00000000-0000-0000-0000-000000000001".parse().unwrap()],
            ..Chosen::copy(commit('b'))
        });
        let json = serde_json::to_value(&seeded).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "commits": [{ "commit": "b".repeat(40), "seed": { "invented": 7 },
                "work": ["00000000-0000-0000-0000-000000000001"] }] })
        );
        assert_eq!(
            serde_json::from_value::<ChosenCommits>(json).unwrap(),
            seeded
        );
    }
}
