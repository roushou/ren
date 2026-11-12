# History and API

Read history from the CLI or the daemon's read-only management API.

## CLI

```sh
ren history --limit 10
ren history --user alice
ren sessions
ren timeline <session-id> --limit 100
ren turns <session-id>
ren report <session-id>
ren turn <turn-id>
ren activity <call-id>
ren search "cargo test" --field tool_input
ren exposures
ren inspect <call-id>
ren inspect <call-id> --body response --output response.bin
ren tail
ren tail --json
ren tui
```

`ren tui` and `ren runs` (with no subcommand) open the same terminal app, built
with the `tui` feature: the session list on the left, the selected session on
the right, the gateway's traffic in the top bar, and the keys that matter now in
the bar at the bottom.

- The list groups every session worth a look by the folder each run works
  in: the folders with a run that needs you first (marked `◆` with how
  many), each listing its runs by what they need, then the newest, under
  what each was asked, with its three newest ended runs. Sessions started
  outside Ren follow, under "started
  elsewhere": Ren does not know their folder. It can be grouped by what each
  needs instead (needs you, working, your turn, watched, ended), from the
  action menu. The arrows (or `j` and `k`) move through it, a digit jumps to
  an entry, and `/` searches it by folder, project, or prompt. Until you choose, the selection
  follows the top of the list: what needs you first.
- The selected session shows its conversation, its activity (each command,
  edit, and tool step), the files it wrote and read, and its model calls
  through the gateway (time, model, how each ended, tokens); Tab moves between
  them. A run's conversation is shown whole, every turn open, as one scroll;
  Enter (or `→`) moves into the session, where ↑ and ↓ (or `j` and `k`)
  scroll it a line at a time and PgUp and PgDn a page.
- Keys come in three layers, so few letters do anything: moving (the arrows
  or `j`/`k`/`h`/`l`, Enter and Esc, Tab, digits, `/`), a few verbs (`i`
  write, `n` new run, Space for the action menu, `?` keys, `q` quit), and the
  answers to an approval, which act only while one waits. Everything else is
  in the action menu (Space): it lists only what applies to the selection
  (interrupt or stop a live run; archive or delete an ended run or a
  watched session; group the list; preview the next theme), each with a letter
  that works inside the menu only. A delete asks once more there.
- `i` writes to the selected run: Enter sends, Ctrl-J starts a new line,
  ↑ and ↓ move through the wrapped rows, Ctrl-E opens the text in your editor
  (`$VISUAL`, then `$EDITOR`, then `vi`) and takes back what it saves,
  Ctrl-P and Ctrl-N (or ↑ and ↓ with nothing typed) bring back what you sent,
  and Esc closes the box, keeping a draft. The box grows with the text, up to
  half the screen, then scrolls. Pasted text keeps its line breaks (bracketed
  paste), so an Enter inside a paste does not send it.
  Written while a turn runs, a message waits and is sent when the turn ends;
  written to a run that ended, it continues that conversation in a new run;
  a session Ren did not start takes nothing.
- An approval shows as a card in the conversation: while it waits, `y`
  allows it once, `a` for the rest of the run, and `d` denies it; the action
  menu also denies it and stops the turn; writing instead denies it and sends
  your words next. Answering one moves
  on to the next run that needs you.
- `n` opens the new-run composer, full screen, on where the run works: the
  folder is always chosen, never assumed, from a list (type to search) with
  the folders recent runs worked in first. Then the prompt has the whole
  body, as long as it needs to be, with the same editing keys as writing to
  a run (Ctrl-E for your editor, pasting with line breaks); Enter starts the
  run, or, with no folder chosen, opens the folder list. The header shows the
  folder and the client with the key that changes each, which works while
  typing: Ctrl-O the folder, Ctrl-L the client (the last one used until
  changed). A line under the prompt is kept for the files a prompt carries;
  Ren cannot add files yet. The web UI's new-run form follows the same rule:
  no folder is assumed, and Start without one opens its picker.
- `?` lists every key, by layer, and `q` quits, leaving runs running; Ctrl-C quits only when pressed twice in a
  row, so a stray press does not.

A run's conversation is what the daemon stored of it, whole, read once when
the run is selected, with what streams in over the run feed merged on top:
the same events, so nothing shows twice and nothing is cut. A reply still
streaming when its turn ends is stored as far as it came.

