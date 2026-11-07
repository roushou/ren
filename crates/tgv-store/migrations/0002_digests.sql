-- What each finished call's bodies showed, derived once by the recorder:
-- every text redacted and cut. A digest of an older version is derived again
-- from the stored bodies. Deleting a call deletes its derived rows.
CREATE TABLE call_digests (
    call TEXT PRIMARY KEY REFERENCES calls(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    -- The user's own words, for turn listings, without parsing `digest`.
    prompt TEXT,
    prompt_omitted INTEGER,
    -- The CallDigest, as JSON.
    digest TEXT NOT NULL
) WITHOUT ROWID;

-- Secrets a call carried: never the secret, only its kind, hint, and
-- fingerprint, per field.
CREATE TABLE call_exposures (
    call TEXT NOT NULL REFERENCES calls(id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    kind TEXT NOT NULL,
    hint TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    count INTEGER NOT NULL
);
CREATE INDEX call_exposures_call ON call_exposures(call);

-- Each call's searchable texts, and their full-text index. The index holds
-- no copy of the text: `search_texts` is its content table, kept in step by
-- the triggers below.
CREATE TABLE search_texts (
    id INTEGER PRIMARY KEY,
    call TEXT NOT NULL REFERENCES calls(id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX search_texts_call ON search_texts(call);

CREATE VIRTUAL TABLE search USING fts5(
    text,
    content = 'search_texts',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER search_texts_added AFTER INSERT ON search_texts BEGIN
    INSERT INTO search(rowid, text) VALUES (new.id, new.text);
END;

CREATE TRIGGER search_texts_removed AFTER DELETE ON search_texts BEGIN
    INSERT INTO search(search, rowid, text) VALUES ('delete', old.id, old.text);
END;
