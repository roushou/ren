-- TGV store schema.
--
-- The observation log is the record of what happened; the tables below it are
-- query views written in the same transaction. Labels and recorded documents
-- are plain text without CHECK constraints, so a newer binary's values survive
-- in older ones.

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

-- Native identifiers are client-supplied hints, scoped by provider and source:
-- the same identifier sent to two providers is two sessions.
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    source TEXT NOT NULL,
    native_id TEXT NOT NULL,
    first_seen INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    UNIQUE (provider, source, native_id)
);
CREATE INDEX sessions_recent ON sessions(last_seen DESC, id DESC);

CREATE TABLE turns (
    id TEXT PRIMARY KEY,
    session TEXT NOT NULL REFERENCES sessions(id),
    native_id TEXT NOT NULL,
    UNIQUE (session, native_id)
);

-- A channel row exists for every call; only WebSocket channels record an end.
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

CREATE TABLE calls (
    id TEXT PRIMARY KEY,
    channel TEXT NOT NULL REFERENCES channels(id),
    started_at INTEGER NOT NULL,
    provider TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    method TEXT NOT NULL,
    config_revision TEXT NOT NULL,
    session TEXT REFERENCES sessions(id),
    turn TEXT REFERENCES turns(id),
    attribution TEXT NOT NULL,
    state TEXT NOT NULL,
    ended_at INTEGER,
    -- Why TGV answered without forwarding (a call_rejection label); NULL for
    -- a forwarded call.
    rejection TEXT,
    -- The operator state revision the call was planned against.
    operator_revision INTEGER,
    -- The routing plan, as JSON: candidates, transform rules, fallback.
    plan TEXT,
    -- Search columns, copied from the final attempt.
    final_attempt TEXT,
    terminal TEXT,
    status INTEGER,
    model TEXT,
    outcome TEXT,
    stop_reason TEXT,
    tool_names TEXT NOT NULL DEFAULT '[]',
    tool_count INTEGER NOT NULL DEFAULT 0,
    usage TEXT,
    capture_complete INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX calls_recent ON calls(started_at DESC, id DESC);
CREATE INDEX calls_session ON calls(session, started_at, id);
CREATE INDEX calls_provider ON calls(provider, started_at DESC, id DESC);
CREATE INDEX calls_retained ON calls(started_at, id) WHERE state <> 'open';

CREATE TABLE attempts (
    id TEXT PRIMARY KEY,
    call TEXT NOT NULL REFERENCES calls(id),
    ordinal INTEGER NOT NULL,
    started_at INTEGER NOT NULL,
    upstream TEXT NOT NULL,
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
    -- What TGV changed in this attempt's request, as JSON: the credential
    -- override and each transform step with its outcome. Header values are
    -- never recorded. A changed body is stored as the attempt's own request
    -- body (bodies.owner = the attempt, direction = 'request').
    changes TEXT,
    UNIQUE (call, ordinal)
);

-- A captured byte stream: a call's request or an attempt's response.
-- stored_bytes counts committed segments; fewer than observed means omissions.
-- observed_bytes is unknown (NULL) until the body's end is recorded.
CREATE TABLE bodies (
    owner TEXT NOT NULL,
    direction TEXT NOT NULL,
    observed_bytes INTEGER,
    stored_bytes INTEGER NOT NULL DEFAULT 0,
    ending TEXT NOT NULL DEFAULT 'open',
    PRIMARY KEY (owner, direction)
) WITHOUT ROWID;

-- Offsets keep gaps explicit: missing capture is never joined into a body
-- that looks complete. Chunks live in blobs.sqlite3, keyed by blake3 hash.
CREATE TABLE segments (
    owner TEXT NOT NULL,
    direction TEXT NOT NULL,
    byte_offset INTEGER NOT NULL,
    len INTEGER NOT NULL,
    chunk BLOB NOT NULL,
    PRIMARY KEY (owner, direction, byte_offset)
) WITHOUT ROWID;
CREATE INDEX segments_chunk ON segments(chunk);

-- Message boundaries inside a body. A WebSocket call's request body is its
-- client messages back to back; its attempt's response body is the upstream
-- messages. Each row marks where one message starts and how long it is.
CREATE TABLE messages (
    owner TEXT NOT NULL,
    direction TEXT NOT NULL,
    byte_offset INTEGER NOT NULL,
    len INTEGER NOT NULL,
    payload TEXT NOT NULL,
    at_wall INTEGER NOT NULL,
    PRIMARY KEY (owner, direction, byte_offset)
) WITHOUT ROWID;

-- Operator commands, in order. The revision is the state each one made.
CREATE TABLE operator_actions (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at_wall INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    actor TEXT NOT NULL,
    command TEXT NOT NULL
);

-- The current operator state, so a restart keeps pauses and session blocks
-- and revisions continue. One row, written in the same transaction as the
-- operator action that made it. The state is the control API's JSON form of
-- OperatorState.
CREATE TABLE operator_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL,
    state TEXT NOT NULL
);
