# TGV contributor guidance

TGV is a personal LLM gateway for Claude Code, Codex, and Pi. It forwards native
provider traffic while recording, inspecting, and exposing it, using the user's own
Anthropic and OpenAI subscriptions. One executable and one storage format serve both
a laptop and a VPS reached through Tailscale.

[`docs/architecture.md`](docs/architecture.md) describes how TGV is built and
lists its known limitations. Section 4 below maps the seven crates.

- All code follows the principles (section 2) and the workspace lints.
- Never describe unbuilt functionality as working. Read the source before assuming
  how something behaves.

---

## 1. Product invariants

These describe what TGV must do. They hold through every refactor.

### Forwarding

- Forward native payloads byte for byte. Never reconstruct traffic through a universal
  message schema. Inspection is a separate, read-only observer: observing traffic must
  never change it.
- Apply the required HTTP proxy header rules (hop-by-hop headers, proxy identity
  headers). Preserving payloads does not mean forwarding every header blindly.
- Any deliberate transformation records the original payload, the resulting payload,
  and the reason, according to capture policy.
- Preserve stream ordering, backpressure, cancellation, trailers, and terminal
  events. Network chunks are arbitrary byte fragments, not protocol event boundaries.
- Track upstream acceptance and downstream delivery separately. Never retry inference
  blindly after ambiguous acceptance or partial delivery. Each retry is a distinct
  attempt. No generic retry middleware around inference.
- Every request uses one immutable configuration snapshot. Record the configuration
  revision and routing decision that request used.

### Lifecycle truthfulness

- Response headers are not stream completion. HTTP completion is not model
  completion. Cancellation requested is not cancellation observed.
- Preserve interrupted and unknown outcomes. Never round them up to success.
- Missing telemetry is not evidence of idleness or completion. Token usage,
  subscription allowance, and monetary cost are different measurements.
- Distinguish wire captures, client-exported data, inferred states, and estimates.

### Capture and storage

- Recording never stalls forwarding. Database writes, parsing, and UI consumers stay
  off the forwarding path. Queues are bounded by bytes and by item count, and overload
  behavior is defined.
- Report dropped or incomplete capture through a status path that does not depend on
  the saturated capture queue.
- Store versioned observations with provenance, timestamps, correlation IDs, ordering,
  and completeness. Build query views from retained events. Don't event-source the
  whole application without a concrete need.
- One active daemon owns its storage on its own host. Remote clients use the API,
  never shared SQLite files. Account for migrations, retention, payload/index
  consistency, and recoverable shutdown. On shutdown, stop HTTP before draining capture.
- A remote daemon cannot read a laptop's session files. Collecting client telemetry
  and forwarding traffic are separate integration responsibilities.

### Security and authentication

- Keep four concepts separate: client, wire protocol, upstream provider, and
  authentication. Gateway credentials and provider credentials are separate.
- Clients own subscription sign-in and token refresh. If TGV ever owns refresh,
  serialize refreshes and define ownership explicitly.
- Never record credential headers or token-bearing URLs. Never expose secrets through
  `Debug`, errors, fixtures, logs, or exports. Sanitize before anything is persisted.
- Never spoof client identity. OAuth availability does not prove that every client is
  entitled to a subscription. Check current official provider documentation before
  changing authentication or gateway behavior.
- Listeners default to loopback. Remote access and management controls are explicit,
  with Tailscale as the preferred private path.

---

## 2. Design principles

TGV follows the shared design principles in `~/dev/principles/design.md` (type-driven
design with a sans-IO core). Read that file before making design decisions. Where
it's unavailable, the TGV-specific rules below still apply.

TGV-specific applications:

- **Identities:** channels, calls, attempts, sessions, turns, and tool calls each get
  their own type (`tgv_recorder::id`). Provider-native identifiers keep separate types
  (`ClaudeSessionId` is not `SessionId`).
