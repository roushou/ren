-- When each conversation entry was first reported. An item reported again
-- (a command that finished, a reply that completed) replaces its entry and
-- its `at`, the time it was last reported; `started_at` stays, so how long
-- a step took can be read. Entries stored before this keep their last time
-- as both.
ALTER TABLE run_entries ADD COLUMN started_at INTEGER NOT NULL DEFAULT 0;
UPDATE run_entries SET started_at = at;
