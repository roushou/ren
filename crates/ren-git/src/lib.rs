//! Git as Ren reads and runs it, with [`Git`] as its primary type: the git
//! program, found on a search path, and the commands Ren runs with it. A
//! folder's repository is read from disk without the program
//! ([`Git::repository`]). This is the one place Ren reads or runs git.

mod error;
mod git;
mod repository;

pub use error::{Exit, GitError};
pub use git::{Author, BranchRemoval, Git, Published, Tree};
pub use repository::{
    Branch, CommitId, Head, NotABranch, NotACommitId, NotARevision, Repository, Revision,
};

#[cfg(test)]
mod tests;
