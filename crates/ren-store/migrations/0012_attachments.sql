-- Files an operator uploaded to a session to go with a prompt. The bytes
-- are files in the daemon's data folder, under the session's own folder;
-- this is what each one is. Images only, for now: their format and size.
-- `sent_at` is when one went out with a prompt; it stays with the session.
CREATE TABLE attachments (
    id TEXT PRIMARY KEY,
    operator_session TEXT NOT NULL,
    name TEXT NOT NULL,
    format TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    size INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    sent_at INTEGER
);
CREATE INDEX attachments_session ON attachments(operator_session, added_at, id);
