//! The compatibility contract, pure: what a change does to the files an
//! existing install depends on (the store's migration pins, its store
//! fixtures, and its migrations, and every crate's label tables), and
//! whether the change's message declares the breaks it makes.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A file the contract watches, known by its path from the repository's
/// root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The store's pinned migration checksums, one line per migration.
    Pins,
    /// The rows a store held at one migration, frozen once shipped.
    StoreFixture,
    /// A store migration's SQL.
    Migration,
    /// A crate's label table: each vocabulary with its labels.
    Labels,
}

impl Surface {
    pub const PINS: &str = "crates/ren-store/tests/fixtures/migrations.txt";
    pub const STORE_FIXTURES: &str = "crates/ren-store/tests/fixtures/stores";
    pub const MIGRATIONS: &str = "crates/ren-store/migrations";
    /// Each crate's label table is at this path inside the crate.
    pub const LABELS: &str = "tests/fixtures/labels.txt";

    /// The surface the file at `path` belongs to, if the contract watches it.
    pub fn of(path: &str) -> Option<Self> {
        if path == Self::PINS {
            return Some(Self::Pins);
        }
        let inside = |folder: &str| {
            path.strip_prefix(folder)
                .and_then(|rest| rest.strip_prefix('/'))
                .filter(|name| !name.contains('/') && name.ends_with(".sql"))
                .is_some()
        };
        if inside(Self::STORE_FIXTURES) {
            return Some(Self::StoreFixture);
        }
        if inside(Self::MIGRATIONS) {
            return Some(Self::Migration);
        }
        path.strip_prefix("crates/")
            .and_then(|rest| rest.split_once('/'))
            .filter(|(krate, rest)| !krate.is_empty() && *rest == Self::LABELS)
            .map(|_| Self::Labels)
    }
}

/// The watched files at one revision, by path.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Snapshot(BTreeMap<String, Vec<u8>>);

impl Snapshot {
    /// Keep `bytes` as the file at `path`, when the contract watches it.
    pub fn keep(&mut self, path: String, bytes: Vec<u8>) {
        if Surface::of(&path).is_some() {
            self.0.insert(path, bytes);
        }
    }
}

/// What a change did to one watched file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: String,
    pub change: Change,
}

/// A change to a watched file, as it bears on existing installs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    PinAdded {
        version: String,
    },
    /// A shipped migration's pin changed: the migration itself changed, and
    /// every store that ran it refuses to open.
    PinChanged {
        version: String,
    },
    PinRemoved {
        version: String,
    },
    FixtureAdded,
    /// A frozen store fixture changed: it no longer shows what that release
    /// wrote.
    FixtureChanged,
    FixtureRemoved,
    MigrationAdded,
    /// A new migration takes data or structure away; see [`Destructive`].
    MigrationDestructive {
        statement: Destructive,
    },
    MigrationChanged,
    MigrationRemoved,
    VocabularyAdded {
        vocabulary: String,
    },
    VocabularyRemoved {
        vocabulary: String,
    },
    LabelAdded {
        vocabulary: String,
        label: String,
    },
    /// A label stored or sent by earlier releases is no longer read.
    LabelRemoved {
        vocabulary: String,
        label: String,
    },
}

/// What a change means for an install of the base revision upgrading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Impact {
    Compatible,
    Breaking,
}

/// A statement that takes data or structure away. A migration expands
/// first; what it stops using is removed by a later one, declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destructive {
    DropTable,
    DropColumn,
    Rename,
    Delete,
}

impl Change {
    pub fn impact(&self) -> Impact {
        match self {
            Self::PinAdded { .. }
            | Self::FixtureAdded
            | Self::MigrationAdded
            | Self::VocabularyAdded { .. }
            | Self::LabelAdded { .. } => Impact::Compatible,
            Self::PinChanged { .. }
            | Self::PinRemoved { .. }
            | Self::FixtureChanged
            | Self::FixtureRemoved
            | Self::MigrationDestructive { .. }
            | Self::MigrationChanged
            | Self::MigrationRemoved
            | Self::VocabularyRemoved { .. }
            | Self::LabelRemoved { .. } => Impact::Breaking,
        }
    }
}