Runs (see [runs](#runs)) also have their own commands, through the control API:

```sh
ren runs start codex ~/dev/acme
ren runs start claude ~/dev/acme --prompt "fix the failing test"
ren runs ls
ren runs projects                                # where a run can start
ren runs say <run-id> fix the failing test
ren runs answer <run-id> <approval-id>          # or: decline, cancel, accept-for-session
ren runs interrupt <run-id>
ren runs stop <run-id>
ren runs watch --run <run-id>
ren runs history
```

`start` takes the current folder when none is given and `--resume <thread>` to
continue a client conversation; through a gateway the folder must be an
absolute path on the gateway's host. `watch` prints each run update as one
JSON line. `history` reads recorded runs like the other history commands.

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
named by `REN_GATEWAY` or saved by `ren init`, they read through that gateway's
API below and print the same JSON as from its storage; exports use the body
export endpoint. `--config` or `--data-dir` without `--gateway` reads this host's
storage, as does having no gateway at all. A remote read needs the daemon
running.

`ren tail` follows the [live feed](#live-feed) of the daemon the CLI targets
(its local listener, or the gateway) and prints a line as each call starts (provider, endpoint, and session) and
finishes (model, outcome, status, tokens, and duration), and when an operator
changes the operator state. Each line starts with the time (UTC) and the call.
Attempt and channel records are left out; `--json` prints every
observation instead, one JSON object per line. It starts from now. When the feed
ends, for example on a daemon restart, it reconnects every two seconds from the
last observation it printed, so nothing in between is skipped. It stops on
interrupt, or if the first connection fails.

A watched session reads as the web UI's Live wall reads it: the newest turn
and the one before it (for a client that sends no turns, such as OpenCode, its
newest four calls), call by call, with the user's prompt (`›`), the tool
results the request carried back (`⎿`), the reply (code blocks drawn behind a
rule), and the tool calls the model asked for (`●`), then a streaming call's
reply and tools so far. Its header is lit green while a response streams, and
amber or red when the newest call needs a look (a slow response head, a 429, a
failed, interrupted, or unknown ending, a secret sent, repeated tool
failures); under its conversation, what the session is doing (waiting for the
first byte, streaming, complete with the client running tools or waiting for
you, inferred from how the last call ended, or how it stopped). The top bar
counts sessions and open calls and shows each client's output tokens over the
last four minutes, counted when calls end: not a live rate. The app reads a
snapshot of sessions and calls every two seconds and whenever the feed says a
call opened or ended, and asks for [progress events](#live-feed). A read that
fails is shown as `stale` until one succeeds. It needs a running daemon; a
gateway that cannot be reached is reported before the screen is taken over.
Runs need control access (this host's control token, or an allowed Tailscale
login through a gateway) and `[runs]` roots on the daemon; without them the
app still shows every session, and says why runs cannot be driven.

The default `terminal` theme uses the terminal's foreground, background, and
ANSI palette. `dark` and `light` use fixed RGB palettes and need a true-color
terminal. Choose a theme for one run, or save this machine's preference:

```sh
ren tui --theme dark
ren tui --save-theme light
ren tui --save-theme terminal
```

`--save-theme` saves and exits without connecting to a daemon. The preference
lives in `$XDG_CONFIG_HOME/ren/tui.toml` (or `~/.config/ren/tui.toml`), separate
from gateway and daemon settings:

```toml
theme = "light"
```

`--theme` overrides the saved preference; without either, the theme is
`terminal`. Choose the next theme in the action menu (Space, then `t`), or
press `t` in the help, to cycle terminal → dark → light. This preview lasts for the current run and does
not save. A non-empty `NO_COLOR` selects monochrome when there is no explicit
flag or saved preference; an explicit choice, including `t`, enables that
theme. Labels and focus emphasis remain available without color.

## Activity

`ren turn` shows what an agent did for one user prompt, assembled from each of
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
come from each client's own tool names and input fields; a tool Ren does not
know has none. Turns are the client's grouping (Claude Code's prompt ID, Codex's
turn ID); Pi sends none, so use `ren activity <call-id>` for its calls. The daemon derives
each call's activity once, when the call finishes, from up to 16 MiB of each
body, and stores it with secrets redacted; a turn reads at most 100 calls
(`omitted_calls` counts the rest). `limitations` on each call say what could not
be read (`truncated`, `capture_gap`, `not_json`, `unfinished_tool_call`, …), or
`not_yet_derived` for a call that finished moments ago, or whose activity the
daemon is deriving again from its stored bodies. Text is
excerpted; `omitted_chars` says how much was left out, and the full text stays in
the body. Retention removes activity with its call.

`ren turns <session-id>` lists a session's turns newest first (`--limit`, up to
1000; no paging), each with its call and error counts, tokens, and the start of
its prompt, with secrets redacted. A
turn's `ended_at` is `null` while any of its calls is still running.

## Search

`ren search <text>` finds what calls contain: the user's prompt (without the
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

`ren exposures` lists credentials that calls carried, newest first. The daemon
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
recorded. Session reports and `ren turn` list exposures as `secret_exposure`
findings, and the Overview lists the last day's first under Needs attention. A secret sent to
a provider should be rotated: Ren cannot recall it. The scan covers the newest
input of each call, not the system prompt or older history a client resends.

## Session report

`ren report <session-id>` shows where a session's context and prompt-cache
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
    reports `idle_before_ms`; a provider's cache entries expire, but Ren cannot
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

`ren mcp` serves this history to coding agents as a
[Model Context Protocol](https://modelcontextprotocol.io) server over stdio, so an
agent can look up what it, or another agent, did:

```sh
claude mcp add --scope user ren -- ren mcp
codex mcp add ren -- ren mcp
```

Its tools are read-only:

| Tool                | Returns                                                                                                           |
| ------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `search_calls`      | Calls, newest first, filtered as in [call search](#call-search) (`session_id` names the session); `{calls, next}` |
| `list_sessions`     | Recent sessions with their counts and token totals                                                                |
| `session_calls`     | One session's calls, oldest first; `{calls, next}`                                                                |
| `search_history`    | [Full-text search](#search), as `ren search` returns it                                                           |
| `list_exposures`    | [Exposed secrets](#exposed-secrets), as `ren exposures` lists them                                                |
| `session_report`    | A session's context and cost report, as `ren report` shows it                                                     |
| `list_turns`        | A session's turns, newest first, as `ren turns` lists them                                                        |
| `get_turn`          | What one turn did, as `ren turn` shows it: prompt, tool calls with effects and results, files, commands           |
| `get_call_activity` | One call's activity, as `ren activity` shows it                                                                   |
| `get_call`          | One call with its attempts, inspection, and observation log, as `ren inspect` prints it                           |
| `get_body`          | An excerpt of a [body preview](#body-previews): decoded text (or raw bytes), at most `max_chars` (default 20,000) |
| `usage`             | Calls, errors, and tokens, as [`/api/usage`](#usage)                                                              |

Listings return at most 100 rows. Invalid arguments, unknown IDs, and an
unreachable daemon are tool errors that the agent reads and can correct.

It reads through the management API of the daemon the CLI targets, like
`ren tail`: the local listener, or the gateway from `--gateway`, `REN_GATEWAY`,
or `ren init`. The daemon must be running. Bodies may contain private prompts
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

| Endpoint                                                               | Result                                                                                                                                                                                                          |
| ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /health`                                                          | `ok` or `degraded` (HTTP 503), the build, the instance (`environment`: `production` or `development`), a summary of the operator state (pause, and how many sessions and users are blocked), recorder counters  |
| `GET /api/metrics`                                                     | Recorder counters and storage usage                                                                                                                                                                             |
| `GET /api/caller`                                                      | How the gateway identifies this request's caller (`kind`, `user`, `login`), as it would a forwarded call; `401` `identity_refused` for a refused token or login                                                 |
| `GET /api/storage`                                                     | Active retention policy, disk usage of the database files, calls with derived activity, and the per-body capture limit (`capture.body_limit_bytes`)                                                             |
| `GET /api/calls?limit=100`                                             | Filtered, paginated calls, newest first (see [search](#call-search))                                                                                                                                            |
| `GET /api/calls/{call-id}`                                             | One call with its attempts, inspection, and observation log                                                                                                                                                     |
| `GET /api/calls/{call-id}/body/{request\|response}`                    | Bounded body preview                                                                                                                                                                                            |
| `GET /api/calls/{call-id}/body/{request\|response}/export`             | The complete captured body as raw bytes, as `ren inspect --body` exports it                                                                                                                                     |
| `GET /api/sessions?limit=100`                                          | Recent sessions with call, turn, and error counts and token totals; `shelf=archived` lists the [archived](#archiving-and-deleting) ones instead                                                                 |
| `GET /api/usage?from_ms=&until_ms=`                                    | Calls, errors, and tokens in total, by UTC hour, by model, and by user (see [usage](#usage))                                                                                                                    |
| `GET /api/sessions/{session-id}/calls?limit=100`                       | One session's calls, oldest first                                                                                                                                                                               |
| `GET /api/sessions/{session-id}/turns?limit=100`                       | A session's turns, newest first, with prompt excerpts (see [activity](#activity))                                                                                                                               |
| `GET /api/search?q=&field=&session=&client=&from_ms=&until_ms=&limit=` | [Full-text search](#search), newest call first                                                                                                                                                                  |
| `GET /api/exposures?session=&from_ms=&until_ms=&limit=`                | [Exposed secrets](#exposed-secrets), newest call first                                                                                                                                                          |
| `GET /api/sessions/{session-id}/report`                                | A session's [context and cost report](#session-report)                                                                                                                                                          |
| `GET /api/turns/{turn-id}`                                             | One turn's [activity](#activity)                                                                                                                                                                                |
| `GET /api/calls/{call-id}/activity`                                    | One call's [activity](#activity)                                                                                                                                                                                |
| `GET /api/runs?limit=100`                                              | Recorded [runs](#runs), newest first: harness, folder, the client's `thread` and the `session` it names, `started_at`, and once ended `ended_at`, `end`, and `exit`; `shelf=archived` lists archived ended runs |
| `GET /api/runs/{run-id}/conversation`                                  | A run's [conversation](#run-conversation), whole: its own entries and those of the runs before it on its thread, oldest first                                                                                   |
| `GET /api/runs/{run-id}`                                               | One recorded run                                                                                                                                                                                                |
| `GET /api/live?after={seq}`                                            | Server-sent events as observations commit                                                                                                                                                                       |

Omit `after` for the first page; pass the returned `next` cursor unchanged for the
next one. The CLI accepts the same cursor with `timeline --after`. Pages order by
start time and call ID; refresh from the beginning to see late arrivals that sort
before a cursor. Limits are 1–1000. Recording is asynchronous, so new data appears
shortly after forwarding, typically within the 50 ms commit interval. Busy readers
return `503` without blocking forwarding. Management responses carry
`Cache-Control: no-store`. The UI does not relay full body exports; use `ren inspect` or the export endpoint. The API shares the
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
when the build had neither git nor packaging metadata. `ren --version` prints the
same, and `ren doctor` compares the two: it is OK only for the same version from
the same unmodified commit, and a warning otherwise. Untracked files are not
counted as modifications. The revision is read when the `ren-rs` package is
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
| `POST /control/v1/pause`                      | `{"retry_after_seconds": 1–3600}`, required    | `ren control pause --retry-after` (default 30)                                                                           |
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

### Run conversation

The daemon stores each driven run's conversation as it happens, whole: every
event the client reports (a turn starting and ending, each item: the
operator's prompt, a reply, reasoning, a command with its output, a file
change, a tool call; approvals, errors, and refusals), in the order it came.
An item reported again (a command that finished, a reply that completed)
replaces its entry in place. Streamed pieces are not stored; the item they
make is, and a reply still streaming when its turn ends is stored as far as
it came. Entries are written through the recorder like bodies, never
blocking forwarding: without queue room an entry is counted as omitted.

`GET /api/runs/{run-id}/conversation` answers `{api_version, entries,
earlier}`. Each entry is `{run, seq, at, turn, event}`, where `event` is the
event as the run feed sends it. The conversation includes the runs before
this one on the same client thread (a resumed conversation reads as one), up
to the newest 20,000 entries; `earlier` says older ones were left out.
Deleting a run deletes its conversation.

### Archiving and deleting

`POST /control/v1/history/sessions/{session}/{change}` and
`POST /control/v1/history/runs/{run}/{change}` archive, unarchive, or delete
what was recorded. `{session}` is Ren's session ID (as `/api/sessions` lists
it, not the client's), and `{change}` is `archive`, `unarchive`, or `delete`.
A run's change applies to the sessions its client recorded under the run's
thread too, and a session's to the runs that worked in it. The CLI is
`ren control archive|unarchive session|run <id>` and
`ren control delete session|run <id> --yes`.

- **Archive** hides it from `/api/sessions` and `/api/runs`, which list it
  with `shelf=archived` instead, with its `archived_at`. A session with traffic
  newer than its archiving is listed again by itself; a run still running is
  always listed.
- **Delete** removes its finished calls with their bodies, attempts, log rows,
  digests, and search entries, and its ended runs. Calls still open and runs
  still running are kept, so a delete is refused while there are any: `409`
  `call_open` or `409` `run_running`. The next retention pass frees the chunks
  only those calls referenced. It cannot be undone; a client that keeps
  calling starts a new Ren session.

Each edit is recorded (a `history_edited` observation with the actor) and
applied by the recorder after what it was recording; the answer is `202`
`{api_version, subject, change}` once it is queued. `404` `not_found` is an
unknown subject or change.

### Runs

`/control/v1/runs` starts and drives agent runs: the official client, started
by the daemon in a folder under a `[runs]` root and driven through its
structured mode: Codex's `codex app-server`, Claude Code's print mode with
streaming JSON (`claude -p --input-format stream-json --output-format
stream-json`), or Pi's RPC mode (`pi --mode rpc`, with Ren's extension). The
client keeps its own sign-in, and its model traffic goes through this gateway
like any session. Ren names a new Claude Code conversation or Pi session
itself (`--session-id`), so a run's `thread` is the session the recorded calls
carry; Claude Code's permission prompts are its approvals, and Ren answers no
other request (hooks, MCP, elicitation). Claude Code reports no live turn id
matching its recorded prompt id, so a driven turn's `turn` is its prompt's
message id, not the recorded turn's `native_id`.

Pi asks for no approvals: it runs its tools as it decides, so a Pi run never
waits on the operator mid-turn. Its turn lasts from the prompt until Pi
settles (`agent_settled`), after any retries or compaction; Pi names no turns,
so the daemon mints each driven turn's id, which is not the recorded turn's
`native_id`. A Pi run resumes the session of that id in the run's own folder.
Dialogs an extension opens are cancelled. The daemon publishes Ren's Pi
extension to `$XDG_CACHE_HOME/ren/pi` (else `~/.cache/ren/pi`) before starting
Pi, so the daemon's environment needs one of them.

Driving Claude Code with a Claude subscription is the operator signing in to
the unmodified `claude` program on their own machine, which Anthropic's
[terms for Claude Code](https://code.claude.com/docs/en/legal-and-compliance)
allow; offering that sign-in or its usage to other people is not. Allow only
your own Tailscale login to drive runs (`[control]`, and `REN_CONTROL_LOGINS`
for the web UI), and keep runs to ordinary, individual use: plan limits assume
it.
Runs are off while `runs.roots` is empty: every route then answers `404`
`runs_disabled`. Access is the control API's.

| Endpoint                                           | Body                                                                                                                                                          | Answer                                                                                                                                                                                                                                                                                                                                                                      |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /control/v1/runs`                             | none                                                                                                                                                          | `{api_version, runs}`: each `{run, harness, workdir, state}`                                                                                                                                                                                                                                                                                                                |
| `POST /control/v1/runs`                            | `{"harness": "codex"\|"claude_code", "dir": "/abs/path"}`, optional `"resume": "<thread>"` and `"prompt"`, the first prompt, sent once the conversation opens | `201` `{api_version, run}`; with `resume`, the run continues that client conversation                                                                                                                                                                                                                                                                                       |
| `POST /control/v1/runs/{run}/prompt`               | `{"text": "…"}`, not blank                                                                                                                                    | `202`                                                                                                                                                                                                                                                                                                                                                                       |
| `POST /control/v1/runs/{run}/approvals/{approval}` | `{"decision": …}`                                                                                                                                             | `202`; `accept`, `accept_for_session`, `decline`, or `cancel`                                                                                                                                                                                                                                                                                                               |
| `POST /control/v1/runs/{run}/interrupt`            | none                                                                                                                                                          | `202`                                                                                                                                                                                                                                                                                                                                                                       |
| `POST /control/v1/runs/{run}/stop`                 | none                                                                                                                                                          | `202`; the run ends once its process has exited                                                                                                                                                                                                                                                                                                                             |
| `GET /control/v1/runs/projects`                    | none                                                                                                                                                          | `{api_version, projects}`: the git repositories under the run roots (not looked inside, three levels down at most, hidden, `node_modules`, `target`, `vendor`, `dist`, and `build` folders skipped), and each root's folders with none below them; each `{path, name, git}`, `git` one of `{"kind": "branch", name}`, `detached` with its `commit`, `unreadable`, or `none` |
| `GET /control/v1/runs/clients`                     | none                                                                                                                                                          | `{api_version, clients}`: every client, each `{harness, status}`, `status` one of `ready` (Ren drives it and its program is on the daemon's path), `not_installed`, or `not_drivable` (Ren cannot drive it yet)                                                                                                                                                             |
| `GET /control/v1/runs/events`                      | none                                                                                                                                                          | server-sent `run` events from now on; `lagged` when some were skipped                                                                                                                                                                                                                                                                                                       |

Commands answer `202` `{api_version}` once sent to the agent; what it does
next arrives as run events. A run's `state` has a `phase`: `starting`, `idle` (with `thread`, the client's
conversation ID), `working` (with `turn` once known and `approvals`, each
`{approval, ask}` waiting for an answer, oldest first: `ask` is a `command`
with its `command`, `cwd`, and `reason`, a `file_change` with its
`reason`, or a `tool` (another of the client's tools) with its `tool` and
`detail`), `stopping` (with its `cause`), or `ended`
(with `end`, one of `spawn_failed`, `open_refused`, `exited`, `stopped`,
`drained`, `recovered`, and `exit`, `{"kind": "code"|"signal", "value": n}` or
`{"kind": "unknown"}`). Run events are `{"kind": "state", run, state}` or
`{"kind": "event", run, event}`, where `event` is what the agent reported: its
`delta` text, `item`s (each with its `kind`, its `read`, what Ren read of it
the same way for every client: `{"as": "prompt"|"reply"|"plan", text}`,
`{"as": "command", command, status, output, exit_code}`,
`{"as": "file_change", paths, status}`, `{"as": "tool", name, status}`,
`{"as": "reasoning"}`, or `{"as": "unread"}`; and its `body` as the client
described it), `approval` requests, `turn_started` and `turn_ended`, and
`error`s. Events are not stored; the run's start, opening, and end are, and its
model traffic is recorded as any session's.

The web UI's Runs view drives runs through this API when its server is given
the control token file (see the [UI's README](../ui/README.md#runs)), and the
terminal app (`ren tui`, `ren runs`) does from a terminal.

Refusals: `400` `invalid_directory` (not absolute, contains `..`, or outside
every root once links are resolved), `400` `directory_not_found`, `400`
`not_drivable` (a client Ren cannot drive yet), `502` `spawn_failed` (the
client is not installed; the run is recorded as `spawn_failed`), `503`
`unrecorded`, `404` `not_found` for an unknown run or approval, and `409` when
the run's state does not allow the command: `starting`, `busy`, `not_working`,
`turn_not_started`, `no_such_approval`, `stopping`, or `ended`.

## Live feed

`GET /api/live` streams `observation` events, one per committed observation
(`call_opened`, `attempt_started`, `response_head`, `attempt_finished`,
`call_finished`, `call_recovered`, `channel_closed`, `operator_action`,
`run_started`, `run_opened`, `run_ended`, `run_recovered`, `history_edited`). Each event's `id` is its sequence number. Without
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
for them, to show replies on Live cards as they stream, and so does `ren tui`;
`ren tail` does not.

## Call search

`GET /api/calls` filters all retained calls and returns `{calls, next}` newest
first. Parameters: `q`, `session` (a Ren session ID), `client` (`claude_code`,
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
first 256 KiB, not a full export; use `ren inspect --body … --output …` for
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
