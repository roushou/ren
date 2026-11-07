# TGV architecture v2

Status: phases 1 to 7 are built and deployed (November 2026); phase 8,
deterministic simulation, is not started. This is the design record: the
plan as approved, and under "Deviations recorded" how each phase was actually
built. For how TGV works now, read `docs/architecture.md`, which describes the
current state.

The plan's premise: backward compatibility is not a goal, so storage, the
management API, the CLI's JSON, and the MCP tools may change, while the
operator's recorded history is kept (section 6).

## 1. Why change

TGV's original architecture is sound: a data plane that forwards, an
observation plane that records, a management plane that reads, around an
append-only observation log. Its own rule was that **inspection is
incremental, while bytes flow, never a re-read**. The features added since
(activity, reports, search, secret detection) broke that rule, and the code
shows it:

| Problem                            | Evidence                                                                                                                                                                                                                                                                               |
| ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Bodies are parsed three times      | The recorder inspects responses as they stream (`RecorderTask::accept`). The indexer re-reads and re-parses every finished call (`Indexer::pass` → `read_documents` → `read_call`). The turn view re-parses up to 200 bodies, 16 MiB each, on every request (`Reader::turn_activity`). |
| CPU work on the forwarding runtime | Inspection runs on the recorder task; activity parsing and secret scanning run in API handlers and the indexer, all on the runtime that relays traffic. Only previews use `spawn_blocking`.                                                                                            |
| One fact, several computations     | Prompts: indexer and read-time fallback. Exposures: indexer table and recomputed live by the turn route. Turn end time: SQL, `TurnActivity`, and `SessionReport`, which disagreed until batch 1.                                                                                       |
| A second store to keep consistent  | `search.sqlite3` needs its own writer, cursor, sweep, stale-row budget accounting, corruption rebuild, and version handling.                                                                                                                                                           |
| Local and remote reads diverge     | Ten CLI commands have a local path (typed structs) and a remote path (untyped JSON), with orchestration duplicated (report + exposures, budgets, index availability).                                                                                                                  |
| Hidden state machines              | The call's candidate loop in `Gateway::forward` (an index and three `Option`s); `AttemptCell`'s mutexes; the recorder's `accept`. The recorder's tap, queue and task have no unit tests.                                                                                               |

v2 restores the original rule and extends it: **every fact derived from
traffic is derived once, at write time, by one pure pipeline, off the
forwarding runtime, and stored with a version so it can be derived again.**

## 2. Shape

```text
                 ┌──────────────── forwarding runtime (multi-thread) ───────────────┐
clients ───────► │ ingress → CallRun (pure) → HttpAttempt / WsChannel (pure) → relay │ ──► providers
                 │                    tap: try_send only, never parses              │
                 └────────────────────────────────┬─────────────────────────────────┘
                                                  │ bounded queue (unchanged rules)
                 ┌──────────── observation runtime (own thread) ────────▼───────────┐
                 │ RecorderCore (pure): chunk · inspect · read request · digest     │
                 │   · interpret (harness) · redact (scanner) · roll up turns       │
                 │ shell: group commit → tgv.sqlite3 (+ blobs) · live broadcast     │
                 │ Rederive: replays stored bodies through the same core            │
                 └────────────────────────────────┬─────────────────────────────────┘
                                                  │ rows only
                 ┌────────────────────── management plane ──────────▼───────────────┐
UI · CLI · MCP ► │ ReadRequest → Reader (SQL over projections) → View               │
                 │ one route table: HTTP codec (server and client) · local · MCP    │
                 └──────────────────────────────────────────────────────────────────┘
```

Three structural moves:

1. **Projections at write time** (section 3): a per-call digest, turn rollups,
   exposures, and the search index are written in the same transaction as the
   call's finish. Reads never parse bodies, except an explicit body preview or
   export.
2. **An observation runtime** (section 4): the recorder, its CPU work, and its
   storage writes run on their own thread. Forwarding cannot be slowed by
   parsing, scanning, hashing, or compression, by construction rather than by
   discipline.