- **Vocabularies:** a closed set of labels is an enum deriving `strum`'s
  `Display`, `EnumString`, `IntoStaticStr`, and `VariantArray` with
  `serialize_all = "snake_case"`, plus `serde_with`'s `SerializeDisplay` and
  `DeserializeFromStr`, so every representation reads the labels from the strum
  attributes (`tgv_protocol::label` lists the derives). An irregular label is a
  `#[strum(to_string = "…")]` on its variant. Never add `#[serde(rename…)]`, which
  would be a second table. Every label is pinned in its crate's `tests/fixtures/labels.txt`
  (`tests/labels.rs`); add a new vocabulary there, and treat a changed line as a
  storage format change. Labels read from storage or the wire use `Stored<T>`,
  which keeps unknown text.
- **Keep the unknown:** wire enums for Anthropic Messages, OpenAI Responses, and SSE
  events keep an `Unknown(Raw…)` variant. `#[serde(other)]` loses the value, so don't
  use it on wire data.
- **Sans-IO core:** SSE framing, native protocol parsing, inspection, attempt
  lifecycle transitions, retry eligibility (`tgv_gateway::retry`), operator
  state (`tgv_gateway::operator`), call planning and transforms (`policy`), retention decisions,
  and history query building. Framing
  stays independent of JSON and of providers.
- **Shell:** Axum handlers, the Reqwest transport, SQLx persistence, the clap CLI, and
  daemon wiring. Recording, inspection, and storage stay independent of CLI argument
  parsing and Axum handlers.
- **Capabilities:** components receive `Clock` (`tgv_recorder::time`) and
  `IdSource` (`tgv_recorder::id`) instead of reading the host clock or generating random IDs. The
  gateway receives an `Upstream` (`tgv_gateway::upstream`) and the shared
  `Controls` (`tgv_gateway::control`). Environment variables (operator API keys) are read once into
  `HostEnv` (`tgv_harness::host`), captured once in `main` and carried by the CLI
  `Context`;
  configuration, the Pi cache, launch, and doctor receive it. The recorder writes
  through its `RecordSink` port (`tgv_recorder::RecordSink`), which `Store` implements.
