//! Where a run can start: the projects under the run roots. A folder that
//! holds a git repository is a project, and the search does not go inside
//! it; a folder directly inside a root with no repository below it is a
//! project too, without git. The search is pure: the host looks at each
//! folder (`AgentHost::look`), and this module decides where to look next.

use crate::workdir::Roots;
use std::path::{Path, PathBuf};

/// What the host saw in one folder.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Look {
    /// The names of the folders directly inside it, links not followed.
    pub folders: Vec<String>,
    /// Whether it holds a git repository (`.git`), and the text of its
    /// `HEAD` when that could be read.
    pub git: Option<Option<String>>,
}

/// A place a run can start in.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Project {
    /// The absolute path to start a run in.
    pub path: String,
    /// Its folder's name, with its parent's when two projects share it.
    pub name: String,
    pub git: ProjectGit,
}

/// A project's repository, as its `HEAD` reads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectGit {
    /// On a branch.
    Branch { name: String },
    /// At a commit, not on a branch: its abbreviated id.
    Detached { commit: String },
    /// A repository whose `HEAD` could not be read.
    Unreadable,
    /// No repository.
    None,
}

impl ProjectGit {
    /// What a `HEAD` file says: `ref: refs/heads/<branch>`, or a commit id.
    pub fn read(head: Option<&str>) -> Self {
        let Some(head) = head.map(str::trim) else {
            return Self::Unreadable;
        };
        if let Some(reference) = head.strip_prefix("ref:") {
            let reference = reference.trim();
            return Self::Branch {
                name: reference
                    .strip_prefix("refs/heads/")
                    .unwrap_or(reference)
                    .to_owned(),
            };
        }
        if head.len() >= 7 && head.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Self::Detached {
                commit: head.chars().take(7).collect(),
            };
        }
        Self::Unreadable
    }
}

/// One search in progress: the folders looked at so far and what was found.
struct Walk {
    search: Search,
    looks: usize,
    found: Vec<(PathBuf, ProjectGit)>,
}

impl Walk {
    /// Look at `folder`, `depth` levels below its root, and below it.
    /// Returns whether it could be looked at.
    fn folder(
        &mut self,
        folder: &Path,
        depth: usize,
        look: &mut impl FnMut(&Path) -> Option<Look>,
    ) -> bool {
        if self.looks >= self.search.looks {
            return false;
        }
        self.looks += 1;
        let Some(seen) = look(folder) else {
            return false;
        };
        if let Some(head) = seen.git {
            self.found
                .push((folder.to_path_buf(), ProjectGit::read(head.as_deref())));
            return true;
        }
        if depth >= self.search.depth {
            return true;
        }
        let mut names: Vec<&String> = seen
            .folders
            .iter()
            .filter(|name| !Search::skips(name))
            .collect();
        names.sort();
        for name in names {
            let child = folder.join(name);
            let before = self.found.len();
            // A root's own folder with no repository below it is offered
            // as it is.
            if self.folder(&child, depth + 1, look) && depth == 0 && self.found.len() == before {
                self.found.push((child, ProjectGit::None));
            }
        }
        true
    }
}

/// How far the search goes, and what it skips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Search {
    /// Levels below a root: 1 is the root's own folders.
    pub depth: usize,
    /// Folders looked at in all, so a huge tree cannot stall the daemon.
    pub looks: usize,
}

impl Search {
    pub const DEFAULT: Self = Self {
        depth: 3,
        looks: 2_000,
    };

    /// Folders that hold build output or dependencies, never projects.
    const SKIPPED: &'static [&'static str] = &[
        "node_modules",
        "target",
        "vendor",
        "dist",
        "build",
        "__pycache__",
    ];

    fn skips(name: &str) -> bool {
        name.starts_with('.') || name.contains('/') || Self::SKIPPED.contains(&name)
    }
}

impl Roots {
    /// The projects under every root, found by looking at folders with
    /// `look` (`None`: the folder could not be read), in path order.
    pub fn projects(
        &self,
        search: Search,
        mut look: impl FnMut(&Path) -> Option<Look>,
    ) -> Vec<Project> {
        let mut walk = Walk {
            search,
            looks: 0,
            found: Vec::new(),
        };
        for root in self.iter() {
            walk.folder(root, 0, &mut look);
        }
        let mut found = walk.found;
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found.dedup_by(|a, b| a.0 == b.0);
        Self::named(found)
    }