3. **One read service** (section 5): a typed `ReadRequest` enum served by the
   `Reader`, with one route table that is both the server's parser and the
   client's encoder. CLI local, CLI remote, MCP, and HTTP produce the same
   typed views.

Plus two state machines made explicit (section 7) and a test strategy that
the pure cores make possible (section 8).

## 3. Projections

### 3.1 The digest pipeline (pure)

One pipeline turns a call's bytes into its stored digest. Every stage is pure
and lives in the lowest crate that owns its meaning:

```text
tgv-protocol   CallReader       request bytes + response bytes/events → Reading
                                (merges today's `inspect` and `activity`)
tgv-harness    Harness::interpret(Reading) → Interpretation
                                (prompt without client context, tool effects,
                                 exit codes, result status)
tgv-recorder   Digest::seal(Reading, Interpretation, &SecretScanner, DigestBudget)
                                → CallDigest   (redacted, cut, versioned)
```

- **`CallReader`** replaces both `CallInspector` and `ActivityReader`. The
  response side stays incremental (fed per piece, gap-aware, fragmentation-
  independent). The request side reads the buffered request once, when its
  body ends. Today's `InspectionRecord` (outcome, usage, model, stop reason,
  tool names, counts) becomes part of the `Reading`, so there is one parser per
  wire format instead of two.
- **`Reading`** holds raw text: newest user input, tool results with outputs,
  tool uses with inputs, reply text, and the inspection facts. It never leaves
  the recorder.
- **`CallDigest`** is what is stored. Its text is redacted (the scanner moves
  from `tgv-store` to `tgv-recorder`: sanitize before anything is persisted)
  and cut to a stored budget large enough for every view (the full text stays
  available through body preview). It carries `DIGEST_VERSION`.
- Tool uses (call N's response) and their results (call N+1's request) are
  paired when a turn is assembled, by tool-call ID. Each digest stores only its
  own side, so a digest never depends on another call.

### 3.2 Storage (one file again)

A new migration adds, in `tgv.sqlite3`:

| Table              | Row                                                                                                                                     | Written                                |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| `call_digests`     | one per call: version, prompt, reply, tool uses (input excerpt, effects), tool results (status, exit code, output excerpt), limitations | at `FinishCall`, same transaction      |
| `exposures`        | kind, field, hint, fingerprint, count per call                                                                                          | same transaction                       |
| `search`           | FTS5, external content over `call_digests` text                                                                                         | by triggers on `call_digests`          |
| `turns` (extended) | rollups: call count, open calls, errors, tokens, first and last times, prompt excerpt                                                   | updated at `OpenCall` and `FinishCall` |

Consequences:

- **Retention is one delete.** Deleting a call deletes its digest, exposures,
  and search rows (`ON DELETE CASCADE`, which works since the store enables
  foreign keys, plus FTS triggers). No sweep, no stale rows, no
  `IndexUsage` proportional budget: the FTS pages are simply part of
  `tgv.sqlite3`'s measured size.
- **Deleted:** `search.sqlite3`, `SearchIndex::open/open_read/usage`,
  `Indexer`, `IndexCursor`, `Backlog`, `PassReport`, the daemon's third task,
  the corruption rebuild, `turn_prompts`, and the read-time prompt fallback.
- **Turn end time is one rule in one place:** a turn has ended when its
  rollup's `open_calls` is zero.
- **Backups include the search index.** Larger backups, but a restored backup
  is immediately searchable.

### 3.3 Rederive

A projection must be rebuildable from its source. `Rederive` is a
background job on the observation runtime that replays stored bodies through
the same pure pipeline:

- On start, it finds calls whose `digest_version` is older than the current
  one (or missing) and re-derives them in bounded batches, newest first.
- It replaces today's "no re-inspection after an inspector change" limitation:
  inspection is part of the digest, so a parser fix reaches old calls.