impl Destructive {
    /// The first statement in `sql` that takes something away, if any.
    /// Comments are skipped; words are compared without case.
    pub fn first_in(sql: &str) -> Option<Self> {
        let words: Vec<String> = sql
            .lines()
            .map(|line| line.split_once("--").map_or(line, |(code, _)| code))
            .flat_map(|code| code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')))
            .filter(|word| !word.is_empty())
            .map(str::to_ascii_uppercase)
            .collect();
        words.windows(2).find_map(|pair| match pair {
            [first, second] => match (first.as_str(), second.as_str()) {
                ("DROP", "TABLE") => Some(Self::DropTable),
                ("DROP", "COLUMN") => Some(Self::DropColumn),
                ("RENAME", "TO" | "COLUMN") => Some(Self::Rename),
                ("DELETE", "FROM") => Some(Self::Delete),
                _ => None,
            },
            _ => None,
        })
    }
}

impl fmt::Display for Destructive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DropTable => "DROP TABLE",
            Self::DropColumn => "DROP COLUMN",
            Self::Rename => "RENAME",
            Self::Delete => "DELETE FROM",
        })
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = &self.path;
        match &self.change {
            Change::PinAdded { version } => write!(f, "{path}: migration {version} pinned"),
            Change::PinChanged { version } => {
                write!(f, "{path}: shipped migration {version} changed")
            }
            Change::PinRemoved { version } => {
                write!(f, "{path}: shipped migration {version} removed")
            }
            Change::FixtureAdded => write!(f, "{path}: store fixture added"),
            Change::FixtureChanged => write!(f, "{path}: frozen store fixture changed"),
            Change::FixtureRemoved => write!(f, "{path}: frozen store fixture removed"),
            Change::MigrationAdded => write!(f, "{path}: migration added"),
            Change::MigrationDestructive { statement } => {
                write!(f, "{path}: new migration uses {statement}")
            }
            Change::MigrationChanged => write!(f, "{path}: shipped migration changed"),
            Change::MigrationRemoved => write!(f, "{path}: shipped migration removed"),
            Change::VocabularyAdded { vocabulary } => {
                write!(f, "{path}: vocabulary {vocabulary} added")
            }
            Change::VocabularyRemoved { vocabulary } => {
                write!(f, "{path}: vocabulary {vocabulary} removed")
            }
            Change::LabelAdded { vocabulary, label } => {
                write!(f, "{path}: {vocabulary} gained `{label}`")
            }
            Change::LabelRemoved { vocabulary, label } => {
                write!(f, "{path}: {vocabulary} lost `{label}`")
            }
        }
    }
}

/// Everything a change did to the watched files, from a base revision to
/// the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    findings: Vec<Finding>,
}

