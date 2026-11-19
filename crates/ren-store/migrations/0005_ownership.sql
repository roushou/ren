-- Ownership (docs/plans/hierarchy.md, phase 2): the project a session
-- works for, and the project a work item is for. NULL is unassigned: a
-- row from before projects, or a session outside every project.
ALTER TABLE operator_sessions ADD COLUMN project TEXT;
ALTER TABLE work_items ADD COLUMN project TEXT;
CREATE INDEX operator_sessions_project ON operator_sessions(project, created_at DESC, id DESC);
CREATE INDEX work_items_project ON work_items(project, updated_at DESC, id DESC);