- It is also how existing history is migrated: after upgrading, the 80 calls
  recorded so far get digests within seconds.
- The invariant **"derived while streaming == derived again from stored
  bodies"** is a property test (section 8), which also checks that capture
  loses nothing a digest needs.

### 3.4 What reads become

| View                 | v1                                           | v2                                             |
| -------------------- | -------------------------------------------- | ---------------------------------------------- |
| Turn activity        | parse ≤ 200 bodies, scan secrets             | read ≤ 100 digests, pair tools by ID           |
| Call activity        | parse 2 bodies                               | read 1 digest                                  |
| Turn listing         | index lookup, else parse ≤ 3 bodies per turn | `SELECT` from `turns`                          |
| Session report       | ≤ 5000 call rows + index exposures           | turn rollups + digests' metrics + `exposures`  |
| Search, exposures    | second database                              | same database, same transaction boundary       |
| Body preview, export | parse 1 body (blocking pool)                 | unchanged: the one explicit read of raw bodies |

## 4. Observation runtime

The daemon builds two runtimes:

- **Forwarding runtime** (multi-thread): ingress, forwarding, relays, the
  read API, controls.
- **Observation runtime** (current-thread, its own OS thread): `RecorderTask`,
  its store writer, `Rederive`, and the live broadcast source.

The tap side is unchanged: non-blocking reservations and `try_send` into the
bounded queue, with the existing overload rules and omission counters. What
changes is where the work lands: chunking, inspection, request reading,
interpretation, redaction, hashing, and compression all run on the
observation thread. Hashing and compression keep `spawn_blocking` there (they
are the heaviest and parallelize). If derivation falls behind, the queue's
existing limits apply; a call whose digest could not be derived is stored as
`digest: deferred` and `Rederive` completes it later. Lag is never rounded up
to success.

Spawning stays a capability: the daemon's `Spawner` implementation owns both
runtimes and the only `disallowed_methods` exemptions.

## 5. One read service

### 5.1 `ReadRequest` and `Reader`

```rust
// tgv-store
pub enum ReadRequest {
    Calls(CallQuery), Call(CallId), SessionCalls(SessionId, PageQuery),
    Sessions(PageLimit), Turns(SessionId, PageQuery), Turn(TurnId),
    CallActivity(CallId), Report(SessionId), Usage(TimeRange),
    Search(SearchQuery), Exposures(ExposureQuery), Storage,
}
pub enum ReadResponse { Calls(CallPage), Call(CallDetail), … }

impl Reader {
    pub async fn serve(&self, request: ReadRequest) -> Result<ReadResponse, StoreError>;
}
```

All views derive `Serialize` and `Deserialize`, so a remote answer is the same
typed value as a local one. Report-plus-exposures, budgets, and paging live in
`serve`, once.

### 5.2 One route table

`tgv-gateway` owns a declarative table: each `ReadRequest` variant ↔ method,
path pattern, and query codec. It is used in both directions:

- the server parses HTTP into a `ReadRequest` (replacing `params.rs`'s `Raw*`
  structs and the handlers' repeated permit/parse/map shape);
- the CLI's `ApiClient` encodes a `ReadRequest` into a URL and decodes the
  typed `ReadResponse` (replacing `management.rs`'s hand-written paths,
  `QueryParams`, and untyped `Value`s).

So `History = Local(Reader) | Remote(ApiClient)` serves every history
command and every MCP tool through one call: `history.serve(request)`. MCP
tool arguments parse into `ReadRequest` (the MCP schema stays generated from
`ToolName`).

### 5.3 The UI contract

Rust tests write one JSON sample per `ReadResponse` variant to
`ui/tests/fixtures/api/`. The UI's decoder tests parse every sample, so a
change on either side that breaks the contract fails a test instead of a page.
The UI's hand-written decoders stay (parse at the boundary); the fixtures pin
them to the server. The API version becomes 2.

## 6. Storage changes and upgrade

- One new migration (shipped migrations are not edited). It adds the tables
  in 3.2 and drops nothing the recorder still writes.
- `search.sqlite3` is deleted on first start of v2.
- Existing calls get digests from `Rederive`. Nothing recorded is lost.
- Pruning, backup, and recovery gain the new tables through foreign keys and
  the existing transaction boundaries.

## 7. State machines made explicit

### 7.1 `CallRun` (gateway)

The candidate loop becomes a pure machine, beside `HttpAttempt`:

```text
Planned ─start(k)→ Attempting{k} ─refused & next eligible→ Attempting{k+1}
                         │                └─ (drain refusal, hand admission over)
                         ├─responded→ Relaying ─attempt ended→ Ended(terminal)
                         ├─failed, not eligible→ Ended(failed)
                         └─aborted | cancelled→ Ended(cancelled)
```

`step(state, event) -> (state, Vec<CallAction>)`, with actions such as
`StartAttempt { candidate }`, `DrainRefusal`, `HandOverAdmission`,
`Register`, `Relay`, `Finish(terminal)`. `Gateway::forward` and `handshake`
become interpreters of these actions. `AttemptCell`'s `Closing`, held
registration, and admission slots become actions emitted by `HttpAttempt`
and `CallRun` transitions instead of `Mutex<Option<…>>` juggling.

Unit tests cover every retry rule without sockets; a property test asserts
that no action sequence retries after any delivery to the client.

### 7.2 `RecorderCore` (recorder)

```rust
pub struct RecorderCore { calls: HashMap<CallId, CallInProgress>, … }
impl RecorderCore {
    pub fn accept(&mut self, item: Item) -> Vec<Op>;  // pure, no I/O, no clock reads
    pub fn flush(&mut self) -> Vec<Op>;               // at each group commit
}
```

The task becomes a shell: receive, `accept`, commit through `RecordSink`,
broadcast. The tap's capture arithmetic (offsets, body limit, 64 KiB pieces,
omissions) becomes a pure `CaptureBudget`. Both get unit tests for the
priority scenarios: gaps, saturation, ordering, attempts without data, and
digest sealing.

## 8. Testing

- **Pure cores, unit-tested:** `CallReader` (fragmentation property tests on
  both directions), `Harness::interpret`, `Digest::seal` (no secret survives;
  budgets), `CallRun`, `HttpAttempt`, `WsChannel`, `RecorderCore`,
  `CaptureBudget`.
- **Projection equivalence:** for every recorded test call, the digest derived
  while streaming equals the digest `Rederive` produces from stored bodies.
- **Read service equivalence:** for every `ReadRequest` in a corpus, `Local`
  and `Remote` answers are equal values, and each round-trips through the
  route table's codec.
- **Contract fixtures** for the UI (5.3).
- **Deterministic simulation (later):** with clock, IDs, upstream, sink, and
  spawning all passed in, a seeded harness can run gateway and recorder on a
  paused-time runtime with injected faults (refused and dropped upstreams,
  slow commits, a full queue) and check the invariants above. This is the
  payoff of 7.1 and 7.2, not a prerequisite.

## 9. Crates

Still seven crates and one `xtask`; no new crate is needed.

