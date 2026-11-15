-- A turn's calls, in order: listing a turn, counting its calls, and the
-- retention sweep's check for turns left without calls.
CREATE INDEX calls_turn ON calls(turn, started_at, id) WHERE turn IS NOT NULL;

-- Runs on one client thread: a run's conversation across the runs that
-- resumed it, and recording a thread once a run learns it.
CREATE INDEX runs_thread ON runs(thread, started_at) WHERE thread IS NOT NULL;
