# History and API

Read history from the CLI or the daemon's read-only management API.

## CLI

```sh
tgv history --limit 10
tgv sessions
tgv timeline <session-id> --limit 100
tgv inspect <call-id>
tgv inspect <call-id> --body response --output response.bin
```

Commands emit JSON: `history` a list of calls, `sessions` a list of sessions,
`timeline` a page `{calls, next}`, and `inspect` one call with its attempts and
observation log. Body exports write the exact captured bytes to a new file, refuse
incomplete bodies, and never overwrite existing files. They work while the daemon
runs and after it stops. Use the same data directory (`--data-dir`) for all
commands. On a remote deployment, run history commands on that host.

## Endpoints

Every JSON body carries `"api_version": 1`; errors are `{api_version, error}` with
`error` one of `not_found`, `invalid_query`, `busy`, or `unavailable`.
A body export answers with the raw bytes (`application/octet-stream`) only when
the body was stored whole; otherwise `not_found`. At most two run at once
(`busy`), and the UI does not forward it.

| Endpoint                                                   | Result                                                                      |
| ---------------------------------------------------------- | --------------------------------------------------------------------------- |
| `GET /health`                                              | `ok` or `degraded` (HTTP 503), the running build, recorder counters         |
| `GET /api/metrics`                                         | Recorder counters and storage usage                                         |
| `GET /api/storage`                                         | Active retention policy and disk usage of both database files               |
| `GET /api/calls?limit=100`                                 | Filtered, paginated calls, newest first (see [search](#call-search))        |
| `GET /api/calls/{call-id}`                                 | One call with its attempts, inspection, and observation log                 |
| `GET /api/calls/{call-id}/body/{request\|response}`        | Bounded body preview                                                        |
| `GET /api/calls/{call-id}/body/{request\|response}/export` | The complete captured body as raw bytes, as `tgv inspect --body` exports it |
| `GET /api/sessions?limit=100`                              | Recent sessions with call and turn counts                                   |
| `GET /api/sessions/{session-id}/calls?limit=100`           | One session's calls, oldest first                                           |
| `GET /api/live?after={seq}`                                | Server-sent events as observations commit                                   |

Omit `after` for the first page; pass the returned `next` cursor unchanged for the
next one. The CLI accepts the same cursor with `timeline --after`. Pages order by
start time and call ID; refresh from the beginning to see late arrivals that sort
before a cursor. Limits are 1–1000. Recording is asynchronous, so new data appears
shortly after forwarding, typically within the 50 ms commit interval. Busy readers
return `503` without blocking forwarding. Management responses carry
`Cache-Control: no-store`. Full body export is CLI-only. The API shares the
listener's loopback/Tailscale access boundary and has no separate login or
cross-origin browser access enabled.

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

## Live feed

`GET /api/live` streams `observation` events, one per committed lifecycle record
(`call_opened`, `attempt_started`, `response_head`, `attempt_finished`,
`call_finished`, `call_recovered`). Each event's `id` is its sequence number. Without
`after`, only new observations are sent; with `after` (or a `Last-Event-ID` header,
which browsers send on reconnect) the feed first replays retained observations after
that position. A `gone` event means the position was removed by retention: reload
and reconnect without `after`. A `lagged` event means the subscriber fell behind
and the feed is catching up from storage. The stream ends when the daemon shuts down.
Observations describe what was recorded, not model or tool state.

## Call search

`GET /api/calls` filters all retained calls and returns `{calls, next}` newest
first. Parameters: `q`, `provider` (`anthropic`, `openai`, `codex`), `model`,
`tool`, `has_tools`, `status` (`active`, `finished`, `errors`, `interrupted`),
`from_ms`, `until_ms` (inclusive), `limit` (1–1000), and `after`. Filters combine
with AND; unknown parameters are rejected.

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
gzip and Zstandard where possible and report the extent (`complete`, `partial`,
`truncated`, or `gap`), the observed size (`null` while the body is still open),
and decoding integrity explicitly. Preview work is bounded and independent of
forwarding. Bodies may contain private prompts or tool results and share the
management API's private access boundary. The UI renders them as inert text and
never stores them; see [UI payload previews](../ui/README.md#payload-previews) for
limits.
