-- Work items: what an operator marked to be done, and where each stands,
-- from queued to shipped.
CREATE TABLE work_items (
    id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    -- Its first note's first line.
    title TEXT NOT NULL,
    -- What was marked (the page, the marks, and the evidence), as JSON.
    work_order TEXT NOT NULL,
    -- A work_state label: queued, sent, landed, shipped, or dismissed.
    state TEXT NOT NULL,
    -- The session that took it up, once sent.
    operator_session TEXT,
    -- The commit its change landed on main as, and the build production
    -- ran with it.
    landed_commit TEXT,
    shipped_revision TEXT
);
CREATE INDEX work_items_updated ON work_items(updated_at DESC, id DESC);
