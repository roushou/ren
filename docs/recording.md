# Recording

What TGV captures, how it stores it, what it infers, and how history is kept,
backed up, and pruned.

## Capture

- Bodies pass through without reserialization unless a configured transform changes
  them (see [Policy](policy.md)). HTTP chunk boundaries may change; the body bytes
  and ordering are preserved.
- HTTP request bodies are buffered whole before they are forwarded, so policy can
  read them and a failover can replay them. This adds the upload time to
  time-to-first-byte, which matters over a slow uplink to a VPS. Streaming
  responses are not buffered.
- One recorder task owns the store. Forwarding hands it lifecycle records and body
  pieces through a bounded queue and never waits for storage. Body data has a slot
  budget (`capture.queue_capacity`) and a byte budget (`capture.queue_bytes`); the
  captured prefix is 16 MiB per direction (`capture.body_limit_bytes`). A limit or
  a saturated queue affects recording, not forwarding.
- Each call reserves its lifecycle records (open, finish, and per planned attempt a
  start, response head, and finish) before forwarding starts, so terminals are never
  dropped for queue pressure. If even that capacity is unavailable, the call is
  unrecorded and a separate counter increments. Omitted body bytes are counted by
  reason: `policy_limit`, `queue_bytes`, `queue_slots`, `storage_failure`, and
  `unattributed` (WebSocket messages while no call is open).
  Offsets and body endings (`ended`, `unfinished`, `open`) expose gaps.
- `GET /health` reports the queue, omissions, storage and retention errors, commit
  latency and lag, deduplication, pruning, and recovered-call counters. Storage or
  retention errors produce HTTP `503`; forwarding continues. If a commit fails,
  the recorder retries without body data so lifecycle records survive. A healthy
  response alone does not establish complete capture.
