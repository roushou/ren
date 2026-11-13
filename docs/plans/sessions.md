# Plan: sessions you work in

Status: in progress, on `agent-runs`. Steps 1 and 2 are done; step 3 is next. This file is
the plan of record: each step lands as its own commits, every commit
passes the full checks, and any change to the plan is written under
[Deviations](#deviations) with its reason. Once the work is done, what
it describes moves into `architecture.md` and the operator guides, and
this file is removed.

## Why

Ren calls the thing an operator drives a _run_: one client process, from
start to end. That one word does three jobs:

1. **What the operator works in**: a folder, a client, settings, a
   draft, and a conversation. It is the operator's, and it is edited.
2. **One lifetime of a client process**: starting, idle, working,
   ended. An implementation detail.
3. **A thing shown next to observed traffic**: the terminal app's list
   mixes runs with watched sessions, and the web Runs page borrows the
   read-only views.

Tied together, they force poor choices: a draft cannot exist without a
process, continuing a conversation makes a new run, the composer's
`/` and `@` offers hang off a process, and an unused draft would need a
process started early and an invented end to hide it.

## The model

Three concepts, each with one job.

- **Session**: what the operator works in. Read-write. Created by `+`
  on a project (or Scratch). It owns its folder; its client, fixed once
  the first prompt is sent; its settings (model, effort, approvals,
  changed from the next turn); its draft; its whole conversation across
  every process; and its own state: `draft`, `active`, or `archived`.
  Stored by the daemon, so the browser and the terminal see the same
  sessions. Its identity is Ren's (`SessionId`); the client's own
  conversation identity is kept beside it (`NativeThreadId`), as
  recordings keep the client's session identity.
- **Process**: one lifetime of the client serving a session. Started
  when one is needed (opening the session, or the first prompt after an
  idle stop), resumed through the client's own resume, stopped when idle.
  The pure lifecycle machine (today's `RunState`) keeps its interrupted
  and unknown ends. Interfaces show it only as a status of the session
  ("Claude Code running", "stopped"), never as an entry of its own.
  Ending a process never ends the session.
- **Recording**: what Ren observed on the wire, grouped by the client
  conversation each call was attributed to. Read-only. Today's observed
  "session". Live, Traffic, and Recordings show recordings; a session
  links to its recordings and never reuses their views.

The composer belongs to the session: `@` searches the session's folder
(no process needed), `/` and `$` offer the client's commands once a
process is up, and model, effort, and approvals change from the next
turn.

## Naming rules

"Session" now means the operator's session. Two uses of the word stay:

- **The client's own sessions.** Claude Code's `--session-id`, Pi's
  sessions, and `NativeSessionId` are the client's vocabulary and keep
  it.
- **Serialized names**, until a separate decision on the read API:
  labels pinned in `tests/fixtures/labels.txt`, identity kinds in error
  text (`SessionId as "session"` becomes `RecordingId as "session"`),
  JSON field names, read API routes (`/api/v1/sessions…`), CLI commands
  and flags, MCP tool names, and database columns. Renaming any of these
  changes stored data, the API, or CLI output, which a rename must not.

Everything else that names the observed concept says _recording_: Rust
types, modules, functions, and non-serialized fields; TypeScript types
and modules; and every word the web UI and terminal app show.

## Steps

Each step is one or more commits that pass `dprint check`, `cargo fmt`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test
--workspace`, and, for UI changes, the UI's format, check, and tests. A
structural move and a behaviour change never share a commit. Unchanged
behaviour is checked by comparison: the pinned labels, the test count,
and `ren --help` and `ren --version` output against the commit before.

1. **Observed sessions become recordings.** No behaviour change.
   - 1a. Rust: the observed identity and its types (`SessionId` →
     `RecordingId`, `SessionView` → `RecordingView`, the store's reads
     and query types, the terminal app's `Sessions` model and its
     panes), their modules and functions, and their non-serialized
     fields and variables. Serialized names stay (see the rules).
   - 1b. Web UI: TypeScript types and modules, the nav entry and page
     ("Sessions" → "Recordings", with `/sessions` URLs redirected to
     `/recordings`), and every label. The terminal app's labels too.
   - 1c. Docs: `AGENTS.md`, `architecture.md`, and the operator guides
     say recording for the observed concept.
2. **Sessions in the core.** The session's identity in
   `ren_recorder::id`, its recorded facts (created, configured, drafted,
   archived) with labels pinned, its pure state machine in
   `ren-supervisor` (`draft` → `active` → `archived`, with typed refusals),
   and a store migration that gives every existing run a session of its
   own, the run becoming its first process. Reads return sessions with
   their processes.
3. **The supervisor serves sessions.** Processes per session: started
   on demand, resumed for the same session, stopped when idle (a session
   without a prompt yet loses nothing). Continuing is the same session.
   `@` searches the session's folder without a process; `/` and `$` come
   from its client's process.
4. **The control API.** `/control/v1/sessions…`: create in a place,
   configure (client while still a draft; settings from the next turn),
   prompt with its delivery, unqueue, answer, interrupt, stop the
   process, archive, commands, files, and a session feed. The `/runs`
   routes keep working until both interfaces have moved.
5. **The interfaces.** Web: `+` on each project and on Scratch in the
   sidebar creates a session and opens it; one composer for every
   session, with client and settings in its bar; process status in the
   header. Then the terminal app: `n` on a folder creates a session
   there; its list holds sessions only, recordings stay in their own
   view.
6. **Removal.** The `/runs` control routes, the full-screen new-run
   form, and run-only code paths go. `docs/api.md` and the guides
   describe sessions.

## Open questions

- Recording vocabulary on the read API, CLI, and MCP: keep "session"
  there, or rename with a version bump. Decided after step 6.
- When a draft session's client starts: on opening the session, or on
  first focus of its prompt if a client writes its own history entry
  merely by opening a conversation. Checked against each client in
  step 3.

## Deviations

- **1a left local variables and plain struct fields named `session`**
  where they hold a `RecordingId`, outside the terminal app (whose model
  was renamed whole). Most such fields are serialized (views, queries,
  API bodies) and keep their names by the rules; the rest are local to a
  function and typed `RecordingId`, so the type says what they are. They
  are renamed when that code is next changed.
- **1a kept these serialized or client-owned names:** the operator's
  session blocks (`BlockSession`, `SessionBlocked`, `blocked_sessions`:
  control API and stored labels), MCP tool names (`SessionCalls`,
  `SessionReport`), the `ren history sessions` command (`SessionsCmd`),
  `HistorySubject::Session` (a stored label), the attribution hints of
  the client's own session (`SessionHint`, `TurnWithoutSession`), and the
  dev seed's `sessions` scenario (a `cargo xtask dev seed` value).
- **2 stores operator sessions in `operator_sessions`**, not `sessions`,
  which holds recordings and keeps its name by the rules. A run names its
  session in `runs.operator_session`: `runs.session` would read as the
  run's recording, which the read API already returns as `session`.
- **2 records one session fact, `Created`** (with `RunSession` on a run's
  start). Settings changed mid-run update the session with its run
  (`RunFact::Configured`), and while each session has one run, archiving
  or deleting the run archives or deletes its session. `Configured`,
  `Archived`, and `Unarchived` facts of the session itself arrive in step 4,
  with the commands that make them. Drafts stay in each interface until
  step 5 decides whether the daemon keeps them.
- **2 backfills each earlier run's session under the run's own id.** SQL
  cannot mint ids, and the two are different identities that may share a
  value.
- **2 reads sessions through `Reader::sessions` and `Reader::session`
  only.** `ReadRequest` and the read API gain them in step 4, beside the
  control routes.
- **2 adds `session` to the runs the control API lists** (`RunSnapshot`),
  additively, so the interfaces can group by it in step 5.
- **1a kept the terminal app's `Focus::Session` and `IntoSession`**: they
  name the detail pane, which shows the operator's sessions after step 5.
