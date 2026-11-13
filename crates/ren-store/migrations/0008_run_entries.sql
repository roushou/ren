-- A run's conversation, whole, as its client reported it: each entry is one
-- event (a turn starting or ending, an item, an approval, an error) in the
-- order it came. An item reported again replaces its entry and keeps its
-- place. `event` is the event's JSON, as the run feed sends it.
CREATE TABLE run_entries (
    run TEXT NOT NULL,
    seq INTEGER NOT NULL,
    at INTEGER NOT NULL,
    turn TEXT,
    item TEXT,
    kind TEXT NOT NULL,
    event TEXT NOT NULL,
    PRIMARY KEY (run, seq)
);
CREATE UNIQUE INDEX run_entries_item ON run_entries(run, item) WHERE item IS NOT NULL;