impl Comparison {
    pub fn between(base: &Snapshot, head: &Snapshot) -> Self {
        let paths: BTreeSet<&String> = base.0.keys().chain(head.0.keys()).collect();
        let mut findings = Vec::new();
        for path in paths {
            let (before, after) = (base.0.get(path), head.0.get(path));
            if before == after {
                continue;
            }
            let Some(surface) = Surface::of(path) else {
                continue;
            };
            let mut found = |change| {
                findings.push(Finding {
                    path: path.clone(),
                    change,
                });
            };
            match surface {
                Surface::Pins => {
                    let (before, after) = (Pins::read(before), Pins::read(after));
                    for (version, line) in &before.0 {
                        match after.0.get(version) {
                            None => found(Change::PinRemoved {
                                version: version.clone(),
                            }),
                            Some(now) if now != line => found(Change::PinChanged {
                                version: version.clone(),
                            }),
                            Some(_) => {}
                        }
                    }
                    for version in after.0.keys().filter(|v| !before.0.contains_key(*v)) {
                        found(Change::PinAdded {
                            version: version.clone(),
                        });
                    }
                }
                Surface::StoreFixture => found(match (before, after) {
                    (None, _) => Change::FixtureAdded,
                    (Some(_), None) => Change::FixtureRemoved,
                    (Some(_), Some(_)) => Change::FixtureChanged,
                }),
                Surface::Migration => found(match (before, after) {
                    (None, Some(sql)) => Destructive::first_in(&String::from_utf8_lossy(sql))
                        .map_or(Change::MigrationAdded, |statement| {
                            Change::MigrationDestructive { statement }
                        }),
                    (None, None) => continue,
                    (Some(_), None) => Change::MigrationRemoved,
                    (Some(_), Some(_)) => Change::MigrationChanged,
                }),
                Surface::Labels => {
                    let (before, after) = (Labels::read(before), Labels::read(after));
                    for (vocabulary, labels) in &before.0 {
                        let Some(now) = after.0.get(vocabulary) else {
                            found(Change::VocabularyRemoved {
                                vocabulary: vocabulary.clone(),
                            });
                            continue;
                        };
                        for label in labels.difference(now) {
                            found(Change::LabelRemoved {
                                vocabulary: vocabulary.clone(),
                                label: label.clone(),
                            });
                        }
                        for label in now.difference(labels) {
                            found(Change::LabelAdded {
                                vocabulary: vocabulary.clone(),
                                label: label.clone(),
                            });
                        }
                    }
                    for vocabulary in after.0.keys().filter(|v| !before.0.contains_key(*v)) {
                        found(Change::VocabularyAdded {
                            vocabulary: vocabulary.clone(),
                        });
                    }
                }
            }
        }
        Self { findings }
    }

    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// Whether the change may land with `declaration` as its message's.
    pub fn verdict(&self, declaration: Declaration) -> Verdict {
        let breaking = self
            .findings
            .iter()
            .any(|finding| finding.change.impact() == Impact::Breaking);
        match (breaking, declaration) {
            (false, _) => Verdict::Compatible,
            (true, Declaration::Breaking) => Verdict::DeclaredBreaking,
            (true, declaration @ (Declaration::Undeclared | Declaration::Partial(_))) => {
                Verdict::UndeclaredBreaking(declaration)
            }
        }
    }
}

/// The migration pins of one revision, by version.
struct Pins(BTreeMap<String, String>);

impl Pins {
    fn read(bytes: Option<&Vec<u8>>) -> Self {
        let text = bytes.map(|bytes| String::from_utf8_lossy(bytes).into_owned());
        Self(
            text.iter()
                .flat_map(|text| text.lines())
                .filter_map(|line| {
                    let version = line.split_whitespace().next()?;
                    Some((version.to_owned(), line.trim().to_owned()))
                })
                .collect(),
        )
    }
}

/// One label table: each vocabulary with its labels.
struct Labels(BTreeMap<String, BTreeSet<String>>);

impl Labels {
    fn read(bytes: Option<&Vec<u8>>) -> Self {
        let text = bytes.map(|bytes| String::from_utf8_lossy(bytes).into_owned());
        Self(
            text.iter()
                .flat_map(|text| text.lines())
                .filter_map(|line| {
                    // Vocabularies are paths (`chunk::Codec`); labels hold no colon.
                    let (vocabulary, labels) = line.rsplit_once(':')?;
                    Some((
                        vocabulary.trim().to_owned(),
                        labels.split_whitespace().map(str::to_owned).collect(),
                    ))
                })
                .collect(),
        )
    }
}

/// What a change's message says of compatibility, in Conventional Commits:
/// a `!` before the header's colon, and a `BREAKING CHANGE:` footer saying
/// what breaks and how an existing install moves on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Declaration {
    Breaking,
    Undeclared,
    /// One of the two, without the other.
    Partial(Missing),
}

/// The half of a breaking declaration a message lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// The header has no `!`.
    Marker,
    /// No `BREAKING CHANGE:` footer says what breaks.
    Note,
}

impl Declaration {
    pub fn read(message: &str) -> Self {
        let mut lines = message.lines();
        let marked = lines
            .next()
            .and_then(|header| header.split_once(':'))
            .is_some_and(|(kind, _)| kind.trim_end().ends_with('!'));
        let noted = lines.any(|line| {
            ["BREAKING CHANGE:", "BREAKING-CHANGE:"]
                .iter()
                .any(|token| {
                    line.strip_prefix(token)
                        .is_some_and(|note| !note.trim().is_empty())
                })
        });
        match (marked, noted) {
            (true, true) => Self::Breaking,
            (false, false) => Self::Undeclared,
            (true, false) => Self::Partial(Missing::Note),
            (false, true) => Self::Partial(Missing::Marker),
        }
    }
}