- **Lifecycles** are pure state machines, one per transport (`HttpAttempt` in
  `tgv_gateway::attempt`, and `WsChannel` in `tgv_gateway::channel`, which owns
  its calls' states), with explicit terminals that include interrupted and unknown outcomes.

### Lints

The strict lints are workspace-wide, in root `Cargo.toml` `[workspace.lints.clippy]`.
They deny, among others, wildcard enum arms and imports, `unwrap`/`expect`/`panic!`
outside tests, `as` conversions, excessive bools, and the ambient calls listed in
`clippy.toml` (host clocks, random UUIDs, environment variables, task spawning).
Modules carry no lint headers of their own. A module with submodules lives in
`name/mod.rs`, never `name.rs` beside a `name/` directory
(`self_named_module_files`).

- The only exemptions are the capability implementations themselves, marked with
  `#[expect(clippy::disallowed_methods, reason = "…")]`: `HostEnv::capture`, the
  daemon's `SystemClock` and `RandomIds` (`tgv_daemon::host`), and the daemon's two
  task spawns.
- Integration-test crates (`crates/*/tests/*.rs`) act as a shell: each has one
  crate-level `#![allow(...)]` with a reason, covering helpers outside `#[test]`
  functions and spawned mock servers. Unit-test modules use `clippy.toml`'s
  `allow-*-in-tests` instead.
- `anyhow` is used only in `tgv-cli`, the command shell (`main.rs`,
  `context.rs`, `commands`, and `Cli::run`). Every other module returns its own
  `thiserror` enum, including `doctor` and `service` in `tgv-cli`.

## 3. Testing

Follow the testing section of the shared principles. Priority scenarios for TGV:
fragmented streams, partial failures, retry decisions, cancellation, capture
saturation, and persistence recovery.

- End-to-end gateway tests use `TestGateway` (`crates/tgv-daemon/tests/support`): the
  gateway, recorder, store, and read and control APIs with a fixed clock and sequential IDs,
  forwarding to a loopback mock upstream or a scripted `Upstream`. Full-daemon
  tests (`tests/daemon.rs`) are only for wiring, lifecycle, and shutdown.
- Automated checks never need personal credentials or consume subscription allowance.

---

## 4. Workspace map

This map describes the code as it is: the seven crates, listed with their
dependencies under "Crates" below. Read the code for current module ownership.

- `docs/architecture.md`: how TGV is built, and its known limitations. The other
  `docs/*.md` files are operator guides linked from `README.md`.
- `crates/tgv-protocol`: sans-IO native protocol understanding, with
  `CallInspector` as its primary type: SSE framing, Anthropic Messages and OpenAI
  Responses wire vocabularies, incremental decompression, per-call inspection
  (`inspect`, whose `pipeline` also reads WebSocket messages), and WebSocket
  message classification.
  It also owns the label machinery (`label`, with `Stored<T>`), the `Provider`,
  `Endpoint`, `Method`, and `Transport` vocabularies (`route`), `Direction`, and
  the native identifiers and message hints the wire carries (`native`). Its
  manifest allows only pure dependencies, and a test keeps its sources free of
  I/O.
- `crates/tgv-harness`: the harnesses, with `Harness` (`ClaudeCode`, `Codex`, `Pi`)
  as its primary type: attribution (`attribution`, with its private `headers`
  parser implementing `Harness::attribution`), session launch plans
  (`Harness::launch`), the bundled Pi extension cache (`pi`), the gateway URL a
  harness is pointed at (`endpoint`), and `HostEnv` (`host`), the host environment
  that launching and configuration read. It performs filesystem and process I/O
  but has no async runtime, HTTP, database, or CLI dependency.
- `crates/tgv-recorder`: the bounded capture queue between forwarding and
  storage, with `Recorder` as its primary type. `CallTap` (and `ChannelTap` for a
  WebSocket's end) is the forwarding side; `RecorderTask` (spawned by the daemon)
  runs inspection, group commits, retention, and the live broadcast, writing
  through the `RecordSink` port. Omissions are counted by reason in
  `RecorderMetrics`, readable after shutdown through `RecorderMeter`. It also
  owns what is recorded: TGV's identities and `IdSource` (`id`), `Clock`
  (`time`), terminals and other recorded facts (`capture`), routing facts and
  `OperatorRevision` (`routing`), batches (`batch`), the content-defined
  `Chunker` (`chunk`), the stored inspection format (`inspection`), and
  `RetentionPolicy` (`retention`). It has no database, HTTP, or CLI dependency.
- `crates/tgv-store`: persistence (SQLx, SQLite WAL), with `Store`, the single
  writer, as its primary type; `Reader` is its read-only pool and `Backup` the
  online backup, which holds the `SweepGate`. `tgv.sqlite3` holds the
  observation log and query tables; `blobs.sqlite3` holds deduplicated,
  content-defined body chunks, in the format `chunk` defines (hash, codec,
  encoding). It contains typed query parameters and read views and implements
  the recorder's `RecordSink`. It owns the versioned migrations for
  `tgv.sqlite3` (`crates/tgv-store/migrations`); never edit one that has
  shipped. Its tests include `tests/recorder.rs`, the recorder against the real
  store.
- `crates/tgv-gateway`: TGV's HTTP interface, with `Gateway` as its primary type.
  It owns forwarding, management, and control routes on one listener, assembled
  by `Gateway::router` or `Gateway::into_ingress`. Forwarding uses a declarative
  route table, the header policy, request buffering with a global
  byte budget, the typed upstream entries with their credential override, the
  candidate loop, the relay that drives the pure `HttpAttempt` machine
  (`attempt`), the WebSocket relay that drives `WsChannel` (`channel`), the
  `Upstream` seam (HTTP send and WebSocket connect) with its Reqwest
  implementation, and the ingress with the shutdown drain, which also waits for
  upgraded channels. Use Axum's request, response, body, and WebSocket types
  directly; no framework-independent HTTP adapter layer. It also owns retry
  eligibility (`retry`), operator state (`operator`), the `Controls` that hold
  it with the running-call registry (`control`), and pure call planning
  (`policy`: block, transform, and fallback rules validated at load, with
  protected headers and body paths). `src/purity.rs` keeps `attempt`, `channel`,
  `retry`, and `operator` free of runtime, I/O, storage, and shell-module
  dependencies. Management receives `Reader`, `Backup`, retention settings, and
  control credentials through `ManagementSetup`; SQL stays in `tgv-store`, and
  forwarding records only through the recorder.
  - `src/api`: the versioned read-only management API (`ApiState`), body previews,
    and the `/api/live` SSE feed. Reads only through `Reader`.
  - `src/api/control.rs`: the mutating `/control/v1` API over the gateway's
    `Controls` (token or Tailscale identity; browsers refused), and its
    `[control]` configuration (`ControlOptions`) and `ControlToken`. Handler state
    stays crate-private; configuration and token types are exported at the root.
- `crates/tgv-daemon`: configuration, resource wiring, and lifecycle, with
  `Daemon` as its primary type (`lib.rs`). It opens storage, binds the listener,
  writes the control token, constructs the gateway and recorder, owns the two
  task spawns, and coordinates HTTP shutdown before capture drain. It defines no
  HTTP routes or handlers and has no production Axum dependency. `config` turns
  `Config` into a `RuntimeConfig`. `host` holds `SystemClock` and `RandomIds`.
  Its integration tests hold `TestGateway` (`tests/support`) and the gateway,
  WebSocket, decision, and full-daemon tests; they use the gateway's complete
  router with test capabilities.
- `crates/tgv-cli`: the `tgv` binary, with `Cli` as its primary type
  (`lib.rs`) and a thin `main.rs` that sets up logging and captures `HostEnv`.
  `commands` holds clap parsing and dispatch: each command is a
  `#[derive(Args)]` struct with its own execution method, and command groups
  are `#[derive(Subcommand)]` enums. `doctor` and `service` are the doctor
  report and the service definitions. Refactors preserve existing flags, output
  formats, exports, and client passthrough. Its integration tests run the built
  binary (`CARGO_BIN_EXE_tgv`).
- `ui`: an independent SvelteKit, Svelte 5, and TypeScript server (Node adapter). It
  reads the daemon only through the management API and forwards only allowlisted
  read-only endpoints. It never forwards browser cookies or credentials, never reads
  storage directly, and never proxies inference.
- `integrations/pi`: the embedded Pi extension and its dependency-free contract tests.
  Pi owns provider authentication, refresh, payload conversion, and retries.
- `deploy`: systemd units. `scripts`: manual smoke tests for Pi, systemd, and launchd.

### Crates

"Depends on" names the nearest crates below; a crate may also use lower ones
directly.

| Crate                    | Primary type                            | Depends on               |
| ------------------------ | --------------------------------------- | ------------------------ |
| `tgv-protocol`           | `CallInspector`                         | none of ours             |
| `tgv-harness`            | `Harness` (`ClaudeCode`, `Codex`, `Pi`) | protocol                 |
| `tgv-recorder`           | `Recorder`, with its `RecordSink` port  | protocol, harness        |
| `tgv-store`              | `Store`, which implements `RecordSink`  | recorder                 |
| `tgv-gateway`            | `Gateway`                               | harness, recorder, store |
| `tgv-daemon`             | `Daemon`                                | gateway, store           |
| `tgv-cli` (binary `tgv`) | `Cli`                                   | daemon                   |

`docs/architecture.md` lists what each crate owns. These rules apply to every
crate:

- **One primary type.** Each library crate exposes one primary type that names
  what it is. Its other public items are that type's inputs, outputs, errors, or
  handles. If a crate has no natural primary type, rethink the crate; never add a
  wrapper or namespace type to satisfy this rule.
- **Names.** Name a crate for its role or for what it describes. Never a verb, a
  category, or a grab bag (`core`, `common`, `model`, `types`, `utils`).
- **No types crate.** A type lives in the lowest crate that owns its invariant.
  Never move a type to a shared crate to break a dependency; find its owner.
- **Ports point down.** A capability trait is declared by the lowest crate that
  consumes it and implemented by a higher one. A crate never depends on a crate
  above it in the table.
- **Harnesses.** Claude Code, Codex, and Pi (the clients of section 1) are one
  closed enum, `Harness`. Don't add a per-harness trait.
- Don't add a crate without first adding it to this table and to
  `docs/architecture.md`.

### Stack and dependencies

Tokio, Axum/Tower, Reqwest/Rustls, SQLx with SQLite WAL, and `tracing`. Use REST and
SSE for management and live observations, and WebSockets only where an upstream
protocol requires them. Add dependencies only when functionality needs them. A
dependency used by one crate belongs in that crate's manifest (test-only usage
included). Dependencies shared by several crates, and all of our own crates, are
declared in root `[workspace.dependencies]` and inherited. Keep `Cargo.lock`
tracked and update it intentionally.

### UI and Pi (TypeScript/JavaScript)

The same principles apply in TypeScript:

- Parse API responses and environment variables at the boundary.
  `ui/src/env.ts` validates gateway origins, keeps loopback defaults, and keeps the
  server's daemon address separate from the client-facing address.
- Use discriminated unions for states instead of loose flags.
- Keep pure logic in `src/lib` modules that can be tested without a browser.

Use filesystem routes and typed load functions. Private HTML, route data, and API
responses are non-cacheable. SvelteKit generates CSP nonces, including for the
inline theme initializer. Never commit generated assets (`ui/build`, `.svelte-kit`).

---

## 5. Build and verification

Use the toolchain pinned in `rust-toolchain.toml` and don't update it incidentally.
New crates inherit workspace settings and lints. Cargo builds only Rust: never run
frontend tooling from Cargo or embed UI assets. The daemon is headless and works
with no UI checkout, build, or server.

Markdown, JSON, and TOML formatting, from the repository root (dprint 0.56.0,
matching CI):

```sh
dprint check
```

Run `dprint fmt` to apply the formatting in `dprint.json`.

Rust changes, from the repository root:

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

UI changes, in `ui` (Bun from `package.json`, Node 22.17+):

```sh
bun install --frozen-lockfile
bun run format:check && bun run check && bun run test && bun run build
```

Verify changed UI interactions manually in a browser with synthetic traffic. Keep
focused unit tests and don't add a browser integration suite yet. UI changes don't
require rebuilding Rust.

Pi integration changes: `node --test integrations/pi/tgv.test.mjs`. When Pi is
installed, build TGV and run `python3 scripts/smoke-pi.py`; it uses isolated config
and OAuth-shaped fixture credentials against local mocks. Passing mock tests does
not establish live subscription access.

Service changes: `python3 scripts/smoke-systemd.py --binary <built-tgv>` as a
non-root user with a systemd user manager (its `--https-port` option needs an unused
Tailscale Serve port; preserve existing Serve routes and policy), or
`python3 scripts/smoke-launchd.py` on macOS (tests `target/debug/tgv`).

Documentation-only changes don't require compiling; check the diff and that every
referenced path exists.

---

## 6. Working conventions

- Keep changes focused. Refactor in steps that each pass verification, and don't
  mix a behavior change with a structural move.
- Preserve unrelated user work.
- Update documentation when behavior or configuration changes. `README.md` and the
  guides in `docs/` are for operators; this file and `docs/architecture.md` are for
  contributors. Documentation describes the current state, not its history.
- Report concisely: what changed, what was verified, and what remains limited or
  unverified.
