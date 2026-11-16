-- A recording (the calls Ren grouped under one client session identifier)
-- was stored as a "session", the word operator sessions now use alone.
-- Renamed in place: SQLite rewrites the references to the table and its
-- columns, and no row is copied.
ALTER TABLE sessions RENAME TO recordings;
ALTER TABLE calls RENAME COLUMN session TO recording;
ALTER TABLE turns RENAME COLUMN session TO recording;

DROP INDEX sessions_recent;
CREATE INDEX recordings_recent ON recordings(last_seen DESC, id DESC);
DROP INDEX sessions_native;
CREATE INDEX recordings_native ON recordings(native_id, first_seen, id);
DROP INDEX calls_session;
CREATE INDEX calls_recording ON calls(recording, started_at, id);
