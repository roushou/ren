//! Where a run can start. Projects under the run roots: a folder that holds
//! a git repository is a project, and the search does not go inside it; a
//! folder directly inside a root with no repository below it is a project
//! too, without git. Other places: each configured place's own folders,
//! hidden ones included, or the place itself when it is a repository. And
//! the scratch folders earlier scratch runs worked in. The search is pure:
//! the host looks at each folder (`AgentHost::look`), and this module decides
//! where to look next.

use crate::workdir::{Bounds, Roots};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::path::{Path, PathBuf};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

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
    /// Its folder's name, with its parent's when two of a kind share it.
    pub name: String,
    /// The path as the operator reads it: `~/…` under their home.
    pub shown: String,
    pub kind: PlaceKind,
    pub git: ProjectGit,
}

/// What kind of place a run can start in.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum PlaceKind {
    /// A project under a run root.
    Project,
    /// A folder of another configured place, offered only when asked.
    Other,
    /// The folder an earlier scratch run worked in.
    Scratch,
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
        name.starts_with('.') || Self::skips_shown(name)
    }

    /// Skipped even where hidden folders are offered.
    fn skips_shown(name: &str) -> bool {
        name.contains('/') || Self::SKIPPED.contains(&name)
    }
}

impl Bounds {
    /// Every place a run can start in: the projects under the roots, the
    /// folders of the other places, and the scratch folders, each named
    /// within its kind and in path order.
    pub fn places(
        &self,
        search: Search,
        mut look: impl FnMut(&Path) -> Option<Look>,
    ) -> Vec<Project> {
        let projects = self.named(self.roots().found(search, &mut look), PlaceKind::Project);
        let mut looks = 0;
        let mut seen = |path: &Path| {
            looks += 1;
            if looks > search.looks {
                None
            } else {
                look(path)
            }
        };
        let mut others = Vec::new();
        for place in self.others() {
            let Some(inside) = seen(place) else { continue };
            if let Some(head) = inside.git {
                others.push((place.clone(), ProjectGit::read(head.as_deref())));
                continue;
            }
            others.extend(Self::folders(
                place,
                &inside,
                Search::skips_shown,
                &mut seen,
            ));
        }
        let scratch = match self.scratch() {
            Some(root) => match seen(root) {
                Some(inside) => Self::folders(root, &inside, Search::skips, &mut seen),
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        let mut places = projects;
        for (kind, mut found) in [(PlaceKind::Other, others), (PlaceKind::Scratch, scratch)] {
            found.sort_by(|a, b| a.0.cmp(&b.0));
            found.dedup_by(|a, b| a.0 == b.0);
            places.extend(self.named(found, kind));
        }
        places
    }

    /// The folders directly inside `folder`, each with its repository.
    fn folders(
        folder: &Path,
        inside: &Look,
        skips: fn(&str) -> bool,
        look: &mut impl FnMut(&Path) -> Option<Look>,
    ) -> Vec<(PathBuf, ProjectGit)> {
        inside
            .folders
            .iter()
            .filter(|name| !skips(name))
            .filter_map(|name| {
                let child = folder.join(name);
                let git = match look(&child)?.git {
                    Some(head) => ProjectGit::read(head.as_deref()),
                    None => ProjectGit::None,
                };
                Some((child, git))
            })
            .collect()
    }
}

impl Roots {
    /// The projects under every root, found by looking at folders with
    /// `look` (`None`: the folder could not be read), in path order and
    /// not yet named.
    fn found(
        &self,
        search: Search,
        look: &mut impl FnMut(&Path) -> Option<Look>,
    ) -> Vec<(PathBuf, ProjectGit)> {
        let mut walk = Walk {
            search,
            looks: 0,
            found: Vec::new(),
        };
        for root in self.iter() {
            walk.folder(root, 0, look);
        }
        let mut found = walk.found;
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found.dedup_by(|a, b| a.0 == b.0);
        found
    }
}

impl Bounds {
    /// Names places of `kind` by their folder, adding the parent's name
    /// where two would read the same. Paths that are not UTF-8 are left out.
    fn named(&self, found: Vec<(PathBuf, ProjectGit)>, kind: PlaceKind) -> Vec<Project> {
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
                    shown: self.shown(path),
                    kind,
                    git: git.clone(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Look, PlaceKind, ProjectGit, Search};
    use crate::workdir::{Bounds, Roots};
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

    /// Projects under `roots`, with no other place and no scratch folder.
    fn bounds(roots: &[&str]) -> Bounds {
        Bounds::new(
            Roots::new(roots.iter().map(PathBuf::from).collect()).unwrap(),
            Vec::new(),
            None,
            None,
        )
        .unwrap()
    }

    #[test]
    fn other_places_and_scratch_folders_are_listed_apart() {
        let bounds = Bounds::new(
            Roots::new(vec![PathBuf::from("/home/op/dev")]).unwrap(),
            vec![
                PathBuf::from("/home/op/.config"),
                PathBuf::from("/home/op/dotfiles"),
            ],
            Some(PathBuf::from("/home/op/scratch")),
            Some(PathBuf::from("/home/op")),
        )
        .unwrap();
        let folder = |names: &[&str], git: Option<&str>| Look {
            folders: names.iter().map(|name| (*name).to_owned()).collect(),
            git: git.map(|head| Some(head.to_owned())),
        };
        let tree = BTreeMap::from([
            ("/home/op/dev", folder(&["acme"], None)),
            (
                "/home/op/dev/acme",
                folder(&[], Some("ref: refs/heads/main")),
            ),
            // Hidden folders are offered in another place; dependencies not.
            (
                "/home/op/.config",
                folder(&["nvim", ".private", "node_modules"], None),
            ),
            (
                "/home/op/.config/nvim",
                folder(&[], Some("ref: refs/heads/main")),
            ),
            ("/home/op/.config/.private", folder(&[], None)),
            // A place that is a repository is offered itself.
            (
                "/home/op/dotfiles",
                folder(&["zsh"], Some("ref: refs/heads/main")),
            ),
            ("/home/op/scratch", folder(&["scratch-1a2b", ".tmp"], None)),
            (
                "/home/op/scratch/scratch-1a2b",
                folder(&[], Some("ref: refs/heads/main")),
            ),
        ]);
        let places = bounds.places(Search::DEFAULT, |path| {
            tree.get(path.to_str().unwrap()).cloned()
        });
        let listed: Vec<(PlaceKind, &str, &str)> = places
            .iter()
            .map(|place| (place.kind, place.name.as_str(), place.shown.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                (PlaceKind::Project, "acme", "~/dev/acme"),
                (PlaceKind::Other, ".private", "~/.config/.private"),
                (PlaceKind::Other, "nvim", "~/.config/nvim"),
                (PlaceKind::Other, "dotfiles", "~/dotfiles"),
                (PlaceKind::Scratch, "scratch-1a2b", "~/scratch/scratch-1a2b"),
            ]
        );
    }

    #[test]
    fn finds_repositories_without_going_inside_them() {
        let roots = bounds(&["/dev", "/srv/ren"]);
        let tree = tree();
        let mut looked = Vec::new();
        let projects = roots.places(Search::DEFAULT, |path| {
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
        let roots = bounds(&["/dev", "/srv/other"]);
        let tree = tree();
        let projects = roots.places(Search::DEFAULT, |path| {
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
        let few = roots.places(Search { depth: 3, looks: 2 }, |path| {
            looks += 1;
            tree.get(path.to_str().unwrap()).cloned()
        });
        assert_eq!(looks, 2);
        assert!(few.len() <= 2);
        let shallow = roots.places(
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
