-- A work item's preview: its session's branch on a copy of production's
-- history, where the marked page can be opened. Both NULL while none is up.
ALTER TABLE work_items ADD COLUMN preview_url TEXT;
ALTER TABLE work_items ADD COLUMN preview_revision TEXT;
