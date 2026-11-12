//! Finding a file of a run's folder for a prompt to name (`@path`): what the
//! host found there, ranked against what the operator typed. Pure: the host
//! walks the folder ([`AgentHost::files`](crate::host::AgentHost::files)),
//! leaving out what git ignores, within [`FileListing::LIMITS`].

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// A file or folder found in a run's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundPath {
    /// Relative to the run's folder, `/`-separated, never empty.
    pub path: String,
    pub kind: PathKind,
}

/// Whether a found path is a file or a folder.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub enum PathKind {
    File,
    Directory,
}

/// How far the host walks a folder: so a search never stalls the runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkLimits {
    /// Paths found before the walk stops.
    pub entries: usize,
    /// Folders deep below the run's folder.
    pub depth: usize,
}

/// What the operator typed after `@`: matched in order, ignoring case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileQuery(Vec<char>);

/// The longest query read: longer ones are cut.
const QUERY_CHARS: usize = 200;

impl FileQuery {
    pub fn parse(text: &str) -> Self {
        Self(
            text.trim()
                .chars()
                .filter(|ch| !ch.is_control())
                .take(QUERY_CHARS)
                .flat_map(char::to_lowercase)
                .collect(),
        )
    }
}

/// A path that matched, with where: the positions (in chars) of the
/// path's characters the query matched, for drawing them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileMatch {
    pub path: String,
    pub kind: PathKind,
    pub positions: Vec<usize>,
}

/// Every path the host found in a run's folder.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileListing {
    paths: Vec<FoundPath>,
}

impl FileListing {
    /// How far a folder is walked for a listing.
    pub const LIMITS: WalkLimits = WalkLimits {
        entries: 20_000,
        depth: 16,
    };

    pub fn new(paths: Vec<FoundPath>) -> Self {
        Self { paths }
    }

    /// The `limit` best matches for `query`, best first: a match in the
    /// file's own name before one spread over its folders, runs of
    /// characters and word starts before scattered ones, and shorter paths
    /// first. With nothing typed, the shallowest paths, by name.
    pub fn search(&self, query: &FileQuery, limit: usize) -> Vec<FileMatch> {
        if query.0.is_empty() {
            let mut shallow: Vec<&FoundPath> = self.paths.iter().collect();
            shallow.sort_by(|a, b| {
                Self::depth(&a.path)
                    .cmp(&Self::depth(&b.path))
                    .then_with(|| a.path.cmp(&b.path))
            });
            return shallow
                .into_iter()
                .take(limit)
                .map(|found| FileMatch {
                    path: found.path.clone(),
                    kind: found.kind,
                    positions: Vec::new(),
                })
                .collect();
        }
        let mut scored: Vec<(i64, FileMatch)> = self
            .paths
            .iter()
            .filter_map(|found| {
                let (score, positions) = Self::score(&found.path, &query.0)?;
                Some((
                    score,
                    FileMatch {
                        path: found.path.clone(),
                        kind: found.kind,
                        positions,
                    },
                ))
            })
            .collect();
        scored.sort_by(|(a, left), (b, right)| b.cmp(a).then_with(|| left.path.cmp(&right.path)));
        scored
            .into_iter()
            .take(limit)
            .map(|(_, found)| found)
            .collect()
    }

    fn depth(path: &str) -> usize {
        path.matches('/').count()
    }

