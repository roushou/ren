-- Sessions an operator works in: a folder and a client, with the
-- conversation across every run (client process) that serves it. Not the
-- `sessions` table, which holds recordings: calls grouped as their client
-- names them. The settings are the session's newest: what its next turn runs
-- on. NULL is the client's own choice.
CREATE TABLE operator_sessions (
    id TEXT PRIMARY KEY,
    harness TEXT NOT NULL,
    workdir TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    model TEXT,
    effort TEXT,
    approvals TEXT,
    archived_at INTEGER
);
CREATE INDEX operator_sessions_created ON operator_sessions(created_at DESC, id DESC);

-- The session each run serves. A run recorded before sessions gets a session
-- of its own, created as the run started, under the run's own id, with the
-- run's settings and archive.
ALTER TABLE runs ADD COLUMN operator_session TEXT;
INSERT INTO operator_sessions(id, harness, workdir, created_at, model, effort, approvals, archived_at)
    SELECT id, harness, workdir, started_at, model, effort, approvals, archived_at FROM runs;
UPDATE runs SET operator_session = id;
CREATE INDEX runs_operator_session ON runs(operator_session, started_at);
