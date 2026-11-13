-- Runs: agent processes Ren started in a folder and drove through their
-- client's structured mode. A run's thread is the client's own conversation id,
-- known once the client opened it. A run without an end is still running, or
-- was left by a daemon that stopped; startup marks those recovered.
CREATE TABLE runs (
    id TEXT PRIMARY KEY,
    harness TEXT NOT NULL,
    workdir TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    thread TEXT,
    ended_at INTEGER,
    run_end TEXT,
    -- How the process exited: 'code' or 'signal' with its value, or
    -- 'unknown' when the host could not say. NULL when no process exited
    -- (it never started, or the run has not ended or was recovered).
    exit_kind TEXT,
    exit_value INTEGER
);
CREATE INDEX runs_started ON runs(started_at DESC, id DESC);

-- A run's session is the session its client named with the run's thread id
-- (Codex sends its thread id as its session id).
CREATE INDEX sessions_native ON sessions(native_id, first_seen, id);