    /// How well `query` matches `path`, and where; none when it does not.
    fn score(path: &str, query: &[char]) -> Option<(i64, Vec<usize>)> {
        let chars: Vec<char> = path.chars().collect();
        let lower: Vec<char> = chars
            .iter()
            .map(|ch| ch.to_lowercase().next().unwrap_or(*ch))
            .collect();
        let name_start = chars
            .iter()
            .rposition(|ch| *ch == '/')
            .map_or(0, |slash| slash + 1);
        // In the name alone, when it holds the whole query; else anywhere.
        let (positions, in_name) = match Self::find(&lower, name_start, query) {
            Some(positions) => (positions, true),
            None => (Self::find(&lower, 0, query)?, false),
        };
        let mut score: i64 = 0;
        let mut previous: Option<usize> = None;
        for &at in &positions {
            score += 16;
            if previous.is_some_and(|previous| previous + 1 == at) {
                score += 24;
            }
            if Self::word_start(&chars, at) {
                score += 20;
            }
            previous = Some(at);
        }
        if in_name {
            score += 60;
            if positions.first() == Some(&name_start) {
                score += 30;
            }
        }
        let length = i64::try_from(chars.len()).unwrap_or(i64::MAX);
        let depth = i64::try_from(Self::depth(path)).unwrap_or(i64::MAX);
        score -= length / 4 + depth * 2;
        Some((score, positions))
    }

    /// The positions from `from` on where `query` matches in order, each
    /// character at its first place after the one before.
    fn find(lower: &[char], from: usize, query: &[char]) -> Option<Vec<usize>> {
        let mut positions = Vec::with_capacity(query.len());
        let mut at = from;
        for ch in query {
            let found = lower
                .get(at..)?
                .iter()
                .position(|candidate| candidate == ch)?;
            positions.push(at + found);
            at += found + 1;
        }
        Some(positions)
    }

    /// Whether `at` starts a word: the path's start, after a separator, or
    /// an upper-case letter after a lower-case one.
    fn word_start(chars: &[char], at: usize) -> bool {
        let Some(before) = at.checked_sub(1).and_then(|before| chars.get(before)) else {
            return true;
        };
        matches!(before, '/' | '-' | '_' | '.' | ' ')
            || (before.is_lowercase() && chars.get(at).is_some_and(|ch| ch.is_uppercase()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(paths: &[&str]) -> FileListing {
        FileListing::new(
            paths
                .iter()
                .map(|path| FoundPath {
                    path: (*path).trim_end_matches('/').to_owned(),
                    kind: if path.ends_with('/') {
                        PathKind::Directory
                    } else {
                        PathKind::File
                    },
                })
                .collect(),
        )
    }

    fn found(listing: &FileListing, query: &str) -> Vec<String> {
        listing
            .search(&FileQuery::parse(query), 5)
            .into_iter()
            .map(|found| found.path)
            .collect()
    }

    #[test]
    fn a_name_match_comes_before_one_spread_over_folders() {
        let listing = listing(&[
            "src/routes/runs/+page.svelte",
            "src/lib/runs.ts",
            "src/lib/server/control.ts",
            "docs/api.md",
            "src/",
        ]);
        assert_eq!(
            found(&listing, "runs"),
            ["src/lib/runs.ts", "src/routes/runs/+page.svelte"]
        );
        assert_eq!(found(&listing, "ctl")[0], "src/lib/server/control.ts");
        assert_eq!(found(&listing, "API"), ["docs/api.md"]);
        assert!(found(&listing, "zzz").is_empty());
    }

    #[test]
    fn positions_mark_the_matched_characters() {
        let listing = listing(&["src/lib/runs.ts"]);
        let matched = listing.search(&FileQuery::parse("rt"), 1);
        assert_eq!(matched[0].positions, [8, 13]);
    }

    #[test]
    fn nothing_typed_lists_the_shallowest_paths_by_name() {
        let listing = listing(&["src/lib/a.ts", "README.md", "src/", "Cargo.toml"]);
        assert_eq!(
            found(&listing, ""),
            ["Cargo.toml", "README.md", "src", "src/lib/a.ts"]
        );
        assert_eq!(found(&listing, "   "), found(&listing, ""));
    }

    #[test]
    fn the_limit_holds() {
        let paths: Vec<String> = (0..50).map(|n| format!("file{n}.rs")).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        assert_eq!(listing(&refs).search(&FileQuery::parse("file"), 7).len(), 7);
    }
}