    /// Names projects by their folder, adding the parent's name where two
    /// would read the same. Paths that are not UTF-8 are left out.
    fn named(found: Vec<(PathBuf, ProjectGit)>) -> Vec<Project> {
        let last = |path: &Path| -> String {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned()
        };
        let names: Vec<String> = found.iter().map(|(path, _)| last(path)).collect();
        found
            .iter()
            .zip(&names)
            .filter_map(|((path, git), name)| {
                let shared = names.iter().filter(|other| *other == name).count() > 1;
                let name = match (shared, path.parent()) {
                    (true, Some(parent)) => format!("{}/{name}", last(parent)),
                    (true, None) | (false, _) => name.clone(),
                };
                Some(Project {
                    path: path.to_str()?.to_owned(),
                    name,
                    git: git.clone(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Look, ProjectGit, Search};
    use crate::workdir::Roots;
    use std::{collections::BTreeMap, path::PathBuf};

    /// A fake tree: each folder's subfolders, and the repos' `HEAD`s.
    fn tree() -> BTreeMap<&'static str, Look> {
        let folder = |names: &[&str]| Look {
            folders: names.iter().map(|name| (*name).to_owned()).collect(),
            git: None,
        };
        let repo = |head: &str, names: &[&str]| Look {
            folders: names.iter().map(|name| (*name).to_owned()).collect(),
            git: Some(Some(head.to_owned())),
        };
        BTreeMap::from([
            (
                "/dev",
                folder(&["acme", "notes", "work", ".cache", "node_modules"]),
            ),
            (
                "/dev/acme",
                repo("ref: refs/heads/main\n", &["src", "target"]),
            ),
            ("/dev/notes", folder(&["drafts"])),
            ("/dev/notes/drafts", folder(&[])),
            ("/dev/work", folder(&["api", "web"])),
            (
                "/dev/work/api",
                repo("0123456789abcdef0123456789abcdef01234567", &[]),
            ),
            ("/dev/work/web", repo("ref: refs/heads/feat/upload", &[])),
            (
                "/srv/ren",
                repo("ref: refs/heads/agent-runs", &["crates", "ui"]),
            ),
            ("/srv/other", folder(&["acme"])),
            ("/srv/other/acme", repo("garbage", &[])),
        ])
    }

    #[test]
    fn finds_repositories_without_going_inside_them() {
        let roots = Roots::new(vec![PathBuf::from("/dev"), PathBuf::from("/srv/ren")]).unwrap();
        let tree = tree();
        let mut looked = Vec::new();
        let projects = roots.projects(Search::DEFAULT, |path| {
            looked.push(path.to_str().unwrap().to_owned());
            tree.get(path.to_str().unwrap()).cloned()
        });
        let listed: Vec<(&str, &str, &ProjectGit)> = projects
            .iter()
            .map(|project| (project.name.as_str(), project.path.as_str(), &project.git))
            .collect();
        assert_eq!(
            listed,
            [
                (
                    "acme",
                    "/dev/acme",
                    &ProjectGit::Branch {
                        name: "main".into()
                    }
                ),
                // A root's folder with no repository below it is offered.
                ("notes", "/dev/notes", &ProjectGit::None),
                (
                    "api",
                    "/dev/work/api",
                    &ProjectGit::Detached {
                        commit: "0123456".into()
                    }
                ),
                (
                    "web",
                    "/dev/work/web",
                    &ProjectGit::Branch {
                        name: "feat/upload".into()
                    }
                ),
                // A root that is a repository is the project itself.
                (
                    "ren",
                    "/srv/ren",
                    &ProjectGit::Branch {
                        name: "agent-runs".into()
                    }
                ),
            ]
        );
        // Never inside a repository, a hidden folder, or dependencies.
        for skipped in [
            "/dev/acme/src",
            "/dev/.cache",
            "/dev/node_modules",
            "/srv/ren/crates",
        ] {
            assert!(!looked.iter().any(|path| path == skipped), "{skipped}");
        }
        // A folder holding a repository is not offered itself.
        assert!(!projects.iter().any(|project| project.path == "/dev/work"));
    }

    #[test]
    fn shared_names_take_their_parent_and_limits_hold() {
        let roots = Roots::new(vec![PathBuf::from("/dev"), PathBuf::from("/srv/other")]).unwrap();
        let tree = tree();
        let projects = roots.projects(Search::DEFAULT, |path| {
            tree.get(path.to_str().unwrap()).cloned()
        });
        let names: Vec<&str> = projects
            .iter()
            .map(|project| project.name.as_str())
            .collect();
        assert!(
            names.contains(&"dev/acme") && names.contains(&"other/acme"),
            "{names:?}"
        );
        let unreadable = projects
            .iter()
            .find(|project| project.path == "/srv/other/acme");
        assert_eq!(
            unreadable.map(|project| &project.git),
            Some(&ProjectGit::Unreadable)
        );

        let mut looks = 0;
        let few = roots.projects(Search { depth: 3, looks: 2 }, |path| {
            looks += 1;
            tree.get(path.to_str().unwrap()).cloned()
        });
        assert_eq!(looks, 2);
        assert!(few.len() <= 2);
        let shallow = roots.projects(
            Search {
                depth: 1,
                looks: 100,
            },
            |path| tree.get(path.to_str().unwrap()).cloned(),
        );
        assert!(
            !shallow
                .iter()
                .any(|project| project.path == "/dev/work/api")
        );
    }

    #[test]
    fn reads_head() {
        assert_eq!(
            ProjectGit::read(Some("ref: refs/heads/main\n")),
            ProjectGit::Branch {
                name: "main".into()
            }
        );
        assert_eq!(ProjectGit::read(None), ProjectGit::Unreadable);
        assert_eq!(ProjectGit::read(Some("nonsense")), ProjectGit::Unreadable);
    }
}