| Crate          | Changes                                                                                                                                               |
| -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tgv-protocol` | `inspect` and `activity` merge into `CallReader` → `Reading`; magic-byte sniffing already shared                                                      |
| `tgv-harness`  | `tools` becomes `Harness::interpret(Reading) -> Interpretation`                                                                                       |
| `tgv-recorder` | gains `secrets` (from store), `Digest`, `RecorderCore`, `CaptureBudget`; `RecorderTask` becomes a shell; `Rederive` core                              |
| `tgv-store`    | digests, rollups, exposures, FTS in `tgv.sqlite3`; `Reader::serve(ReadRequest)`; loses `search.sqlite3`, `Indexer`, read-time body parsing, `secrets` |
| `tgv-gateway`  | `CallRun`; route table and codec; handlers shrink to the table                                                                                        |
| `tgv-daemon`   | two runtimes and the `Spawner`; loses the indexer task                                                                                                |
| `tgv-cli`      | `History = Local                                                                                                                                      | Remote(ApiClient)``; ``management.rs``, ``QueryParams``, and per-command dual paths go; MCP tools map to ``ReadRequest` |
| `ui`           | API v2 decoders, contract fixtures; pages unchanged in behavior                                                                                       |

## 10. Phases

Each phase lands as commits that pass every check; behavior changes and moves
stay in separate commits.

1. **Digest core, unwired.** `CallReader`, `Harness::interpret`,
   `Digest::seal`, scanner moved to the recorder. Pure tests only.
2. **Write-time projections.** Migration; recorder derives and stores
   digests, exposures, FTS, and turn rollups; `Rederive`. Reads still use v1
   paths, and the equivalence test compares them.
3. **Reads from projections.** Turn, call activity, turns, report, search, and
   exposures read rows. Delete the indexer, `search.sqlite3`, and read-time
   parsing. Deploy here: this is the CPU fix.
4. **Observation runtime.** Daemon's two runtimes and `Spawner`.
5. **One read service.** `ReadRequest`, route table, `ApiClient`, `History`,
   MCP over it, API v2, contract fixtures, UI decoders.
6. **`CallRun`.** Then `AttemptCell` simplification.
7. **`RecorderCore` and `CaptureBudget`.**
8. **Optional:** deterministic simulation suite.

### Deviations recorded

- **Phase 1, built as:** no new `CallReader` type. The response side stays
  `CallInspector`, which now hands each frame to a `ResponseReader` as well
  and finishes with a `ResponseReading` (inspection plus activity). The
  request side is `RequestReader`, gathered per call. They stay separate
  because a request belongs to the call and a response to each attempt. The
  `Reading` the recorder seals is `{ request, response }` activity; the
  inspection report keeps its own stored record. Interpretation is
  `InterpretedCall::new(harness, reading)` (it also handles calls with no
  known client), and sealing is `CallDigest::seal`. Exposures count a secret
  in a tool's input once; the command target that repeats it is redacted but
  not counted. Measured cost: a 4.9 MB request reads in 3-10 ms (release).

- **Phase 2, built as:** no turn rollup table. A turn listing is already one
  `GROUP BY` over `calls`, which is cheap; what was expensive was its prompts,
  and those now come from `call_digests.prompt`. The digest is its own batch
  operation (`Op::Digest`, no observation row), so the same operation serves
  live finishes and `Rederive`, and a failed commit's retry drops it like body
  data (`Rederive` restores it). The tables are `call_digests`,
  `call_exposures`, and `search_texts` with the FTS5 index `search`. `Rederive`
  runs on the recorder task through a new `RecordSink::undigested` port, which
  the store answers from an internal read pool. On a copy of the operator's 80
  calls, the backfill took about 3 s and matched the current index's documents,
  prompts, exposures, and hit counts exactly.

- **Phase 3, built as:** search and exposures are `Reader` methods (the store's
  `search` module keeps the query and result types), so the gateway, CLI, and
  report read one handle; the report adds its exposures itself. Turn and call
  activity, turn prompts, and storage read digests only; `ReadCall` is a call
  row plus its digest, and a missing digest shows `not_yet_derived`. The doctor's
  check is `digests` (derived of finished calls), from a `digested_calls` count
  in `/api/storage`, whose `disk.search` is gone. On a copy of the operator's
  data, all 42 turn, activity, turn-listing, and report outputs were identical
  to the previous build's, and a 29-call turn view took 13 ms (debug build)
  instead of 110 ms.

- **Phase 4, built as:** one dedicated thread, not a second runtime handle
  passed around: `ObservationThread::start(task)` builds a current-thread
  runtime, runs the recorder task to completion on its own OS thread, and
  hands back its output through a channel (with an explicit stop for the
  shutdown deadline). The forwarding runtime is the process's existing
  multi-thread runtime; nothing else moved. The store's `spawn_blocking` for
  hashing and compression now uses the observation runtime's blocking pool.
  A unit test blocks the observation thread for 300 ms while the caller's
  single-threaded runtime keeps ticking; the daemon's lifecycle and shutdown
  tests pass unchanged.

- **Phase 5, built as:** `ReadResponse` is serialize-only; a remote answer is
  the API's JSON value rather than a typed one, because every surface only
  prints or forwards it, and typed decoding would have put `Deserialize` on
  every view for no reader. Equality holds because both sides serialize the
  same `ReadResponse`: the CLI's local-versus-gateway test compares bytes, and
  the contract fixtures pin the JSON. The "one route table" is `ApiRead`'s
  `uri` and `parse`, kept one table by a round-trip test over every read
  (including values with `&`, `?`, and `#`), and the router registers every
  read path on one handler. `ReadRequest::Calls` and friends answer with the
  API's envelopes, so `tgv history`, `sessions`, `turns`, and `exposures` now
  print `{calls, next}`, `{sessions}`, `{turns}`, and `{exposures}` like the
  API. Malformed identifiers and unknown parameters now answer JSON errors
  (`not_found`, `invalid_query`) instead of the framework's plain text. On a
  copy of the operator's data, all 18 read bodies equal the previous build's
  apart from `api_version`.