/// Whether a change may land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing an existing install depends on breaks.
    Compatible,
    /// It breaks, and its message says so.
    DeclaredBreaking,
    /// It breaks, and its message does not say so (in full).
    UndeclaredBreaking(Declaration),
}

#[cfg(test)]
mod tests {
    use super::{
        Change, Comparison, Declaration, Destructive, Missing, Snapshot, Surface, Verdict,
    };

    fn snapshot(files: &[(&str, &str)]) -> Snapshot {
        let mut snapshot = Snapshot::default();
        for (path, text) in files {
            snapshot.keep((*path).to_owned(), text.as_bytes().to_vec());
        }
        snapshot
    }

    fn changes(base: &[(&str, &str)], head: &[(&str, &str)]) -> Vec<Change> {
        Comparison::between(&snapshot(base), &snapshot(head))
            .findings()
            .iter()
            .map(|finding| finding.change.clone())
            .collect()
    }

    const LABELS: &str = "crates/ren-store/tests/fixtures/labels.txt";
    const FIXTURE: &str = "crates/ren-store/tests/fixtures/stores/0001.sql";
    const MIGRATION: &str = "crates/ren-store/migrations/0002_region.sql";

    #[test]
    fn only_the_contracts_files_are_watched() {
        assert_eq!(Surface::of(Surface::PINS), Some(Surface::Pins));
        assert_eq!(Surface::of(FIXTURE), Some(Surface::StoreFixture));
        assert_eq!(Surface::of(MIGRATION), Some(Surface::Migration));
        assert_eq!(Surface::of(LABELS), Some(Surface::Labels));
        for path in [
            "crates/ren-store/tests/fixtures/stores/notes.md",
            "crates/ren-store/migrations/old/0001_store.sql",
            "crates/ren-store/src/lib.rs",
            "crates//tests/fixtures/labels.txt",
            "ui/apps/web/tests/fixtures/labels.txt",
            "crates/a/b/tests/fixtures/labels.txt",
        ] {
            assert_eq!(Surface::of(path), None, "{path}");
        }
        let mut kept = Snapshot::default();
        kept.keep("README.md".into(), b"x".to_vec());
        assert_eq!(kept, Snapshot::default());
    }

    #[test]
    fn labels_may_be_added_but_never_taken_away() {
        let base = [(
            LABELS,
            "chunk::Codec: raw zstd\nquery::Shelf: current archived\n",
        )];
        assert_eq!(
            changes(
                &base,
                &[(
                    LABELS,
                    "chunk::Codec: raw zstd lz4\nquery::Shelf: current archived\nnew::Kind: a\n"
                )]
            ),
            [
                Change::LabelAdded {
                    vocabulary: "chunk::Codec".into(),
                    label: "lz4".into()
                },
                Change::VocabularyAdded {
                    vocabulary: "new::Kind".into()
                },
            ]
        );
        let renamed = changes(&base, &[(LABELS, "chunk::Codec: raw zst\n")]);
        assert_eq!(
            renamed,
            [
                Change::LabelRemoved {
                    vocabulary: "chunk::Codec".into(),
                    label: "zstd".into()
                },
                Change::LabelAdded {
                    vocabulary: "chunk::Codec".into(),
                    label: "zst".into()
                },
                Change::VocabularyRemoved {
                    vocabulary: "query::Shelf".into()
                },
            ]
        );
        // Reordered labels are the same labels.
        assert!(
            changes(
                &base,
                &[(
                    LABELS,
                    "query::Shelf: archived current\nchunk::Codec: zstd raw\n"
                )]
            )
            .is_empty()
        );
    }

