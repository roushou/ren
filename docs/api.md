# History and API

Read history from the CLI or the daemon's read-only management API.

## CLI

```sh
tgv history --limit 10
tgv history --user alice
tgv sessions
tgv timeline <session-id> --limit 100
tgv turns <session-id>
tgv report <session-id>
tgv turn <turn-id>
tgv activity <call-id>
tgv search "cargo test" --field tool_input
tgv exposures
tgv inspect <call-id>
tgv inspect <call-id> --body response --output response.bin
tgv tail
tgv tail --json
tgv tui
```

Commands emit the same JSON the read API answers (without `api_version`),
whether they read this host's storage or a gateway: `history` and `timeline` a
page `{calls, next}`, `sessions` `{sessions}`, `turns` `{turns}` for a session,
`exposures` `{exposures}`, `report` a
session's [context and cost report](#session-report), `turn` and
`activity` what a turn or a call did (see [activity](#activity)), `search`
matching prompts, replies, and tool calls (see [search](#search)), and `inspect`
one call with its attempts and observation log. Body exports write the exact captured bytes to a new file, refuse
incomplete bodies, and never overwrite existing files. They work while the daemon
runs and after it stops. Use the same data directory (`--data-dir`) for all
commands.

The commands read the daemon they target. With `--gateway`, or else a gateway
named by `TGV_GATEWAY` or saved by `tgv init`, they read through that gateway's
API below and print the same JSON as from its storage; exports use the body
export endpoint. `--config` or `--data-dir` without `--gateway` reads this host's
storage, as does having no gateway at all. A remote read needs the daemon
running.

`tgv tail` follows the [live feed](#live-feed) of the daemon the CLI targets
(its local listener, or the gateway) and prints a line as each call starts (provider, endpoint, and session) and
finishes (model, outcome, status, tokens, and duration), and when an operator
changes the operator state. Each line starts with the time (UTC) and the call.
Attempt and channel records are left out; `--json` prints every
observation instead, one JSON object per line. It starts from now. When the feed
ends, for example on a daemon restart, it reconnects every two seconds from the
last observation it printed, so nothing in between is skipped. It stops on
interrupt, or if the first connection fails.

`tgv tui`, in a CLI built with `--features tui`, shows the same daemon's live
sessions in the terminal: one pane per active session (seen in the last five
minutes, or with a call open), as many as the terminal has room for. Each
pane shows the session's client and provider, the newest turn's replies and
tool calls with their results, a streaming call's reply and tools so far,
what the session is doing (waiting for a response, responding, the client's
turn or yours, inferred from how the last call ended, or how it stopped), the
turn's files read and written and commands run, context and output tokens
from the newest call that reported usage, and alerts (slow response heads,
429s, failed or interrupted calls, unknown endings, secrets sent, repeated
tool failures). The status line counts sessions and open calls, and shows
each client's output tokens over the last four minutes, counted when calls
end: not a live rate. It reads a snapshot of sessions and calls every two
seconds and whenever the feed says a call opened or ended, and asks for
[progress events](#live-feed). A read that fails is shown as `stale` until
one succeeds. Keys: tab and the arrows move between panes, enter zooms into a
session, `p` keeps a session on the wall even when quiet, `?` lists the
keys, and `q` quits. It needs a running daemon; a gateway that cannot be
reached is reported before the screen is taken over.

## Activity

`tgv turn` shows what an agent did for one user prompt, assembled from each of
its calls' activity, derived when the call finished:

- `prompt`: the user's own words, without the context a client adds to user
  messages (Claude Code's `<system-reminder>` blocks and the `Tool loaded.` it
  adds after loading a deferred tool, Codex's environment and `AGENTS.md`
  instructions);
- `calls`: each call's tool calls, with their input, their `effects` (`read_file`,
  `write_file`, `run_command`, `search`, `delegate`, or `fetch`, and the path,
  command, pattern, task, or URL the input names), and the `result` a later
  request in the turn carried (`succeeded`, `failed`, or `unreported`; `null`
  when none answered), plus the start of the model's reply;
- `files_read`, `files_written`, `commands`, and counts of tool calls, failed
  ones, and unanswered ones;
- each call's `metrics` (context, cache read and written, and output tokens,
  request bytes, duration), as in the [session report](#session-report);
- `findings`: `large_tool_output` (a tool result of at least 20,000 characters,
  which every later call carries) and `repeated_failure` (one tool failing on
  one target at least 3 times in the turn).

A tool call is what the model requested, not proof that it ran. A result's
status is what the client reported: Anthropic's `is_error`, or for Codex the
exit code its output states; other Responses results are `unreported`. Effects
come from each client's own tool names and input fields; a tool TGV does not
know has none. Turns are the client's grouping (Claude Code's prompt ID, Codex's
turn ID); Pi sends none, so use `tgv activity <call-id>` for its calls. The daemon derives
each call's activity once, when the call finishes, from up to 16 MiB of each
body, and stores it with secrets redacted; a turn reads at most 100 calls
(`omitted_calls` counts the rest). `limitations` on each call say what could not
be read (`truncated`, `capture_gap`, `not_json`, `unfinished_tool_call`, …), or
`not_yet_derived` for a call that finished moments ago, or whose activity the
daemon is deriving again from its stored bodies. Text is
excerpted; `omitted_chars` says how much was left out, and the full text stays in
the body. Retention removes activity with its call.

`tgv turns <session-id>` lists a session's turns newest first (`--limit`, up to
1000; no paging), each with its call and error counts, tokens, and the start of
its prompt, with secrets redacted. A
turn's `ended_at` is `null` while any of its calls is still running.

## Search

`tgv search <text>` finds what calls contain: the user's prompt (without the
context clients add), the model's reply, tool input (the tool's name, the files
and commands it names, and its input), and tool output. All words must match;
`"quoted text"` matches as a phrase. Punctuation separates words, so search
`"install-vps"` as a phrase. Query syntax is never interpreted: `OR`, `NEAR(`, or
`*` are searched for as text. Filter with `--session`, `--client`, and `--field`
(`prompt`, `reply`, `tool_input`, `tool_output`); `--limit` defaults to 20.

Results are newest call first: each names the call, session, turn, client,
field, and a snippet with matches marked `«like this»`. `indexed_calls` says how
much history is searchable: every finished call, once its activity is derived
(see above). Calls retention removes leave the index too.

## Exposed secrets

`tgv exposures` lists credentials that calls carried, newest first. The daemon
scans every finished call's newest user input (including context the
client adds, such as Claude Code's reminders, since that is sent to the provider
too), tool input and output, and the model's reply, for well-known formats only:
`anthropic_key` (`sk-ant-…`), `openai_key` (`sk-…`, `sk-proj-…`), `github_token`
(`ghp_…`, `github_pat_…`), `aws_access_key` (`AKIA…`), `google_api_key` (`AIza…`),
`slack_token` (`xox…`), `stripe_live_key` (`sk_live_…`), `private_key` (a PEM
block), and `jwt`. It does not guess at random-looking text, so a finding is rarely
a false alarm, but a key in another format is not found.

