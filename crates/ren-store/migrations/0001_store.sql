-- Ren's metadata store, `ren.sqlite3`. Body chunks are in `blobs.sqlite3`,
-- which the store creates beside it.

-- Connections: an HTTP request's, or a WebSocket channel carrying many calls.
CREATE TABLE channels (
    id TEXT PRIMARY KEY,
    transport TEXT NOT NULL,
    opened_at INTEGER NOT NULL,
    provider TEXT,
    closed_at INTEGER,
    ending TEXT,
    -- The close code of the frame that ended the channel, sent or forwarded.
    close_code INTEGER,
    -- Messages that arrived while no call was open: counted, not stored.
    unattributed_messages INTEGER NOT NULL DEFAULT 0,
    unattributed_bytes INTEGER NOT NULL DEFAULT 0
);

-- Calls grouped by the session identifier their client sent, per provider
-- and attribution source: the same identifier sent to two providers is two
-- recordings.
CREATE TABLE recordings (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    source TEXT NOT NULL,
    native_id TEXT NOT NULL,
    first_seen INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    archived_at INTEGER,
    UNIQUE (provider, source, native_id)
);
CREATE INDEX recordings_recent ON recordings(last_seen DESC, id DESC);
CREATE INDEX recordings_native ON recordings(native_id, first_seen, id);

CREATE TABLE turns (
    id TEXT PRIMARY KEY,
    recording TEXT NOT NULL REFERENCES recordings(id),
    native_id TEXT NOT NULL,
    UNIQUE (recording, native_id)
);