- **Phase 6, built as:** `CallRun` (`tgv-gateway::call`) with `step(dispatch,
  retry) -> CallStep<R>`, generic over the response the shell holds, so each
  step carries what it acts on (`FallOver { refused, next }`, `Relay(R)`,
  `Stop(CallStop)`) and the shell has no impossible combinations to match.
  Recording and permits stay in the shell: the recorder tap and control
  registration still start on the first attempt, and the admission permit is
  still handed between attempts by `AttemptCell`, which this phase did not
  change. Unit tests cover each retry rule; a property test runs random
  outcome sequences. The gateway's end-to-end fallback tests pass unchanged.

- **Phase 7, built as:** `RecorderCore` (`recording`) with
  `accept(Record)`, `take_batch()`, and `rederived(exchanges)`; the batch
  accumulates inside the core rather than each call returning operations, which
  keeps the existing `Batch` (chunks plus operations) as the one output.
  `Record` is the queue item without its permits, and `Item::into_record`
  separates them. The tap's capture arithmetic is `CaptureOptions::split`. The
  move landed first as its own commit with the workspace tests unchanged; the
  core's unit tests (lifecycle, chunks, inspection, digest, gaps, failover,
  mid-stream commits) and the split's property test followed. Deterministic
  simulation (phase 8) is not started.

## 11. Trade-offs and decisions for review

- **Write-time cost.** Every call's request is parsed once, viewed or not.
  It runs on the observation thread, and only once instead of once per view
  (today the indexer already parses every finished call once). Phase 1
  measures it on a multi-MB conversation before anything is wired.
- **Stored budgets are fixed at write time.** Excerpts in digests are capped
  (proposed: 2,000 characters per tool output, 32 KiB per searchable field).
  Changing them means a version bump and a `Rederive` pass; full text remains
  a body preview away.
- **FTS in `tgv.sqlite3`** makes retention and budgets exact and removes a
  file, at the cost of larger backups and FTS writes in the group commit
  (off the forwarding path). Alternative: keep a separate file, fed by the
  recorder instead of an indexer. Recommended: one file.
- **A second runtime** isolates forwarding by construction. Alternative:
  `spawn_blocking` around the CPU stages only, which is smaller but relies on
  every future change remembering it. Recommended: the runtime.
- **API v2 breaks clients.** The UI, CLI, and MCP server ship from this
  repository together; MCP clients re-read tool schemas each session.
- **Out of scope:** event-sourcing the application, actors beyond the
  recorder, a generated TypeScript client.