The secret itself is never stored or shown: an exposure keeps its `kind`, the
`field` it was in, a masked `hint` (the known prefix and last four characters,
`sk-ant-…a1b2`), a `fingerprint` (16 hex characters of its BLAKE3 hash, equal for
repeats of one secret), and how often it appeared. Activity and search keep
text with each secret replaced by `«redacted: kind»`; captured bodies are kept as
recorded. Session reports and `tgv turn` list exposures as `secret_exposure`
findings, and the Overview lists the last day's first under Needs attention. A secret sent to
a provider should be rotated: TGV cannot recall it. The scan covers the newest
input of each call, not the system prompt or older history a client resends.

## Session report

`tgv report <session-id>` shows where a session's context and prompt-cache
tokens went, from its call records alone (no body is read, so it is fast):

- `turns`: for each turn, its calls, the idle time before it (since the
  session's previous call ended), context tokens at its first and last call,
  the cache hit percent (cache reads over context), tokens written to the cache,
  and output tokens;
- `totals` over the session's calls that reported usage, including the peak
  context;
- `findings`, in call order:
  - `cache_rebuild`: a call read less than half of the earlier peak context from
    the cache and wrote at least half of it again (at least 10,000 tokens). It
    reports `idle_before_ms`; a provider's cache entries expire, but TGV cannot
    see its cache, so the cause is not stated.
  - `context_growth`: the 5 largest steps of at least 10,000 tokens past the
    session's earlier peak.

Context is new input plus cache reads and writes for Anthropic, and input for
OpenAI, whose input already counts cached tokens. Findings need an earlier peak
of at least 10,000 tokens, so a session's start is not reported. Peaks are
session-wide, so a subagent's smaller context is neither a rebuild nor growth.
Token counts are provider-reported, not cost or subscription allowance. A report
reads at most 5,000 calls (`omitted_calls` counts the rest).

## MCP server

`tgv mcp` serves this history to coding agents as a
[Model Context Protocol](https://modelcontextprotocol.io) server over stdio, so an
agent can look up what it, or another agent, did:

```sh
claude mcp add --scope user tgv -- tgv mcp
codex mcp add tgv -- tgv mcp
```

Its tools are read-only:

| Tool                | Returns                                                                                                           |
| ------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `search_calls`      | Calls, newest first, filtered as in [call search](#call-search) (`session_id` names the session); `{calls, next}` |
| `list_sessions`     | Recent sessions with their counts and token totals                                                                |
| `session_calls`     | One session's calls, oldest first; `{calls, next}`                                                                |
| `search_history`    | [Full-text search](#search), as `tgv search` returns it                                                           |
| `list_exposures`    | [Exposed secrets](#exposed-secrets), as `tgv exposures` lists them                                                |
| `session_report`    | A session's context and cost report, as `tgv report` shows it                                                     |
| `list_turns`        | A session's turns, newest first, as `tgv turns` lists them                                                        |
| `get_turn`          | What one turn did, as `tgv turn` shows it: prompt, tool calls with effects and results, files, commands           |
| `get_call_activity` | One call's activity, as `tgv activity` shows it                                                                   |
| `get_call`          | One call with its attempts, inspection, and observation log, as `tgv inspect` prints it                           |
| `get_body`          | An excerpt of a [body preview](#body-previews): decoded text (or raw bytes), at most `max_chars` (default 20,000) |
| `usage`             | Calls, errors, and tokens, as [`/api/usage`](#usage)                                                              |

Listings return at most 100 rows. Invalid arguments, unknown IDs, and an
unreachable daemon are tool errors that the agent reads and can correct.

It reads through the management API of the daemon the CLI targets, like
`tgv tail`: the local listener, or the gateway from `--gateway`, `TGV_GATEWAY`,
or `tgv init`. The daemon must be running. Bodies may contain private prompts
and tool output, and an agent reading them sends them to its provider like any
other context. It serves protocol revision `2026-07-28`, where every request
carries its version, and, after an `initialize` handshake, the earlier revisions
`2025-11-25`, `2025-06-18`, `2025-03-26`, and `2024-11-05`. Requests are answered
in order, one at a time; stdout carries only protocol messages and logs go to
stderr.

## Endpoints

Every JSON body carries `"api_version": 1`; errors are `{api_version, error}` with
`error` one of `not_found`, `invalid_query`, `busy`, `unavailable`, or
`identity_refused` (`401`, from `/api/caller` only). An identifier that is not one
(a call ID, or a body direction other than `request` or `response`) is
`not_found`, and an unknown or unreadable query parameter is `invalid_query`. The [control API](#control-api)
(`/control/v1`) is versioned separately.
A body export answers with the raw bytes (`application/octet-stream`) only when
the body was stored whole; otherwise `not_found`. At most two run at once
(`busy`), and the UI does not forward it.

| Endpoint                                                               | Result                                                                                                                                                                                                         |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /health`                                                          | `ok` or `degraded` (HTTP 503), the build, the instance (`environment`: `production` or `development`), a summary of the operator state (pause, and how many sessions and users are blocked), recorder counters |
| `GET /api/metrics`                                                     | Recorder counters and storage usage                                                                                                                                                                            |
| `GET /api/caller`                                                      | How the gateway identifies this request's caller (`kind`, `user`, `login`), as it would a forwarded call; `401` `identity_refused` for a refused token or login                                                |
| `GET /api/storage`                                                     | Active retention policy, disk usage of the database files, calls with derived activity, and the per-body capture limit (`capture.body_limit_bytes`)                                                            |
| `GET /api/calls?limit=100`                                             | Filtered, paginated calls, newest first (see [search](#call-search))                                                                                                                                           |
| `GET /api/calls/{call-id}`                                             | One call with its attempts, inspection, and observation log                                                                                                                                                    |
| `GET /api/calls/{call-id}/body/{request\|response}`                    | Bounded body preview                                                                                                                                                                                           |
| `GET /api/calls/{call-id}/body/{request\|response}/export`             | The complete captured body as raw bytes, as `tgv inspect --body` exports it                                                                                                                                    |
| `GET /api/sessions?limit=100`                                          | Recent sessions with call, turn, and error counts and token totals                                                                                                                                             |
| `GET /api/usage?from_ms=&until_ms=`                                    | Calls, errors, and tokens in total, by UTC hour, by model, and by user (see [usage](#usage))                                                                                                                   |
| `GET /api/sessions/{session-id}/calls?limit=100`                       | One session's calls, oldest first                                                                                                                                                                              |
| `GET /api/sessions/{session-id}/turns?limit=100`                       | A session's turns, newest first, with prompt excerpts (see [activity](#activity))                                                                                                                              |
| `GET /api/search?q=&field=&session=&client=&from_ms=&until_ms=&limit=` | [Full-text search](#search), newest call first                                                                                                                                                                 |
| `GET /api/exposures?session=&from_ms=&until_ms=&limit=`                | [Exposed secrets](#exposed-secrets), newest call first                                                                                                                                                         |
| `GET /api/sessions/{session-id}/report`                                | A session's [context and cost report](#session-report)                                                                                                                                                         |
| `GET /api/turns/{turn-id}`                                             | One turn's [activity](#activity)                                                                                                                                                                               |
| `GET /api/calls/{call-id}/activity`                                    | One call's [activity](#activity)                                                                                                                                                                               |
| `GET /api/live?after={seq}`                                            | Server-sent events as observations commit                                                                                                                                                                      |

Omit `after` for the first page; pass the returned `next` cursor unchanged for the
next one. The CLI accepts the same cursor with `timeline --after`. Pages order by
start time and call ID; refresh from the beginning to see late arrivals that sort
before a cursor. Limits are 1–1000. Recording is asynchronous, so new data appears
shortly after forwarding, typically within the 50 ms commit interval. Busy readers
return `503` without blocking forwarding. Management responses carry
`Cache-Control: no-store`. The UI does not relay full body exports; use `tgv inspect` or the export endpoint. The API shares the
listener's loopback/Tailscale access boundary and has no separate login or
cross-origin browser access enabled.

### Session totals

Each session in `/api/sessions` carries `error_count`, the model calls (`kind=model`)
the `errors` status filter would find, and `tokens: {input, output, cache_read}`, summed from provider-reported
usage. Input includes cached context whichever way each call's provider accounts
for it; a total is `null` when no call reported usage. Token counts are not
subscription allowance or charges.

### Usage

`/api/usage` covers the calls started within an optional inclusive range
(`from_ms`, `until_ms`; without them, every retained call). It returns `totals`, `hours`
(only hours with calls, oldest first, keyed by the hour's UTC start; clients group
them into local days), and `models` (each provider and reported model, most input
first; `model` is `null` when no response reported one), and `users` (each named
user, most input first; `user` is `null` for calls no named user made: unlisted
and unidentified callers, and calls stored without a caller). Each carries `calls`,
`errors` (model calls the `errors` status filter finds, as in session totals), and `tokens: {input, output,
cache_read}`, with input including cached context as in session totals. Unknown
parameters and a reversed range are rejected.

### Builds

`/health` reports the build the daemon runs as `build`: `{"version": "0.1.0",
"revision": "<commit>"}`. The revision is the full git commit the binary was
compiled from, with `-dirty` when tracked files differed from it, or `unknown`
when the build had neither git nor packaging metadata. `tgv --version` prints the
same, and `tgv doctor` compares the two: it is OK only for the same version from
the same unmodified commit, and a warning otherwise. Untracked files are not
counted as modifications. The revision is read when the `tgv-rs` package is
compiled, so after editing a tracked file without staging it, `-dirty` can
appear only after that package is rebuilt.

### Control API

`/control/v1` changes the operator state; see [operator controls](policy.md#operator-controls)
for who may call it. A request with `Origin` or `Sec-Fetch-Site` is refused (`403`
`cross_site`); one with a `Tailscale-User-Login` must name a login in
`control.tailscale_logins` (`403` `forbidden`); any other needs
`Authorization: Bearer <control.token>` (`401` `unauthorized`). Responses are
never cached, and errors are `{api_version, error}`.

`GET /control/v1/state`, and every command that succeeds, answer
`{api_version, state, running}`: `state` is `{revision, pause, blocked_sessions,
blocked_users}` (`pause` is `{"state": "running"}` or `paused` with its
`retry_after`; each block ends at a time (`{"kind": "time", "at": <ms>}`) or when
unblocked (`{"kind": "unblocked"}`)), and `running` lists the running calls
(`call`, `transport`, `user`).

| Endpoint                                      | Body                                           | Notes                                                                                                                    |
| --------------------------------------------- | ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `POST /control/v1/pause`                      | `{"retry_after_seconds": 1–3600}`, required    | `tgv control pause --retry-after` (default 30)                                                                           |
| `POST /control/v1/resume`                     | none                                           |                                                                                                                          |
| `POST /control/v1/sessions/{session}/block`   | optional `{"duration_seconds": 1–604800}`      | `{session}` is the client's session UUID; without a duration, until unblocked                                            |
| `POST /control/v1/sessions/{session}/unblock` | none                                           |                                                                                                                          |
| `POST /control/v1/users/{user}/block`         | optional `{"duration_seconds": 1–604800}`      | only a configured user; any other name is `404` `not_found`                                                              |
| `POST /control/v1/users/{user}/unblock`       | none                                           |                                                                                                                          |
| `POST /control/v1/calls/{call}/cancel`        | optional `{"block_session_seconds": 1–604800}` | `202` with the state, or `200` when the session is blocked too; `404` `not_running`                                      |
| `POST /control/v1/backup`                     | none                                           | `{api_version, backup}` with the copy's counts; `409` `backup_running`, `503` `backup_incomplete`, `500` `backup_failed` |

A malformed body or a value out of range is `400` `invalid_request`, and a
malformed session ID is `404` `not_found`. A command the recorder cannot record
is not made: `503` `unrecorded` when the recorder has no room now (try again),
`500` `unrecordable` when its record cannot be written.

## Live feed

`GET /api/live` streams `observation` events, one per committed observation
(`call_opened`, `attempt_started`, `response_head`, `attempt_finished`,
`call_finished`, `call_recovered`, `channel_closed`, `operator_action`). Each event's `id` is its sequence number. Without
`after`, only new observations are sent; with `after` (or a `Last-Event-ID` header,
which browsers send on reconnect) the feed first replays retained observations after
that position. A `gone` event (`{"oldest_retained": <seq>}`) means the position was
removed by retention: reload and reconnect without `after`. A `lagged` event means the subscriber fell behind
and the feed is catching up from storage. An `error` event means the daemon could not read its store; the feed then ends.
The stream also ends when the daemon shuts down.
Observations describe what was recorded, not model or tool state.

With `progress=true`, the feed also sends `progress` events while it follows:
`{call, reply, cut, tools, thinking}` for a call whose response is streaming, at
most one every 250 ms per call and only when it grew or the model started or
stopped thinking. `reply` is the last 2,000 characters
of the reply so far, with credentials in well-known formats redacted (the last
4,000 are scanned first, so one across the cut is still found); `cut` says the
reply is longer; `tools` names the tool calls begun, without their inputs; `thinking` says the
response so far ends inside a thinking block or reasoning item, whose text is
not sent. They
have no `id`: they are never stored, never replayed on resume, and a feed that
falls behind skips some. The finished call's activity is the record. The UI asks
for them, to show replies on Live cards as they stream, and so does `tgv tui`;
`tgv tail` does not.

## Call search

`GET /api/calls` filters all retained calls and returns `{calls, next}` newest
first. Parameters: `q`, `session` (a TGV session ID), `client` (`claude_code`,
`codex`, `pi`, `opencode`), `user` (a [named user](policy.md#named-users)), `kind` (`model`, `other`), `provider` (`anthropic`, `openai`,
`codex`, `deepseek`), `model`, `tool`, `has_tools`, `status` (`active`, `finished`, `errors`,
`interrupted`), `from_ms`, `until_ms` (inclusive), `limit` (1–1000), and `after`.
Filters combine with AND; unknown parameters are rejected.

- `client` matches the harness whose own headers or metadata attributed the call;
  a call with no attribution matches no client.
- `user` matches the calls a named user made, proven by its Tailscale login or
  token; each call's `caller` says how.
- `kind=model` keeps calls that ask a model for output (messages and responses,
  over HTTP or as a WebSocket message); `kind=other` keeps the rest: token counts,
  model listings, the WebSocket handshakes that open a channel, and a client's
  own checks. Claude Code's quota probe, sent as it starts, is one: a one-token
  request whose only message is `quota`, which Anthropic may answer with `429`.
  A call counts as a model call until its digest says otherwise. Each listed
  call carries that reading as `purpose` (`work` or `quota_probe`), `null` until
  its digest is derived.

- `q` matches call IDs, reported models, and observed tool names,
  case-insensitively; `%` and `_` are literal. Search strings are at most 128 UTF-8
  bytes. Bodies are never searched.
- Model and tool filters use the response inspection saved when a call finishes,
  so active calls and calls without observations don't match them.
- `errors` covers HTTP 4xx/5xx, upstream/stream errors, and reported model failures;
  `interrupted` is separate.
- Pass `next` unchanged with the same filters. The cursor survives retention of its
  source row. Pages are live queries, not snapshots: restart from the first page to
  see newly arrived matches.

## Body previews

`GET /api/calls/{id}/body/{request|response}` returns a bounded preview of the
first 256 KiB, not a full export; use `tgv inspect --body … --output …` for
complete captured bytes. The response body is the final attempt's. Previews decode
gzip and Zstandard (recognised by their magic bytes) and Brotli (when the attempt
recorded `Content-Encoding: br`, since Brotli has no magic bytes; `decoding.applied`
is then `brotli`) where possible and report the extent (`complete`, `partial`,
`truncated`, or `gap`), the observed size (`null` while the body is still open),
and decoding integrity explicitly. Preview work is bounded and independent of
forwarding. Bodies may contain private prompts or tool results and share the
management API's private access boundary. The UI renders them as inert text and
never stores them; see [UI payload previews](../ui/README.md#payload-previews) for
limits.
