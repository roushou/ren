-- Fields the read queries filter and sum on, as columns rather than inside
-- JSON, so a usage report or a tool filter does not parse every row.

-- A call's token usage: one column per count, and its input accounting
-- (a label). A call reported usage when `input_accounting` is not NULL.
ALTER TABLE calls ADD COLUMN input_accounting TEXT;
ALTER TABLE calls ADD COLUMN input_tokens INTEGER;
ALTER TABLE calls ADD COLUMN output_tokens INTEGER;
ALTER TABLE calls ADD COLUMN cache_read_tokens INTEGER;
ALTER TABLE calls ADD COLUMN cache_creation_tokens INTEGER;
ALTER TABLE calls ADD COLUMN reasoning_tokens INTEGER;
UPDATE calls SET
    input_accounting = json_extract(usage, '$.accounting'),
    input_tokens = json_extract(usage, '$.input_tokens'),
    output_tokens = json_extract(usage, '$.output_tokens'),
    cache_read_tokens = json_extract(usage, '$.cache_read_tokens'),
    cache_creation_tokens = json_extract(usage, '$.cache_creation_tokens'),
    reasoning_tokens = json_extract(usage, '$.reasoning_tokens')
WHERE usage IS NOT NULL;
ALTER TABLE calls DROP COLUMN usage;

-- The tools a call's response requested, in order.
CREATE TABLE call_tools (
    call TEXT NOT NULL REFERENCES calls(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    PRIMARY KEY (call, position)
) WITHOUT ROWID;
INSERT INTO call_tools(call, position, name)
SELECT c.id, t.key, t.value FROM calls c, json_each(c.tool_names) t;
ALTER TABLE calls DROP COLUMN tool_names;

-- A run entry's prompt text, when the entry is a prompt, for the sessions
-- list's first prompt.
ALTER TABLE run_entries ADD COLUMN prompt TEXT;
UPDATE run_entries SET prompt = json_extract(event, '$.item.read.text')
WHERE kind = 'item' AND json_extract(event, '$.item.read.as') = 'prompt';
CREATE INDEX run_entries_prompt ON run_entries(run, at) WHERE prompt IS NOT NULL;

-- A session's first turn, for the sessions list, without reading every entry.
CREATE INDEX run_entries_turn_started ON run_entries(run, at) WHERE kind = 'turn_started';
