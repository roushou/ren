-- What each relayed run reached through the relay: one row per request it
-- made of the relay, as it ended. `asked` is a tunnel's host:port, or the
-- method of a request that was not one; `end` a passage_end label;
-- `through` the listed destination that admitted it, NULL when refused;
-- `listed_by` a purpose label, why that destination is listed, NULL when
-- refused or unknown; `sent` and `received` the bytes each way, NULL
-- unless it closed.
CREATE TABLE run_passages (
    run TEXT NOT NULL,
    seq INTEGER NOT NULL,
    at INTEGER NOT NULL,
    asked TEXT NOT NULL,
    end TEXT NOT NULL,
    through TEXT,
    listed_by TEXT,
    sent INTEGER,
    received INTEGER,
    PRIMARY KEY (run, seq)
);