CREATE TABLE calls (
    id TEXT PRIMARY KEY,
    channel TEXT NOT NULL REFERENCES channels(id),
    started_at INTEGER NOT NULL,
    provider TEXT NOT NULL,
    wire TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    method TEXT NOT NULL,
    config_revision TEXT NOT NULL,
    recording TEXT REFERENCES recordings(id),
    turn TEXT REFERENCES turns(id),
    attribution TEXT NOT NULL,
    -- Who called: a caller_kind label, with the named user and Tailscale
    -- login when there are ones.
    caller TEXT NOT NULL,
    user TEXT,
    tailscale_login TEXT,
    state TEXT NOT NULL,
    ended_at INTEGER,
    -- Why Ren answered without forwarding (a call_rejection label); NULL for
    -- a forwarded call.
    rejection TEXT,
    -- The operator state revision the call was planned against.
    operator_revision INTEGER,
    -- The routing plan, as JSON: candidates, transform rules, fallback.
    plan TEXT,
    -- Copied from the final attempt, for listing and filtering.
    final_attempt TEXT,
    terminal TEXT,
    status INTEGER,
    model TEXT,
    outcome TEXT,
    stop_reason TEXT,
    tool_count INTEGER NOT NULL DEFAULT 0,
    -- Token usage the provider reported: its input accounting (a label) and
    -- each count. A call reported usage when `input_accounting` is not NULL.
    input_accounting TEXT,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cache_read_tokens INTEGER,
    cache_creation_tokens INTEGER,
    reasoning_tokens INTEGER,
    capture_complete INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX calls_recent ON calls(started_at DESC, id DESC);
CREATE INDEX calls_provider ON calls(provider, started_at DESC, id DESC);
CREATE INDEX calls_retained ON calls(started_at, id) WHERE state <> 'open';
CREATE INDEX calls_recording ON calls(recording, started_at, id);
CREATE INDEX calls_turn ON calls(turn, started_at, id) WHERE turn IS NOT NULL;
CREATE INDEX calls_user ON calls(user, started_at DESC, id DESC) WHERE user IS NOT NULL;

CREATE TABLE attempts (
    id TEXT PRIMARY KEY,
    call TEXT NOT NULL REFERENCES calls(id),
    ordinal INTEGER NOT NULL,
    started_at INTEGER NOT NULL,
    upstream TEXT NOT NULL,
    provider TEXT NOT NULL,
    status INTEGER,
    response_format TEXT,
    response_encoding TEXT,
    terminal TEXT,
    -- Why the attempt ended and how far it had got.
    terminal_cause TEXT,
    terminal_phase TEXT,
    ended_at INTEGER,
    inspection TEXT,
    inspection_version INTEGER,
    -- What Ren changed in this attempt's request, as JSON: the credential
    -- override and each transform step with its outcome. Header values are
    -- never recorded. A changed body is stored as the attempt's own request
    -- body (bodies.owner = the attempt, direction = 'request').
    changes TEXT,
    UNIQUE (call, ordinal)
);

-- The tools a call's final attempt requested, in order.
CREATE TABLE call_tools (
    call TEXT NOT NULL REFERENCES calls(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (call, position)
) WITHOUT ROWID;

-- A body's capture: how much was observed and stored, and how it ended. Its
-- bytes are segments, each a slice of a chunk in `blobs.sqlite3`.
CREATE TABLE bodies (
    owner TEXT NOT NULL,
    direction TEXT NOT NULL,
    observed_bytes INTEGER,
    stored_bytes INTEGER NOT NULL DEFAULT 0,
    ending TEXT NOT NULL DEFAULT 'open',
    PRIMARY KEY (owner, direction)
) WITHOUT ROWID;

CREATE TABLE segments (
    owner TEXT NOT NULL,
    direction TEXT NOT NULL,
    byte_offset INTEGER NOT NULL,
    len INTEGER NOT NULL,
    chunk BLOB NOT NULL,
    PRIMARY KEY (owner, direction, byte_offset)
) WITHOUT ROWID;
CREATE INDEX segments_chunk ON segments(chunk);

-- WebSocket message boundaries within a body.
CREATE TABLE messages (
    owner TEXT NOT NULL,
    direction TEXT NOT NULL,
    byte_offset INTEGER NOT NULL,
    len INTEGER NOT NULL,
    payload TEXT NOT NULL,
    at_wall INTEGER NOT NULL,
    PRIMARY KEY (owner, direction, byte_offset)
) WITHOUT ROWID;

-- What each finished call showed, derived by the recorder.
CREATE TABLE call_digests (
    call TEXT PRIMARY KEY REFERENCES calls(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    -- Why the client sent the call (a call_purpose label).
    purpose TEXT NOT NULL,
    -- The user's own words, for turn listings, without parsing `digest`.
    prompt TEXT,
    prompt_omitted INTEGER,
    -- The CallDigest, as JSON.
    digest TEXT NOT NULL
) WITHOUT ROWID;

-- Credentials a call carried, by a masked hint and a fingerprint.
CREATE TABLE call_exposures (
    call TEXT NOT NULL REFERENCES calls(id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    kind TEXT NOT NULL,
    hint TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    count INTEGER NOT NULL
);
CREATE INDEX call_exposures_call ON call_exposures(call);

-- A call's searchable texts, redacted, and their full-text index.
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

-- The observation log: every recorded fact, in order.
CREATE TABLE observations (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at_wall INTEGER NOT NULL,
    at_mono INTEGER NOT NULL,
    channel TEXT NOT NULL,
    call TEXT,
    attempt TEXT,
    kind TEXT NOT NULL,
    schema INTEGER NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX observations_call ON observations(call, seq);

CREATE TABLE operator_actions (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at_wall INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    actor TEXT NOT NULL,
    command TEXT NOT NULL
);

CREATE TABLE operator_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL,
    state TEXT NOT NULL
);

-- Sessions an operator works in: a folder and a client, with the settings
-- its next run starts with.
CREATE TABLE operator_sessions (
    id TEXT PRIMARY KEY,
    harness TEXT NOT NULL,
    workdir TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    model TEXT,
    effort TEXT,
    approvals TEXT,
    archived_at INTEGER
);
CREATE INDEX operator_sessions_created ON operator_sessions(created_at DESC, id DESC);

-- One client process Ren started and drove, serving an operator session.
CREATE TABLE runs (
    id TEXT PRIMARY KEY,
    operator_session TEXT NOT NULL,
    harness TEXT NOT NULL,
    workdir TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    -- The client's own conversation, once it opened one.
    thread TEXT,
    -- The settings it started on; NULL is the client's own.
    model TEXT,
    effort TEXT,
    approvals TEXT,
    -- The toolset generation on its search path; NULL when unknown.
    toolset_generation INTEGER,
    ended_at INTEGER,
    run_end TEXT,
    -- How the process exited: 'code' or 'signal' with its value, or
    -- 'unknown' when the host could not say. NULL when no process exited
    -- (it never started, or the run has not ended or was recovered).
    exit_kind TEXT,
    exit_value INTEGER,
    archived_at INTEGER
);
CREATE INDEX runs_started ON runs(started_at DESC, id DESC);
CREATE INDEX runs_operator_session ON runs(operator_session, started_at);
CREATE INDEX runs_thread ON runs(thread, started_at) WHERE thread IS NOT NULL;

-- A run's conversation: every event its client reported, an item reported
-- again (finished, completed) replacing its earlier entry.
CREATE TABLE run_entries (
    run TEXT NOT NULL,
    seq INTEGER NOT NULL,
    -- When it was last reported, and when first.
    at INTEGER NOT NULL,
    started_at INTEGER NOT NULL,
    turn TEXT,
    item TEXT,
    -- The event's kind (a drive_event_kind label) and the event, as JSON.
    kind TEXT NOT NULL,
    event TEXT NOT NULL,
    -- A prompt's text, when the entry is a prompt.
    prompt TEXT,
    PRIMARY KEY (run, seq)
);
CREATE UNIQUE INDEX run_entries_item ON run_entries(run, item) WHERE item IS NOT NULL;
CREATE INDEX run_entries_prompt ON run_entries(run, at) WHERE prompt IS NOT NULL;
CREATE INDEX run_entries_turn_started ON run_entries(run, at) WHERE kind = 'turn_started';

-- Images uploaded to an operator session; their bytes are files in the
-- daemon's attachments folder.
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
