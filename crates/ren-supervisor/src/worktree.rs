//! A worktree Ren owns, pure: one of a repository's worktrees, made for one
//! session in the worktrees folder on a branch of its own, so sessions
//! working on one repository never share a checkout. Each has a slot no
//! other worktree holds while it exists: a block of ports and a data
//! folder, given to its runs as `REN_WORKTREE`, `REN_WORKTREE_PORTS`, and
//! `REN_WORKTREE_DATA`, so what they start (a test server, a preview)
//! cannot collide.
//!
//! A worktree's folder is `<worktrees>/<repo>-<slot>`; its data folder is
//! `<worktrees>/.data/<repo>-<slot>`; its branch is `ren/<repo>-<slot>-<id>`,
//! unique even when a slot is used again.

use std::path::{Path, PathBuf};

/// One worktree: its folder in the worktrees folder, and its slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    root: PathBuf,
    name: String,
    slot: Slot,
}

/// A worktree's place among the worktrees that exist: its ports and its
/// data folder are its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Slot(u16);

impl Slot {
    /// The ports each slot holds.
    const PORTS: u16 = 100;
    /// Slot 1's first port; below it are the ports Ren and its dev slot use.
    const FIRST_PORT: u16 = 20_000;
    /// The most slots: their ports end below 65535.
    const MOST: u16 = 450;

    /// The lowest slot none of `taken` holds, if any is left.
    pub fn free(taken: &[Slot]) -> Option<Self> {
        (1..=Self::MOST)
            .map(Self)
            .find(|slot| !taken.contains(slot))
    }

    /// Its ports, first and last.
    pub fn ports(self) -> (u16, u16) {
        let first = Self::FIRST_PORT + (self.0 - 1) * Self::PORTS;
        (first, first + Self::PORTS - 1)
    }
}

impl Worktree {
    /// The folder worktrees' data folders are kept in.
    const DATA: &str = ".data";

    /// The worktree of `repo` in `slot`, under `root`.
    pub fn new(root: &Path, repo: &Path, slot: Slot) -> Option<Self> {
        let repo = repo.file_name()?.to_str()?;
        Some(Self {
            root: root.to_path_buf(),
            name: format!("{repo}-{}", slot.0),
            slot,
        })
    }

    /// The worktree `workdir` is, when it is a folder of `root`'s named as
    /// one.
    pub fn of(root: &Path, workdir: &Path) -> Option<Self> {
        if workdir.parent()? != root {
            return None;
        }
        let name = workdir.file_name()?.to_str()?;
        Self::slot_of(name).map(|slot| Self {
            root: root.to_path_buf(),
            name: name.to_owned(),
            slot,
        })
    }

    /// The slots the folders named `names` hold.
    pub fn taken<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<Slot> {
        names.into_iter().filter_map(Self::slot_of).collect()
    }

    fn slot_of(name: &str) -> Option<Slot> {
        let (repo, slot) = name.rsplit_once('-')?;
        let slot: u16 = slot.parse().ok()?;
        (!repo.is_empty() && !repo.starts_with('.') && (1..=Slot::MOST).contains(&slot))
            .then_some(Slot(slot))
    }

    /// Whether it is a worktree of the repository in `repo`: its name is
    /// that folder's name and a slot.
    pub fn is_of(&self, repo: &Path) -> bool {
        repo.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| self.name == format!("{name}-{}", self.slot.0))
    }

    /// `<repo>-<slot>`, its folder's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn folder(&self) -> PathBuf {
        self.root.join(&self.name)
    }

    pub fn data(&self) -> PathBuf {
        self.root.join(Self::DATA).join(&self.name)
    }

    /// Its branch, made unique by `id`.
    pub fn branch(&self, id: &str) -> String {
        format!("ren/{}-{id}", self.name)
    }

    /// Its slot's ports, first and last.
    pub fn ports(&self) -> (u16, u16) {
        self.slot.ports()
    }

    /// The Podman pod Ren's own preview runs in (`cargo xtask preview`),
    /// removed when the worktree is.
    pub fn preview_pod(&self) -> String {
        format!("ren-preview-{}", self.name)
    }

    /// The transient unit a project's preview runs as (`ren project
    /// preview`: its serve step), stopped when the worktree is removed.
    pub fn preview_unit(&self) -> String {
        format!("ren-preview-{}-serve", self.name)
    }

    /// What its runs are told of it: `REN_WORKTREE*`, and the same values
    /// under the old `REN_WORKSPACE*` names, kept for project scripts
    /// written before the rename.
    pub fn env(&self) -> Vec<(&'static str, String)> {
        let (first, last) = self.slot.ports();
        let (folder, ports, data) = (
            self.folder().display().to_string(),
            format!("{first}-{last}"),
            self.data().display().to_string(),
        );
        vec![
            ("REN_WORKTREE", folder.clone()),
            ("REN_WORKTREE_PORTS", ports.clone()),
            ("REN_WORKTREE_DATA", data.clone()),
            ("REN_WORKSPACE", folder),
            ("REN_WORKSPACE_PORTS", ports),
            ("REN_WORKSPACE_DATA", data),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{Slot, Worktree};
    use std::path::Path;

    #[test]
    fn a_worktree_is_named_for_its_repository_and_slot() {
        let root = Path::new("/data/worktrees");
        let worktree = Worktree::new(root, Path::new("/home/op/dev/ren"), Slot(3)).unwrap();
        assert_eq!(worktree.folder(), Path::new("/data/worktrees/ren-3"));
        assert_eq!(worktree.data(), Path::new("/data/worktrees/.data/ren-3"));
        assert_eq!(worktree.branch("1a2b3c4d"), "ren/ren-3-1a2b3c4d");
        assert_eq!(worktree.preview_pod(), "ren-preview-ren-3");
        assert_eq!(worktree.preview_unit(), "ren-preview-ren-3-serve");
        assert_eq!(
            worktree.env(),
            [
                ("REN_WORKTREE", "/data/worktrees/ren-3".to_owned()),
                ("REN_WORKTREE_PORTS", "20200-20299".to_owned()),
                (
                    "REN_WORKTREE_DATA",
                    "/data/worktrees/.data/ren-3".to_owned()
                ),
                ("REN_WORKSPACE", "/data/worktrees/ren-3".to_owned()),
                ("REN_WORKSPACE_PORTS", "20200-20299".to_owned()),
                (
                    "REN_WORKSPACE_DATA",
                    "/data/worktrees/.data/ren-3".to_owned()
                ),
            ]
        );
        assert_eq!(
            Worktree::of(root, Path::new("/data/worktrees/ren-3")),
            Some(worktree)
        );
        for other in [
            "/data/worktrees/.data",
            "/data/worktrees/ren",
            "/data/scratch/ren-3",
            "/data/worktrees/ren-0",
            "/data/worktrees/ren-451",
        ] {
            assert_eq!(Worktree::of(root, Path::new(other)), None, "{other}");
        }
    }

    #[test]
    fn a_slot_is_the_lowest_free_and_ports_never_overlap() {
        let taken = Worktree::taken(["ren-1", "ren-2", "acme-web-4", ".data", "notes"]);
        assert_eq!(taken, [Slot(1), Slot(2), Slot(4)]);
        assert_eq!(Slot::free(&taken), Some(Slot(3)));
        assert_eq!(Slot(1).ports(), (20_000, 20_099));
        assert_eq!(Slot(450).ports(), (64_900, 64_999));
        let all: Vec<_> = (1..=450).map(Slot).collect();
        assert_eq!(Slot::free(&all), None);
    }
}
