//! The commits the operator chose to preview: each wants a deployment.

use ren_git::Commit;
use serde::{Deserialize, Serialize};

/// The commits the operator asked to preview, the newest ask first. The
/// gateway records an ask or a withdrawal (`ren preview`, the UI); the
/// factory's next pass reads it, and each one wants a preview until it is
/// withdrawn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChosenCommits {
    commits: Vec<Commit>,
}

impl ChosenCommits {
    /// Its file in production's data folder.
    pub const FILE: &str = "preview-requests.json";

    /// The asks, newest first.
    pub fn commits(&self) -> &[Commit] {
        &self.commits
    }

    /// Asks for `commit`, first; one asked before moves to the front.
    pub fn ask(&mut self, commit: Commit) {
        self.commits.retain(|asked| asked != &commit);
        self.commits.insert(0, commit);
    }

    /// Withdraws the ask for `commit`; whether there was one.
    pub fn withdraw(&mut self, commit: &Commit) -> bool {
        let before = self.commits.len();
        self.commits.retain(|asked| asked != commit);
        self.commits.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::ChosenCommits;
    use ren_git::Commit;

    fn commit(digit: char) -> Commit {
        digit.to_string().repeat(40).parse().unwrap()
    }

    #[test]
    fn the_newest_ask_comes_first_and_once() {
        let mut requests = ChosenCommits::default();
        requests.ask(commit('a'));
        requests.ask(commit('b'));
        requests.ask(commit('a'));
        assert_eq!(requests.commits(), [commit('a'), commit('b')]);
        assert!(requests.withdraw(&commit('a')));
        assert!(!requests.withdraw(&commit('a')));
        assert_eq!(requests.commits(), [commit('b')]);
    }
}
