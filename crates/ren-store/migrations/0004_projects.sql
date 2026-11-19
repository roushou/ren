-- Projects: the repositories Ren builds, each registered by the operator
-- (docs/plans/hierarchy.md). A name and a repository are each one
-- project's, archived or not.
CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    -- A slug that names it in the CLI and in URLs.
    name TEXT NOT NULL UNIQUE,
    -- The repository's canonical absolute path.
    repository TEXT NOT NULL UNIQUE,
    -- A factory label: ren or contract.
    factory TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    archived_at INTEGER
);
