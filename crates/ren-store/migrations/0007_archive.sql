-- Archiving: when the operator archived a session or a run, or NULL. A
-- session counts as archived only while it has no traffic newer than that
-- (`archived_at >= last_seen`), so a session that comes back to life is shown
-- again. A run counts as archived once it has ended.
ALTER TABLE sessions ADD COLUMN archived_at INTEGER;
ALTER TABLE runs ADD COLUMN archived_at INTEGER;