- Credential headers, arbitrary transport headers, and request URLs are not stored.
  Allowlisted correlation UUIDs are recorded (see [Clients](clients.md#sessions-and-turns)).
  Captured bodies are private content and may themselves contain secrets from
  prompts or tool results. Set `capture.body_limit_bytes = 0` for metadata-only
  recording.
- Storage is two SQLite WAL databases in the data directory. `tgv.sqlite3` holds
  the observation log and the query tables, written in one transaction.
  `blobs.sqlite3` holds body content split into content-defined chunks (8–64 KiB),
  identified by BLAKE3 hash, stored once, and compressed with Zstandard when that
  makes them smaller. Repeated conversation prefixes across turns are therefore
  stored once. Storage is host-local; a process lock prevents two daemon writers.
- `search.sqlite3` is a full-text index of what calls show (prompts, replies,
  tool input and output; see [search](api.md#search)). It is derived from the
  bodies above, never the source of truth: delete it and the daemon rebuilds it.
  It holds excerpts of the same private content, owner-only, with credentials in
  well-known formats replaced by `«redacted: kind»`; their kinds, masked hints,
  and fingerprints are listed as [exposures](api.md#exposed-secrets).
- Records commit in groups every 50 ms (`capture.commit_interval_ms`) or sooner
  under load. Accepted records drain on normal shutdown. A crash can lose queued
  records; calls left open are marked `recovered` on the next startup, an unknown
  outcome, never success.

## Calls, attempts, and terminals

A **call** is one client request; each try against the upstream is an
**attempt**. A call has one attempt unless a fallback rule replays it (see
[Policy](policy.md)). Each attempt records how it ended, kept apart from the HTTP
status and the model outcome:

- `body_complete`: the response body reached its end by EOF or framing. Not
  successful inference, receipt by the client, or completion of agent tools. With
  cause `declared_length`, the response's `content-length` was relayed whole and
  the server stopped reading before the stream's own end arrived: an HTTP/2
  provider can send its end-of-stream after the last data, and the server stops
  once it has written the declared length to an HTTP/1.1 client.
- `not_sent`: the connection could not be established, so nothing was sent.
- `refused`: the request was sent, and the provider reset its HTTP/2 stream with
  `REFUSED_STREAM`, which means it closed the stream before any processing
  (RFC 9113 section 8.7). Not accepted.
- `acceptance_unknown`: the request was, or may have been, sent and no response
  head arrived (a header timeout, or a failure on a connection in use). The
  provider may have accepted it.
- `stream_failed`: the response stream stopped after its head.
- `cancelled`: by the client leaving mid-upload, by an operator, or by shutdown.
- `interrupted`: the response was let go without an observed end, usually because
  the client closed it.
- `terminal_event` (WebSocket): the call's terminal event arrived
  (`response.completed`, `.failed`, `.incomplete`, or `error`). The event, not this
  label, says how the model call ended.

The attempt's `terminal_cause` (`connect_failed`, `stream_refused`,
`header_timeout`, `send_failed`, `idle_timeout`, `body_error`, `upstream_closed`,
`client`, `shutdown`, `operator`, `declared_length`) and
`terminal_phase` (`upload`, `awaiting_head`, `response`) say why and how far it
got.

A WebSocket socket is one channel. Its handshake is recorded as a call of its own
(method `GET`, status `101`, or the upstream's refusal). Each `response.create`
then opens a call (method `MESSAGE`) that ends with its terminal event, or when
either socket closes or fails (`cancelled` by the client, `stream_failed` with
`upstream_closed` or `body_error`), or at shutdown. A call's request body is its
client messages back to back (the create, and any `response.interrupt` for it); its
response body is the upstream messages. The call detail lists where each message
starts. An interrupt the upstream accepted makes the terminal's cause `client`.
Messages that arrive while no call is open are counted on the channel and in the
`unattributed` omission, not stored. The channel records how it ended
(`client_closed`, `upstream_closed`, `client_failed`, `upstream_failed`, `drained`,
`aborted`, `dropped`, or `recovered` after a restart) and the close code.

## Gateway responses and limits

When the gateway answers instead of the provider, the status says why: `503` when
the concurrency limit is reached or the daemon is shutting down, `502` when the
connection or send failed, and `504` when no response head arrived in time.
Provider responses, including errors and redirects, pass through unchanged and are
never retried, except where an opt-in fallback rule names the status. Default
limits are 32 concurrent requests, 10 seconds to connect, 120 seconds to response
headers (including upload), and 120 seconds between upstream reads. There is no
total response duration limit.

The daemon handles Ctrl-C and SIGTERM by draining: it refuses new calls with `503`,
lets running calls continue for up to 10 seconds, then cancels what remains
(recorded as `cancelled` with cause `shutdown`), and finally drains recording.

## Response inspection

`inspect` includes each attempt's `inspection`, derived by the recorder from the
captured Anthropic Messages and OpenAI Responses bodies; `history` rows carry its
summary (`model`, `outcome`, `stop_reason`, `usage`, `tool_names`). It reports model,
model completion/failure, stop reason, provider-reported token usage, and requested
tool IDs/names. Tool arguments, results, and generated text remain in raw capture;
the summary does not establish whether a tool ran. Missing token counts remain
`null`. OpenAI input totals include cached tokens; Anthropic's input field excludes
them, as indicated by `usage.accounting` (`includes_cache` or `excludes_cache`).
These are usage observations, not subscription allowance or monetary charges.

Model completion and HTTP completion are independent. Codex can consume
`response.completed` and close before HTTP EOF: the call's `terminal` remains
`interrupted`, while `outcome` records `completed`. Inspection coverage and
limitations expose missing bytes, unfinished streams, malformed or oversized
events, and absent terminal events. Parsing stops at the first capture gap;
unknown events continue to pass through unchanged.

Inspection reads only captured bytes. It is bounded to 1 MiB per SSE event, 16 MiB
per JSON body, 64 MiB of decoded output, and 128 distinct tool calls. Gzip,
Zstandard, and Brotli are decoded only for inspection; capture and forwarding keep
the compressed bytes. Other encodings, and several stacked encodings, are reported
as unavailable. Responses recorded before Brotli support keep the label
`unsupported`, so their previews stay raw bytes. Codex can omit
`Content-Type` on `/responses`; TGV then assumes SSE and records the format as
inferred. Metadata-only capture cannot provide usage or model completion.

Client tool-execution telemetry, quota parsing, configuration reload, protocol
translation, and re-inspection of stored bodies are not implemented.

Protocol references: [Anthropic streaming](https://platform.claude.com/docs/en/build-with-claude/streaming)
and [OpenAI Responses events](https://developers.openai.com/api/reference/resources/responses/streaming-events).

## Backup

With the daemon running, `tgv control backup` writes a consistent copy of the
history to `<data-dir>/backup/` (`tgv.sqlite3` and `blobs.sqlite3`, owner-only).
The location is fixed; neither the CLI nor the control API can choose it. The
metadata is snapshotted in one read transaction, then only chunks the backup
lacks are copied, and chunks no longer referenced are removed, so later backups
are fast and the copy tracks retention. The chunk sweep pauses while a backup
runs; recording and forwarding continue. Only one backup runs at a time (`409`).
Never copy the live database files of a running daemon.

Keep the command running until it prints its report. If it stops part way, the
previous backup is still complete. A backup whose history lost a referenced
chunk fails (`503 backup_incomplete`) and leaves the previous one in place.

The backup is a data directory: copy `<data-dir>/backup` elsewhere (another disk
or host) with any file tool, since it is not written while no backup runs. To
restore, stop TGV and replace `tgv.sqlite3` and `blobs.sqlite3` in the data
directory with the backup's two files, removing any `-wal` and `-shm` files
beside them. It needs as much free space as the history uses. The search index is
not backed up: delete `search.sqlite3` too, and the daemon rebuilds it from the
restored history.

## Retention

Retention is enabled by default: **7 days, 10,000 calls, and 2 GiB of database
pages in use**. Configure `[retention]` in `tgv.example.toml`; set `enabled = false`
to keep everything. All numeric limits must remain positive. Age is measured from
call start, and either count or storage pressure can remove younger calls.

The recorder removes the oldest finished calls in transactions of at most 16, then
deletes chunks no remaining body references. Maintenance runs at startup and every
60 seconds; full batches schedule another pass sooner. A call, its attempts, bodies,
and observations are deleted together; session counts and orphan turns/sessions are
updated in the same transaction. Open calls are protected; recovered calls are
eligible. Recording never waits for pruning.

The storage limit counts live pages of `tgv.sqlite3` and `blobs.sqlite3`, plus the
search index's live pages in proportion to the indexed calls the store still has.
Index rows of just-deleted calls stay until the indexer's next pass (within
seconds) and do not count, so retention never deletes extra calls while the index
catches up. `tgv doctor` and the Recorder page show the index's size and coverage.

Limits are eventual, not hard disk quotas: active work and maintenance intervals
can exceed them. Because bodies are deduplicated, removing a call frees only the
chunks nothing else references. Freed pages are reused rather than returned to
the filesystem, so files keep their allocated size; `/api/storage` reports both
allocated and in-use bytes. There is no automatic `VACUUM`, secure-erasure
guarantee, or backup cleanup. Monitor actual filesystem space too.

Retention changes history: old links can return `404`, expired cursors return
`400`, and the live feed reports `gone`. Refresh from the start. Session counts
describe retained calls, and a resumed client whose entire session expired receives
a new TGV session ID. Raw captures cannot be recovered through TGV after pruning.
