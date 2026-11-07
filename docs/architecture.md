# TGV architecture

How TGV is built: its requirements, the units of work it records, the crates
that implement it, and the behaviour of each plane. [`AGENTS.md`](../AGENTS.md)
holds the contributor rules; [`README.md`](../README.md) is for operators.

---

## 1. Overview

TGV is three planes around a typed, append-only observation log:

```text
            ┌──────────────────────── data plane ────────────────────────┐
clients ──► │ ingress (HTTP · WebSocket)                                 │ ──► providers
            │   → call pipeline: admit → buffer+parse → plan → dispatch  │
            │   → relay (streamed) ── tap (non-blocking, bounded) ──┐    │
            └───────────────────────────────────────────────────────┼────┘
                                                                    ▼
            ┌──────────────────── observation plane ─────────────────────┐
            │ recorder: per-call ordered capture · incremental inspection│
            │   · chunk+compress → group-commit writer → log + tables    │
            │   → live broadcast                                         │
            └────────────────────────────────────────────────────────────┘
                                                                    ▲
            ┌──────────────────── management plane ──────────────────────┐
UI · CLI ─► │ query API (read pool) · live API (SSE) · control API       │
            │ config snapshot + operator state (both revisioned)         │
            └────────────────────────────────────────────────────────────┘
```

The central ideas:

1. **Channel → Call → Attempt.** One unit model covers HTTP requests, WebSocket
   connections that carry many calls, and failover that makes several attempts
   per call.
2. **Retries stay narrow.** Only attempts that were provably not accepted fail
   over. Clients already retry heavily, and they own every retry after a request
   may have been accepted.
3. **The source of truth is an observation log**, with query tables written in the
   same transaction. The live UI reads the log over SSE.
4. **Bodies are content-addressed.** They are split by content-defined chunking,
   compressed, deduplicated, and garbage-collected by mark-and-sweep. Agent
   requests resend the whole conversation every turn, so raw storage would grow
   quadratically; deduplication makes it roughly linear.
5. **Inspection is incremental** while bytes flow, never a re-read after the
   attempt finishes.
6. **Request bodies are buffered** under a global memory bound and upload
   timeouts, so policy can read them and failover can replay them.

---

## 2. Requirements

TGV targets heavy use by one operator: several concurrent agents all day, MB-sized tool
outputs, weeks of retention, and tens of GB of history.

| #  | Requirement                                                                                                                                                     |
| -- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| R1 | Forward native traffic faithfully over HTTP/SSE (byte-faithful) and WebSocket (message-faithful; ping/pong, fragmentation, and compression are per connection). |
| R2 | Observation never stalls forwarding. Capture is bounded, and every loss is accounted for by reason.                                                             |
| R3 | Record everything with provenance and ordering, at tens of GB, with weeks of retention.                                                                         |
| R4 | Inspect native protocols incrementally and expose live observations.                                                                                            |
| R5 | Route each call to an upstream. Fail over only when it is provably safe. Record every decision.                                                                 |
| R6 | Operator controls on live traffic: cancel, pause, and block.                                                                                                    |
| R7 | Deliberate transformations that record the original, the result, and the reason.                                                                                |
| R8 | Same binary and storage format locally and on a Tailscale VPS. The UI is separate, optional, and deployed independently.                                        |
| R9 | Deterministic tests of every component without the whole app.                                                                                                   |

### Non-goals

- Owning or pooling subscription credentials, or routing between subscription
  accounts.
- Transforming response streams.
- Protocol translation between Messages and Responses.
- Multiple daemons, or storage shared between them.
- Lanes in the public OpenAI WebSocket mode (no current client uses them).
- Token budgets on subscription traffic: token usage doesn't measure
  subscription allowance.

### External constraints

Re-verify these against current provider documentation before changing related
behaviour.

Sources:

- [Claude Code gateway protocol](https://code.claude.com/docs/en/llm-gateway-protocol)
- [Claude Code errors and retries](https://code.claude.com/docs/en/errors)
- [Anthropic API errors](https://platform.claude.com/docs/en/api/errors)
- [Claude Code legal and compliance](https://code.claude.com/docs/en/legal-and-compliance)
- [OpenAI WebSocket mode](https://developers.openai.com/api/docs/guides/websocket-mode)
- [Sign in with ChatGPT inference](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference)
- `openai/codex` at `48524f33`: `codex-api/src/endpoint/responses_websocket.rs` and `core/src/client.rs`
- [SQLite internal vs external BLOBs](https://www.sqlite.org/intern-v-extern-blob.html)
- [SQLite WAL](https://www.sqlite.org/wal.html)

**Anthropic (Claude Code gateway protocol)**

- Forward `anthropic-version` and `anthropic-beta` verbatim. With a subscription,
  `anthropic-beta` carries the OAuth capability, and stripping it causes 401 errors.
- Inspect without modifying:
  - beta headers and body fields travel in pairs;
  - the `system` array is position-sensitive and must be passed through as received;
  - error bodies must pass through unmodified, because Claude Code matches on their
    wording;
  - SSE `ping` events are never buffered or stripped.
- Pass through `retry-after`, `x-should-retry`, and `anthropic-ratelimit-unified-*`.
- Claude Code retries before any output (documented: up to 10 times). It never
  re-runs a request after text or a tool call has completed.
- There is no idempotency key and no lookup of a message by ID. An `event: error`
  can arrive mid-stream after a 200.
- Claude Code subagents run concurrent calls under one session.

**Codex over WebSocket**

- The URL is the Responses URL with the scheme changed to `wss://`. The handshake
  sends the normal auth headers plus `OpenAI-Beta: responses_websockets=2026-02-06`.
- Each call is one Text frame, `{"type":"response.create",…}`. The server sends one
  JSON frame per Responses event, plus Codex-specific events.
- One response runs at a time per connection.
- Continuation uses `previous_response_id`, and that state is scoped to the
  connection.
- `x-codex-turn-state` is a sticky routing token from the handshake.
- Cancelling sends `response.interrupt`, answered by `response.interrupt.accepted`
  or `response.interrupt.failed` (Codex source only).
- A `426` on the handshake makes Codex fall back to HTTP/SSE (Codex source).
- Codex may compress request bodies with zstd.

**Public OpenAI WebSocket mode**

- Optional `stream_id` lanes: FIFO within a lane, concurrent across lanes.
- Documented limits: 16 responses in flight, 32 lanes, 60-minute connections.

**Terms**

- Anthropic forbids third parties to "route requests through … plan credentials on
  behalf of their users" or to "collect, store, or intermediate Claude.ai
  credentials".
- OpenAI says not to share `auth.json`.
- So TGV never stores subscription credentials and never pools accounts. API keys
  owned by the operator are a separate, allowed case.

**SQLite**

- BLOBs smaller than about 100 KB read faster from inside the database.
- In WAL mode, a transaction across attached databases is atomic per file, not
  across files.
- Long-running readers block checkpoints.

---

## 3. Units of work

| Unit      | Meaning                           | HTTP                                              | WebSocket                                              |
| --------- | --------------------------------- | ------------------------------------------------- | ------------------------------------------------------ |
| `Channel` | One client connection or exchange | One request/response                              | The upgraded socket, pinned 1:1 to one upstream socket |
| `Call`    | One logical model invocation      | The request                                       | One `response.create` through to its terminal event    |
| `Attempt` | One upstream try of a call        | 0..n (0 when blocked or rejected before dispatch) | 0 or 1, because state is scoped to the connection      |

- **A call has its own terminal**, separate from attempt terminals. It covers calls
  with no attempt: blocked, paused, too large, or abandoned during upload.
- **Transformed bodies belong to the attempt.** The call holds the client's
  original request body. Each attempt references the body it actually sent, plus
  the rule and reason when it differs.
- **Sessions and turns** are attribution groupings above calls, built from client
  hints and never inferred. Sessions are keyed by provider, hint source, and native
  ID. Several calls can be active in one turn, because Claude Code subagents share a
  session and prompt ID.
- **On WebSocket**, the handshake is a call of its own (method `GET`, status `101`
  or the upstream's refusal). Each `response.create` is a call with method
  `MESSAGE` on the handshake's channel. Session hints come from the handshake
  headers; turn hints come from each message's `client_metadata` when present.

Identities: `ChannelId`, `CallId`, `AttemptId`, `SessionId`, `TurnId`, and
`UpstreamId`, plus a separate type for each provider-native identifier.

---

## 4. Crates

The workspace has one crate per component. Each exposes one primary type at its
root, and dependencies run one way; Cargo rejects cycles.

```text
tgv-cli      ─► tgv-daemon, tgv-gateway
tgv-daemon   ─► tgv-gateway, tgv-store
tgv-gateway  ─► tgv-harness, tgv-recorder, tgv-store
tgv-store    ─► tgv-recorder
tgv-recorder ─► tgv-harness, tgv-protocol
tgv-harness  ─► tgv-protocol
xtask        ─► tgv-daemon, tgv-harness   (repository tasks, unpublished)
```

| Crate                                      | Primary type                                                                  | Owns                                                                                                                                                                                                                                                                                                                                                                                                 |
| ------------------------------------------ | ----------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tgv-protocol`                             | `CallInspector`                                                               | `Provider`, `Endpoint`, `Method`, `Transport`, `Direction`; `Stored<T>` and the label trait; SSE framing; encodings; Anthropic Messages and OpenAI Responses wire types; WebSocket message classification (`Payload`, `CloseCode`, `ClientMessage`, `UpstreamMessage`, `InterruptOutcome`); the native identifiers and `MessageHints` the wire carries; inspection; the activity reader (`activity`) |
| `tgv-harness`                              | `Harness` enum: `ClaudeCode`, `Codex`, `Pi`                                   | attribution (`Attribution`, `SessionHint`, hint sources and issues); launch plans; the Pi extension cache; `GatewayEndpoint`, the URL a harness is pointed at; `HostEnv`; what each harness's tools and messages mean (`tools`)                                                                                                                                                                      |
| `tgv-recorder`                             | `Recorder`; `RecorderTask` runs it; `RecordSink` is its port                  | `ChannelId`, `CallId`, `AttemptId`, `SessionId`, `TurnId`; `Clock`, `IdSource`, and their test implementations; recorded facts (terminals, causes, phases, rejections, body endings, omissions, `ChannelEnd`, `ConfigRevision`, `UpstreamId`, `OperatorRevision`); the stored inspection format; the `Chunker`; batches; `RetentionPolicy`; taps, queue, metrics, live broadcast                     |
| `tgv-store`                                | `Store`, which implements `RecordSink`; `Reader` and `Backup` are its handles | SQLite and migrations, the stored chunk format, queries, views, turn and call activity assembled from stored digests (`activity`), full-text search and exposures over the digests (`search`), retention, `Recovery`, the sweep gate, backup                                                                                                                                                         |
| `tgv-gateway`                              | `Gateway`                                                                     | all HTTP routes: forwarding, the read API and `/api/live`, and authenticated controls and backup; route table, header policy, request buffering, upstream entries, the `Upstream` seam and Reqwest implementation, `HttpAttempt`, `WsChannel`, retry facts, policy, `Controls`, `OperatorState`, the running-call registry, ingress and the shutdown drain, the `Build` `/health` reports            |
| `tgv-daemon`                               | `Daemon`                                                                      | `Config` → `RuntimeConfig`; resource wiring, listener binding, control-token file, and lifecycle; the system clock and random IDs; the server task, and the observation thread (`ObservationThread`), whose own current-thread runtime runs the recorder                                                                                                                                             |
| `tgv-cli` (package `tgv-rs`, binary `tgv`) | `Cli`                                                                         | capturing `HostEnv` in `main`, recording the source revision (`build.rs`), commands, gateway choice and `client.toml`, history and controls on this host or through a gateway's APIs (`management`), `tgv tail` (`tail`), the pure MCP core of `tgv mcp` (`mcp`), doctor and its build comparison, service definitions                                                                               |
| `xtask` (unpublished, `cargo xtask`)       | `Xtask`                                                                       | `install-vps`: installing or upgrading TGV from a checkout as a systemd user service between calls, published with Tailscale Serve, and with `--ui-https-port` the UI as `tgv-ui.service`; its publishing flags are saved for later runs                                                                                                                                                             |

- **Purity.** `tgv-protocol`'s manifest names only pure dependencies (`bytes`,
  `serde`, `serde_json`, `http` types, and the decompressors), and a source test
  (`sources_stay_free_of_runtime_and_io`) rejects runtime and I/O paths. Inside
  `tgv-gateway`, `src/purity.rs` keeps `attempt`, `channel`, `retry`, and
  `operator` free of runtime, I/O, storage, and shell-module dependencies.
- **Ports point down.** The recorder declares `RecordSink`, `Clock`, and
  `IdSource`; the store implements `RecordSink`; the daemon implements the system
  clock and random IDs, which carry the only `disallowed_methods` exemptions
  besides `HostEnv::capture`.
- **Operator documents are opaque below the gateway.** The recorder and store carry
  an operator action as a revision, a command label, and serialized JSON documents.
  `tgv-gateway` owns `OperatorState` and parses the stored snapshot on restore.
- **One HTTP service.** The gateway assembles inference, management, and control
  routes into one router (`Gateway::router`) and serves one listener
  (`Gateway::into_ingress`). The daemon opens resources, binds the socket, writes
  the control-token file, and owns startup and shutdown; it defines no routes.
  Management reads through the store's `Reader` and `Backup` handles; SQL stays in
  the store, which has no HTTP dependency. Forwarding records only through the
  recorder.

---

## 5. Data plane

### Routes and headers

A declarative route table (`Route::ALL`) maps each client path to a provider
endpoint and names the harness whose correlation headers it accepts. Pi's
extension headers are recognised on every route. A header policy expressed as
data (`HeaderPolicy::REQUEST` and `RESPONSE`) removes hop-by-hop, proxy identity,
and Tailscale identity headers, and strips TGV's own `x-tgv-*` hints before
forwarding. Redirects, automatic retries, response decompression, and
environment proxies are disabled on the upstream client.

### Request buffering

Routing, transformations, and failover replay need the whole request, so HTTP
requests are buffered before dispatch (`buffer`).

- **Global memory bound.** A byte semaphore is acquired from `Content-Length`
  before reading, or incrementally while reading chunked bodies. When the budget
  (`buffer.budget_bytes`, default 1 GiB) is exhausted, the client gets an
  immediate `503` with `retry-after`.
- **Per-request cap.** A request over its provider's limit
  (`[buffer.request_limits]`) gets `413`, with a typed rejection recorded on the
  call. Defaults are the providers' documented limits: Anthropic 32 MiB, OpenAI
  512 MiB, and Codex 512 MiB (the Codex backend documents none, so OpenAI's is
  used). There is no pass-through mode, because it would bypass policy.
- **Timeouts.** Separate idle (30 s) and total (300 s) upload timeouts answer
  `408`.
- **Abandoned uploads.** A guard records the call with the rejection `abandoned`
  when the client disconnects before the read ends.
- **No extra copies.** The buffered `Bytes` go to the recorder without copying.
- **Latency.** Buffering adds the upload time to time-to-first-byte, which matters
  over a home uplink to a VPS. Response streams are never buffered.

### Policy and the call plan

`policy` is pure: `Policy::plan(&CallFacts, &OperatorState)` produces admission,
ordered candidates, transform rules, and the retry policy. Rules are validated
when the configuration loads.

- **Two revisions per call.** Configuration and operator state change
  independently, so each call records `config_revision` and `operator_revision`.
  Every request uses one immutable configuration snapshot.
- **The plan is recorded with the call** before dispatch, with each attempt's
  upstream and changes, or the reason the call was refused.
- **Rules.** Block rules on call facts, a global pause and session blocks from
  operator state, transform rules, and fallback rules that name a status and an
  operator-key upstream.
- **On WebSocket**, routing happens once, at upgrade, and the channel is pinned to
  the provider's own entry. Every `response.create` still goes through admission,
  block, and pause checks against the current snapshot; a refusal is an `error`
  event of type `tgv_*`.

### Upstream entries

```rust
pub struct UpstreamEntry { id: UpstreamId, provider: Provider, url: UpstreamUrl, auth: UpstreamAuth }
pub enum UpstreamAuth { ClientOwned, OperatorKey(OperatorKey) }   // never a subscription token
```

`UpstreamTable` holds one client-credential entry per provider (named after it)
and operator-key entries from `[[upstreams]]`. Keys are read through `HostEnv`
when the daemon starts and never appear in `Debug`. Dispatching to an operator-key
entry is a _protected override_: the client's credentials and the OAuth
`anthropic-beta` value are replaced by the key. It is recorded with the upstream
ID, never with header values. Codex accepts no operator keys.

### Transformations

A transform step is `set_header`, `remove_header`, `json_set`, or `json_remove`
(JSON Pointer), plus the protected credential override above.

- **Protected headers and paths** may not be touched by normal steps: credentials,
  client identity (`user-agent`, `originator`, `x-app`), beta headers, framing
  headers, and identity or position-sensitive body fields (`/system`, `/metadata`,
  `/betas`, `/previous_response_id`, `/client_metadata`). A rule that touches them
  is refused when the configuration loads.
- **Re-serialization.** The body is re-serialized (key order preserved) only when a
  step changed it, and stored as the attempt's own request body beside the
  client's original. Both are deduplicated, so this is cheap.
- **Encoded requests.** The model is read from gzip, zstd, and Brotli requests, but body
  steps do not run on them and they are not re-encoded.
- **Never transformed:** responses and error bodies.

### Retry and failover

Retry eligibility (`retry`) is a pure function of three separate facts:

```rust
pub enum WireFact {                        // what happened on the wire, from a SendFailure or a status
    NotAccepted,                           // connect/DNS/TLS error, HTTP/2 REFUSED_STREAM
    AcceptanceUnknown,                     // write on a reused connection failed, header timeout
    Responded { status: u16 },             // status line received
}
pub enum Delivery { Nothing, Started }     // has anything reached the client?
```

| Facts                                             | Default                           | Can policy allow it?                                         |
| ------------------------------------------------- | --------------------------------- | ------------------------------------------------------------ |
| Not accepted, `Delivery::Nothing`                 | Try the next compatible candidate | Yes                                                          |
| `Responded` with 429/529/5xx, `Delivery::Nothing` | Pass through verbatim             | Yes, e.g. a plan-limit 429 falls over to an operator API key |
| `AcceptanceUnknown`, or `Delivery::Started`       | Never retry                       | No                                                           |

- "Not accepted" requires positive evidence: Reqwest's `is_connect()` (recorded as
  `not_sent`), or a remote HTTP/2 `RST_STREAM` with `REFUSED_STREAM` (recorded as
  `refused`, since the request was sent but closed before processing). Handing a
  body to a pooled connection doesn't count.
- A refused attempt's response is read (64 KiB and 5 s at most) and recorded
  before the next candidate is tried. Each retry is a distinct attempt.
- **Why retries are narrow.** The client runs tools and only ever receives one
  response, so duplicate tool execution is not the risk. The risks are duplicated
  subscription allowance, duplicated server-side tools (for example web search),
  and corrupted stored-response state.
- WebSocket calls never fail over: `previous_response_id` state is scoped to the
  connection.

### Lifecycles

Each transport has a pure state machine with explicit terminals:

- **`HttpAttempt`** (`attempt`). The relay is a thin shell that feeds it events
  from the upload body, the dispatch waiting for the response head, and the
  response body, and runs the actions it returns. Transitions return a typed
  error when illegal. Events include `Dropped`, so a handle dropped without an
  observed end is still an explicit event.
- **`WsChannel`** (`channel`) owns its calls' states, because calls end through
  channel events (a close ends every open call). It is generic over the response
  identifier, so interrupts and their answers match the response they name.

An attempt's terminal carries the acceptance fact (`body_complete`, `not_sent`,
`refused`, `acceptance_unknown`, `stream_failed`, `cancelled`, `interrupted`, and
`terminal_event` on WebSocket), with a separate `terminal_cause` and
`terminal_phase`. Status stays separate from the transport. Dropping a response
body without an observed end is `interrupted`, because a drop does not prove who
stopped it; `cancelled` by the client is recorded only when the client's upload
fails before the response head. The one exception is a body dropped after
exactly its declared `content-length` was relayed (`LengthDelivered`): that is
the body's end by its framing, recorded as `body_complete` with cause
`declared_length`. It happens when an HTTP/2 upstream sends END_STREAM after the
last DATA frame and Hyper, writing HTTP/1.1 to the client, stops polling once the
length is written. Reaching the length while the body is still being read ends
nothing, so trailers still arrive.

### WebSocket relay

- One client socket is pinned to one upstream socket (`Upstream::connect`, an
  HTTP/1.1 client with Reqwest's upgrade). Two pumps, one per direction, and the
  shutdown watcher run in the channel's one task, which Axum spawns on upgrade.
  Each pump forwards a message before it reads the next one, so messages are never
  buffered without bound.
- Each socket's sink sits behind one lock. An operator's `response.interrupt` goes
  through the same upstream sink as forwarded frames, so the two never interleave.
  Operator cancels reach the relay through an unbounded channel that carries
  cancel requests, not message data.
- Each side answers its own pings. Extensions are not negotiated on either leg, so
  `Sec-WebSocket-Extensions` is never forwarded.
- Admission counts channels (`websocket.max_channels`) and in-flight calls
  separately, so idle sockets hold no call permits. Beyond the channel limit, and
  in `websocket.mode = "refuse"`, handshakes get `426` and Codex falls back to
  HTTP/SSE. Each call waits for call admission, and the client's next message waits
  with it.
- Close frames are sent after the pumps stop, so a blocked write cannot hold one
  back.

### Operator controls

| Control                          | Effect                                                                                                                                                                                                                  |
| -------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Cancel (no output delivered yet) | A TGV-labelled, non-retryable error (`409`, `x-should-retry: false`, a body distinguishable from upstream errors). Optionally blocks the session briefly, so the client doesn't simply retry.                           |
| Cancel (output already flowing)  | HTTP: close the upstream; the client sees an interrupted stream. WebSocket: inject `response.interrupt`, keep the matching `interrupt.accepted/failed` reply away from the client, and pass the terminal event through. |
| Pause                            | An immediate `503` with `retry-after`. No hold in the gateway, so no buffers or admission permits are tied up. The clients wait.                                                                                        |
| Block                            | A session block that returns a non-retryable `403`.                                                                                                                                                                     |

`Controls` holds the operator state and a registry of running calls. Every command,
including a cancel, makes a new revision and is applied only after the recorder
has reserved room to record it; a command that cannot be serialized is refused
(`500 unrecordable`). An operator interrupt sent before `response.created` waits
for the response ID.

### Shutdown

1. Stop admitting new calls (`503`) and close live feeds.
2. Let running calls continue for up to 10 seconds.
3. Close idle WebSocket channels with code 1012 ("service restart"); a channel
   with a call running closes once that call ends or the deadline passes.
4. Cancel what remains: bodies record `cancelled` with cause `shutdown`, and a call
   whose upload is still arriving is answered `503` with the rejection `shutdown`.
   Connections get 2 seconds to close.
5. Stop HTTP, then drain the recorder (a separate 10-second budget).
6. Release the storage lock.

The ingress waits for upgraded channels through a task tracker. A binary upgrade
still cuts streams that outlive the deadline.

---

## 6. Observation plane

### Capture path

One ordered pipeline with reserved terminals (`CallTap`, and `ChannelTap` for a
WebSocket's end).

- **Reserved terminals.** Admitting a call reserves its lifecycle records (open,
  finish, and per planned attempt a start, response head, and finish) before
  forwarding starts, so terminals are never lost for queue pressure. A reservation
  is released when its record is dequeued, and a call permit is held until its
  finish is dequeued. The recorder admits `4 × max_concurrent_requests` calls, and
  the queue holds `queue_capacity + 3 × max_calls` items. If even the reservation
  is unavailable, the call is unrecorded and counted.
- **Ordering.** Data items carry a sequence number per call. The recorder
  finalizes a call only after its data has been drained or accounted as omitted.
- **Data budget.** Body bytes are bounded by slots (`capture.queue_capacity`) and
  bytes (`capture.queue_bytes`, default 64 MiB); the captured prefix is 16 MiB per
  direction. Overflow is dropped and counted by reason: `policy_limit`,
  `queue_bytes`, `queue_slots`, `storage_failure`, and `unattributed` (WebSocket
  messages while no call is open). `unrecorded` counts calls; the other reasons
  count bytes. Counters are in `RecorderMetrics`, readable after shutdown through
  `RecorderMeter`, and reported through `/health`, which does not depend on the
  capture queue.

### Recorder

`RecorderTask` runs inspection, digests, group commits, retention,
and the live broadcast, writing through `RecordSink`. It runs on the observation
thread: a thread of its own with a current-thread runtime
(`tgv_daemon::host::ObservationThread`), so its CPU work (chunking, hashing,
inspection, deriving and sealing digests) and its storage writes never occupy a
forwarding worker. Taps reach it only through the bounded queue.

- **Group commit.** Every 50 ms (`capture.commit_interval_ms`), or sooner at 512
  operations or 4 MiB of chunk data, with `synchronous=NORMAL`. A power loss can
  lose the last batch. Hashing and compression run in `spawn_blocking`.
- **Commit failure.** The recorder retries the batch without body data, so
  lifecycle records survive and the dropped bytes count as `storage_failure`.
- **Response tails.** Every open response chunker is flushed at each group commit
  so live previews advance. Chunk boundaries of a streamed response therefore
  depend on commit timing; boundaries resynchronize on content, so deduplication
  across turns is affected only near cuts. The synthetic multi-turn test measures
  about 3.4× (asserted above 3×).
- **Live broadcast** is a bounded ring of committed observations. A slow
  subscriber gets `lagged` and catches up from the database.
- **Digests.** As a call's bytes flow, the recorder gathers its request
  (`RequestReader`) and reads each attempt's response activity alongside
  inspection. When the call finishes it interprets the final attempt's reading
  for the call's harness (`InterpretedCall`) and seals it (`CallDigest::seal`):
  every text redacted, then cut to `DigestBudget::STORED`. The digest is written
  in the same batch as the call's finish.
- **Rederive.** A pass on the recorder task asks the sink for finished calls whose
  digest is missing or older than `CallDigest::VERSION` (25 at a time, newest
  first; every 250 ms while some wait, else every 30 s) and replays their stored
  bodies through the same readers. Tests check that a digest derived while
  recording equals one derived from storage, for HTTP and WebSocket calls.

### Inspection

`CallInspector` runs per call inside the recorder, while bytes flow, over SSE
events and WebSocket messages alike.

- Every wire enum keeps an `Unknown` variant with the raw value.
- Malformed input becomes a typed decode error, recorded as a limitation.
- Outcome precedence is defined in one place (`ModelOutcome::merge`); `StopReason`
  and `ResponseStatus` cover every documented value plus `Unknown`.
- Parsing stops at the first capture gap. Gzip, Zstandard, and Brotli are decoded
  only for inspection; capture and forwarding keep the compressed bytes.
- Bounds: 1 MiB per SSE event, 16 MiB per JSON body, 64 MiB of decoded output, and
  128 distinct tool calls.
- Stored inspection columns carry an `inspection_version`.

### Activity

Activity is what a call's bodies show an agent doing. It is derived once, by
the recorder, when a call finishes, and stored as the call's digest (see
Recorder). `RequestReader` gathers a request's newest user input (the messages
or input items after the model's last output: user text, and tool results with
their error flag); the inspection pipeline hands every response frame to a
`ResponseReader` for its text and tool calls with their inputs, from a document,
SSE events, or WebSocket messages, after the same decompression inspection uses.
`InterpretedCall` (`tgv-harness`) applies what each client's tools touch, which
user text is the client's own context, and how Codex reports exit codes.
`CallDigest::seal` redacts every text and cuts it to `DigestBudget::STORED`:
prompt and reply 4,000 characters, each tool input and output 2,000, each
searchable field 32 KiB. The store's `activity` module assembles a turn from its
calls' digests, matching tool results to the tool calls they answer anywhere in
the turn, and cuts excerpts further to the view's budget. No body is read. A
tool call is requested, not executed; a status is client-reported; what could
not be read is a limitation, and a call whose digest is not derived yet says so
(`not_yet_derived`).

The session report (`activity::report`) reads call rows and the exposures table.
Per-call metrics come from the provider-reported usage, by its input
accounting, and recorded times; findings (`FindingKind`) apply fixed thresholds
and state measurements, never causes. Turn activity adds the findings that need
tool results.

### Search

Search reads the full-text index `search` (SQLite FTS5, `unicode61` tokenizer,
external content over `search_texts`) in `tgv.sqlite3`: each finished call's
prompt, reply, tool input (name, targets, input), and tool output, redacted, at
most 32 KiB per field, written with the call's digest. Retention removes a call's
rows with it, and the index's pages count toward the storage budget as part of
the metadata file. Search terms become quoted FTS5 strings joined by `AND`, so
input never reaches the query grammar.

`SecretScanner` (`tgv-recorder::secrets`) finds credentials in fixed, documented
formats with linear-time regular expressions, never by entropy. Sealing a
digest scans the call's raw newest user input (client context included), reply,
and each tool input and output, redacts what it finds, and keeps each exposure's
kind, field, masked hint, and a truncated BLAKE3 fingerprint in
`call_exposures`, never the secret. Turn activity and session reports show
these as findings.

### Storage

**The log.** `observations(seq, at_wall, at_mono, channel, call?, attempt?, kind, schema, payload)`.

- `kind` is plain text, parsed into an enum with an `Unknown` variant. It has no
  CHECK constraint, so new kinds need no table rebuild, and older binaries keep the
  unknown.
- `seq` is the only order across calls. `at_mono` is monotonic time since daemon
  start; `at_wall` is for display only.

**Query tables.** `channels`, `calls`, `attempts`, `sessions`, `turns`,
`operator_actions` (the newest 10,000), the `operator_state` snapshot, and the
search columns. Derived from each finished call: `call_digests` (version, prompt,
and the digest as JSON), `call_exposures`, and `search_texts` with its FTS5 index
`search` (external content, kept in step by triggers). They reference `calls`
with `ON DELETE CASCADE`, so retention removes them with their call. The writer updates them directly, in the same transaction as the
log; there is no generic reducer framework.

**Bodies.**

- `tgv.sqlite3` holds metadata, the log, and segment rows. It is small, hot, and
  served by a read pool.
- `blobs.sqlite3` holds chunks: `chunks(hash, codec, raw_len, data)`. FastCDC cuts
  them (8/16/64 KiB minimum, average, maximum), BLAKE3 identifies them, and each
  is stored with zstd only when that makes it smaller. Page size is 16 KiB.
- A body is an ordered list of `segments(body, ord, offset, len, chunk_hash)` plus
  its completeness evidence. Gaps stay explicit. `observed_bytes` is null until a
  body's end is recorded; it is never guessed.
- Each direction of a WebSocket channel is stored as a byte stream with a message
  index (offset, opcode, time, length), chunked like an HTTP body. That avoids one
  log row per delta event.

**Crash consistency.** No reference counts are stored across the two files. Chunks
commit first, then the segments that reference them, so a crash can only leave
unreferenced chunks, which garbage collection removes.

**Recovery.** At startup, each open channel or call gets a `recovered`
observation: an unknown outcome, never success. `Recovery` also carries the stored
operator snapshot; `Controls::restore` continues from it, and an unreadable
snapshot stops startup rather than starting unpaused.

### Retention

1. Delete the oldest finished calls (with their attempts, bodies, and observations)
   in transactions of at most 16, updating session counts and orphan turns and
   sessions in the same transaction. Open calls are protected; recovered calls are
   eligible.
2. Mark-and-sweep removes chunks that no segment references, using an index on
   `segments(chunk_hash)`.
3. Measure real usage from `page_count - freelist_count` on both files.
4. Repeat until under budget.

Maintenance runs at startup and every 60 seconds; full batches schedule another
pass sooner. Under deduplication, deleting the oldest call frees little, so this
loop is the honest way to meet a byte budget. Free pages are reused, so there is
no routine vacuum.

### Backup

`Backup` (run by `POST /control/v1/backup` and `tgv control backup`) writes
`<data-dir>/backup/`:

1. A `VACUUM INTO` metadata snapshot in a private staging directory, on its own
   read-only connection (`VACUUM INTO` fails under `query_only`).
2. The referenced chunks the backup lacks. Chunks never change, so this is a
   simple append.
3. A check that no referenced chunk is missing (`503 backup_incomplete`
   otherwise).
4. A rename of the snapshot into place, then removal of chunks it no longer
   references.

A `SweepGate` held for the whole backup defers the chunk sweep, so retention
cannot remove a chunk the snapshot references. The location is fixed so a remote
operator cannot direct writes elsewhere on the host. A backup changes no operator
state; it is logged, not recorded in `operator_actions`.

---

## 7. Management plane

- **Reads.** A pool of query-only connections, each holding short transactions.
  Body exports read segment by segment in separate short transactions; committed
  bodies never change. Checkpoints run periodically on both files, and
  `journal_size_limit` is set on both. Busy readers return `503` without blocking
  forwarding.
- **Pagination.** Keyset pagination through one `Cursor` type.
- **Responses.** Typed DTOs. Each carries `api_version`; the UI checks
  compatibility at load and shows an explicit mismatch error.
- **Live feed.** `/api/live?after=<seq>` (or `Last-Event-ID`) replays retained
  observations, then streams new ones; it emits `gone` when the position was
  pruned. The SvelteKit server streams it without buffering, through an explicit
  allowlist entry.
- **Health and metrics.** `/health` and `/api/metrics`: queue depth and bytes,
  omissions by reason, commit latency and lag, page counts for both files, and
  deduplication.
- **Control API** (`/control/v1`). Requests with `Origin` or `Sec-Fetch-Site`
  (browsers) are refused. A request through Tailscale Serve must carry a
  `Tailscale-User-Login` listed in `control.tailscale_logins`. Otherwise it must
  carry the per-run token from `control.token` (mode 0600) in the data directory.
  Loopback alone is not enough, because Serve forwards tagged devices and Funnel
  traffic to loopback without identity headers.
- **Configuration.** A raw TOML `Config` becomes a `RuntimeConfig` through
  `TryFrom`, with a revision. The gateway endpoint (`GatewayEndpoint`) and the
  upstreams (`UpstreamUrl`) both require HTTPS, or HTTP on loopback, without
  credentials, queries, or fragments. The host environment (`HostEnv`) is read once, in
  `main`.
- **Redaction.** One allowlist decides which header names are stored. Header
  values are never stored, except allowlisted correlation UUIDs.

---

## 8. Testing

- **Test seams** are `Clock`, `IdSource`, `Upstream` (HTTP request and WebSocket
  connect), and `RecordSink`. Everything else is concrete. The gateway is generic
  over `U: Upstream`, since the seam is a test substitution rather than runtime
  variation.
- `tgv-protocol`: property tests over arbitrary chunk splits, and synthetic event
  sequences written in the tests.
- `TestGateway` (`crates/tgv-daemon/tests/support`): the complete router with the
  real recorder and store in a temporary directory, a fixed clock, and sequential
  IDs, forwarding to a loopback mock upstream (for Reqwest's behaviour) or a
  scripted `Upstream` (for the seam itself).
- Store tests use SQLite in temporary directories; `tgv-store/tests/recorder.rs`
  runs the recorder against the real store.
- Full-daemon tests (`tests/daemon.rs`) only cover wiring, lifecycle, and shutdown.
- Every label is pinned in its crate's `tests/fixtures/labels.txt`.

---

## 9. Known limitations

- No `BlobStore` seam; store tests run against SQLite.
- No re-inspection of stored bodies after an inspector change, and no manual
  compaction command.
- `/api/metrics` reports page counts but not WAL sizes.
- HTTP/2 streams above a GOAWAY's last stream ID are not detected as refused.
- Upstream connections are not warmed while an upload is received.
- The request buffer does not keep request trailers.
- WebSocket sockets have no idle timeout; a quiet call is bounded only by the
  client.
- `previous_response_id` continuity is not tracked.
- The `426` fallback is confirmed in the Codex source, not against a release.
- A channel whose handshake call is pruned before it closes keeps its row.
- The public OpenAI WebSocket mode is refused.
- The UI has no control actions.
- Backups go only to `<data-dir>/backup`, and only through a running daemon.
- Activity keeps excerpts, not full text (`DigestBudget::STORED`); a turn reads
  at most 100 calls, and turn listings have no paging. Reports do not detect
  compaction, and time to first token is not recorded.
- Search matches whole words (punctuation splits them) and indexes at most
  32 KiB per field of each call.
- The container image is built from a checkout; none is published, and
  `install-vps` does not manage containers.