    #[test]
    fn shipped_pins_and_fixtures_are_frozen() {
        let pins = "0001 store aa\n";
        assert_eq!(
            changes(
                &[(Surface::PINS, pins)],
                &[(Surface::PINS, "0001 store aa\n0002 region bb\n")]
            ),
            [Change::PinAdded {
                version: "0002".into()
            }]
        );
        assert_eq!(
            changes(
                &[(Surface::PINS, pins)],
                &[(Surface::PINS, "0001 store ab\n")]
            ),
            [Change::PinChanged {
                version: "0001".into()
            }]
        );
        assert_eq!(
            changes(&[(Surface::PINS, pins)], &[(Surface::PINS, "")]),
            [Change::PinRemoved {
                version: "0001".into()
            }]
        );
        assert_eq!(changes(&[], &[(FIXTURE, "rows")]), [Change::FixtureAdded]);
        assert_eq!(
            changes(&[(FIXTURE, "rows")], &[(FIXTURE, "other rows")]),
            [Change::FixtureChanged]
        );
        assert_eq!(changes(&[(FIXTURE, "rows")], &[]), [Change::FixtureRemoved]);
        assert!(changes(&[(FIXTURE, "rows")], &[(FIXTURE, "rows")]).is_empty());
    }

    #[test]
    fn a_new_migration_expands_and_never_takes_away() {
        let added = |sql: &str| changes(&[], &[(MIGRATION, sql)]);
        assert_eq!(
            added("ALTER TABLE calls ADD COLUMN region TEXT;\n-- drop table later\n"),
            [Change::MigrationAdded]
        );
        for (sql, statement) in [
            ("drop  table calls;", Destructive::DropTable),
            (
                "ALTER TABLE calls\n  DROP COLUMN wire;",
                Destructive::DropColumn,
            ),
            (
                "ALTER TABLE calls RENAME TO old_calls;",
                Destructive::Rename,
            ),
            (
                "ALTER TABLE calls RENAME COLUMN wire TO api;",
                Destructive::Rename,
            ),
            ("DELETE FROM calls WHERE 1;", Destructive::Delete),
        ] {
            assert_eq!(
                added(sql),
                [Change::MigrationDestructive { statement }],
                "{sql}"
            );
        }
        assert_eq!(
            changes(&[(MIGRATION, "a")], &[(MIGRATION, "b")]),
            [Change::MigrationChanged]
        );
        assert_eq!(
            changes(&[(MIGRATION, "a")], &[]),
            [Change::MigrationRemoved]
        );
    }

    #[test]
    fn a_break_needs_the_marker_and_the_note() {
        assert_eq!(
            Declaration::read(
                "refactor(store)!: one migration\n\nWhy.\n\nBREAKING CHANGE: start from an empty data directory.\n"
            ),
            Declaration::Breaking
        );
        assert_eq!(
            Declaration::read("feat!: x\n\nBREAKING-CHANGE: the API moved\n"),
            Declaration::Breaking
        );
        assert_eq!(Declaration::read("fix: x\n\nbody"), Declaration::Undeclared);
        assert_eq!(
            Declaration::read("fix!: x\n\nBREAKING CHANGE:   \n"),
            Declaration::Partial(Missing::Note)
        );
        assert_eq!(
            Declaration::read("fix: x\n\nBREAKING CHANGE: it moved\n"),
            Declaration::Partial(Missing::Marker)
        );
        // A footer-like first line is the header, not a note.
        assert_eq!(
            Declaration::read("BREAKING CHANGE: x"),
            Declaration::Undeclared
        );

        let breaking =
            Comparison::between(&snapshot(&[(FIXTURE, "a")]), &snapshot(&[(FIXTURE, "b")]));
        assert_eq!(
            breaking.verdict(Declaration::Breaking),
            Verdict::DeclaredBreaking
        );
        assert_eq!(
            breaking.verdict(Declaration::Partial(Missing::Note)),
            Verdict::UndeclaredBreaking(Declaration::Partial(Missing::Note))
        );
        let compatible = Comparison::between(&snapshot(&[]), &snapshot(&[(FIXTURE, "a")]));
        assert_eq!(
            compatible.verdict(Declaration::Undeclared),
            Verdict::Compatible
        );
        assert_eq!(
            compatible.verdict(Declaration::Breaking),
            Verdict::Compatible
        );
    }
}
