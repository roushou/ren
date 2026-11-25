# Plan: instances

Superseded by [`deployments.md`](deployments.md), which folds these
instances into deployments of commits.

Ren can run more of itself than production: the canary, a preview of each
pull request, a preview of a commit asked for, and the dev slot. Each is a
Ren daemon and UI, on its own data, at its own address. But none of them is
something the operator starts. A preview is what the factory's next timer
pass decides to run; a commit asked for (`ren preview ask`) is a line in
`<data>/preview-requests.json` that waits for that pass; the dev slot is a
separate system of its own (`cargo xtask dev`). They read as side effects.

This plan makes one thing of them: an **instance**, which the operator (or
a policy) starts on demand, of any commit, branch, or checkout, on the data
it asks for, and gets back at once with its state and its address. Each
phase says whether it is built. Record every deviation here, with its
reason.

The principle: **an instance is started, not wished for.** Asking for one
returns it, in a state that says what it is doing; nothing waits for a
timer.

## What exists

| Instance   | Code                                                   | Started by              | Built by                      | Data              | Address              |
| ---------- | ------------------------------------------------------ | ----------------------- | ----------------------------- | ----------------- | -------------------- |
| Production | `install-vps`, `ren.service`                           | promotion               | the canary's kept build       | the real store    | `:8443`, `:8444`     |
| Canary     | `xtask/src/canary`, `replica.rs`, `pod.rs`             | its timer               | xtask, on the host (`builds`) | a backup's copy   | not published        |
| Previews   | `xtask/src/canary/previews.rs`, `ren_gateway::preview` | the canary's timer pass | xtask, on the host            | a backup's copy   | a fixed slot's ports |
| Dev slot   | `xtask/src/dev` (`slot.rs`, the fake provider, seeds)  | `cargo xtask dev up`    | cargo, from the checkout      | a seeded scenario | `:9443`, `:9444`     |

What already points the right way, and is kept:

- `ren_gateway::preview::Previews`: a pure, persisted state machine
  (`want(wanted, capacity) -> (Previews, Vec<Action>)`) with explicit
  states, including failed with why.
- `builds/<commit>`: a commit is built once and the build kept; promotion
  installs that same build.
- `Replica` and `Pod`: an instance runs as one rootless Podman pod, its
  daemon and UI each a container that sees only its build and its data.
- The self-build plan already says where this goes (phase 7, "one kind of
  instance"; phase 9, "a commit preview is the primitive"). This plan is
  how to get there.

## What stands in the way

- **The daemon cannot build.** Production's daemon runs in a container
  with no toolchain, so only xtask on the host builds, and only from a
  timer. That is why asking is a file and starting is a pass.
- **Builds run on the host, as the operator.** Building a pull request's
  head runs its build scripts (`build.rs`, `bun install`'s scripts) with
  the operator's home, keys, and network. For Ren's own pull requests that
  is the operator's code; for a project's, or a contributor's, it is not.
- **Fixed slots.** A preview's ports, pod, and tailnet route come from its
  slot number; the slots are as many as the policy allows, and a slot is
  the only handle.
- **Three code paths** pick their own ports, names, data folders, and
  routes (the self-build plan's phase 7 says so too).

## Decisions

- **One spec, as data.** An instance is described by:
  - **source:** a commit; a branch, followed (its tip, superseded as it
    moves); a pull request, followed until it merges or closes; or a
    checkout on this host, as it is (uncommitted changes included);
  - **data:** empty; a scenario (a named seed, today's `dev seed`); or a
    replica (a copy of production's latest backup);
  - **upstream:** the real providers, every operator key replaced by a
    placeholder as replicas do now; or the fake provider;
  - **lifetime:** until stopped; while it follows (a pull request open, a
    branch existing); or for a duration;
  - its **project**: Ren, or a registered project, whose factory says how
    it is built.
- **One state machine.** `Previews` becomes `Instances`: requested,
  building, starting, running, superseded, stopping, stopped, and failed
  with why; a build or a start whose outcome is unknown (the daemon
  restarted mid-way) is its own state, never rounded up to running.
  Transitions stay pure, `(Instances, Event) -> (Instances, Vec<Action>)`,
  tested without I/O. It stays where previews are, a module of
  `ren-gateway` renamed `instance`; the daemon's shell carries out the
  actions.
- **The daemon starts instances, through Podman.** It already reaches
  Podman's API socket for runs. A build is a container: the source mounted
  read-only (a fetched commit's worktree, or the checkout), the toolchains
  read-only, the shared build cache read-write, the build output
  read-write, no home, no keys, and the network a relayed run has (the
  registries its project lists). Its result lands in
  `builds/<project>/<commit>` (a checkout's build is keyed by its tree, not
  kept past its instance). Then the instance's pod starts from it.
- **Ren builds itself by its own manifest.** Ren gets a
  `.ren/project.toml`, as any project has, declaring its build (the release
  binary, the UI bundle, and the fake provider's binary for scenarios), so
  Ren is built like any project and xtask's `builds` goes. Today Ren has
  none: its build is xtask's (`Factory::Ren`).
- **Asking returns the instance.** `POST /control/v1/instances` (spec in,
  instance out: its id, state, and address), `GET /api/instances`, `DELETE
  /control/v1/instances/{id}`, and the same as `ren instance start | list |
  stop | logs`. The UI's project page lists a project's instances and
  starts one from a commit, a branch, or a pull request.
- **Addresses from a published range.** The install publishes a range of
  tailnet ports once (Tailscale Serve, as now); an instance takes a free
  pair from it and keeps it while it lives. The daemon never runs
  `tailscale` itself. An instance's address is part of its state.
- **Policies are followers, not a pass.** A pull request preview is an
  instance whose source follows the pull request; the policy (in
  `release.toml`: how many run, which is evicted first) asks for and stops
  them as the GitHub watch sees pull requests open, move, and close. The
  canary is the instance that follows `main` on a replica, with its checks
  and promotion on top.
- **The dev slot is an instance of a checkout.** Source: this checkout;
  data: a scenario; upstream: the fake provider, a container in its pod.
  `cargo xtask dev up` becomes `ren instance start --checkout . --scenario
  <name> --fake`; the slot's own units, ports, and folders go.
- **No agents in instances, in this plan.** Instances keep today's rule:
  no runs. Letting an instance drive agents needs relayed runs in
  production first (self-build plan, phase 12).

## Phases

1. **`Instances` in place of `Previews`.** Built. `ren-gateway`'s
   `preview` module is `instance`: `Instances`, each `Instance` of a
   `Source` (a pull request followed, or a commit asked for, `CommitAsks`)
   in its `Slot`, `Building`, `Running`, or `Failed` (`InstanceState`), and
   one pure step, `Instances::step`, from an `Event` (what a pass wants, or
   the factory's report that a start ended) to the instances and the
   `Action`s that follow; a report that does not fit is refused
   (`StaleReport`) and changes nothing. The factory's pass drives it
   through `step`. Verified: the state machine's tests (now through
   `step`, a late or doubled report refused) and the file's pinned shape;
   the compatibility gate found no change to labels, fixtures, or the API;
   the daemon's contract test, which pins `/api/projects/ren/previews`
   from a file the old code wrote, passed unchanged; and the dev slot,
   built from this change, served a `previews.json` in the shape
   production's older build wrote as it did.
2. **Builds in containers.** Built, for Ren's builds. `Builds::make`, which
   the canary and the previews pass both build through, runs none of a
   commit's code on the host: it fetches the commit's dependencies by its
   lockfiles (`cargo fetch --locked`, `bun install --frozen-lockfile
   --ignore-scripts`) into the builds' own caches
   (`~/.local/share/ren/build-cache`), then runs each step that runs the
   commit's code (`cargo build --offline`, the binary's `--version`, the
   UI's `bun run build`) in a container (`xtask`'s `BuildJob`, `ren-build`)
   with no network, no new privileges, and a home of its own in the
   checkout's `target`, seeing the checkout and the cargo cache
   read-write, and read-only `/usr`, `/etc/alternatives`, the toolchains of
   the programs that build (found from `cargo`, Bun, and Node as the host
   resolves them), and the repository's git folder (from which the build
   names its commit). The UI is staged on the host as before, its
   production dependencies installed with no scripts. Verified, by two
   tests run by hand: a step that lists the home finds only the folders
   above its mounts, and cannot read `~/.ssh` or `~/.gitconfig`, has no SSH
   agent, resolves no name, and reaches no address; and `HEAD` was built
   end to end into a store of its own, its binary naming its commit and its
   UI built, in about two minutes of compiling once fetched.
3. **Started on demand.** Built, through a factory service on the host.
   `cargo xtask canary serve` listens on a Unix socket
   (`$XDG_RUNTIME_DIR/ren-factory/factory.sock`; `canary enable` runs it as
   `ren-factory.service`) and runs a previews pass for each ask, one at a
   time, the asks that arrive during one taken by the next. Production's
   daemon names it (`[factory] socket`, its folder mounted into the
   daemon's container), and the gateway hands it to the control API as its
   `Factory` (`ren-gateway`'s `factory` module, the one-line protocol; the
   daemon's `FactorySocket`). `POST /control/v1/previews/ask` and
   `/withdraw` record the change, ask the factory to plan it, and answer
   once the pass has written its plan (the commit holding its slot,
   building, or taken back), or after five seconds while another pass
   runs. The timer's pass hands its previews pass to the service when one
   listens. A replica's configuration drops `[factory]`: a preview never
   asks production's factory, and an older build reads its configuration.
   The builds' store is locked, so the service and the canary's own build
   never write it at once. Verified in the dev slot, with no timer: an ask
   for a built commit answered in under three seconds with its slot
   building, and the preview then answered `/health` as that commit, a
   preview; a withdrawal answered at once and its pod was gone when the
   pass ended; a withdrawal then a new ask retries a commit that failed.
4. **Followers.** Built. A pull request wants a preview of its head unless
   it is a draft (`Wanted::pull`, the one rule the factory's pass and the
   daemon share). The daemon's GitHub watch tells a `PullFollower` (pure,
   in `ren-gateway`'s `instance`) each repository it sees; when what its
   pull requests want moved (one opened, closed, became ready or a draft,
   or its head moved), the daemon asks the factory for a pass, one at a
   time, a move seen meanwhile waiting for the next. A read that failed
   moves nothing. Verified in the dev slot with no timer, its fake GitHub's
   `acme/ren` pull requests written by hand (`fake-pulls.toml`): a pull
   request opened at a built commit was planned in 12 seconds and ran in
   16; its head moved to another built commit was running that commit in
   25 (its `/health` said so); closed, its preview's pod was gone in 22.
   The poll is 30 seconds.
5. **The dev slot, folded in.** Built, as dev instances started by xtask
   on the host. The slot's fixed places became a `DevIndex`'s (`dev::index`:
   loopback ports in steps of ten, tailnet ports in steps of a hundred,
   folders, unit and container names), index 0 keeping every one of the
   slot's, so that step changed nothing (same configuration, same units).
   Each checkout then claims an index (`dev::claims`, pure, kept in
   `~/.local/state/ren-dev-claims.json` under a lock): `dev up` claims the
   lowest free one, every other `dev` command acts on the checkout's own,
   a gone checkout's claim is let go, and `dev down --release` gives one
   up. An instance's data is a scenario (`dev up --scenario`, played once
   into data never seeded), its upstream the fake provider, its source the
   checkout. Verified with a second worktree beside this checkout's
   instance 0, as an agent would use it, with no `--instance`: `dev up
   --ui` claimed instance 1 (daemon on 18797, UI on 13010, units
   `ren-dev-1-*`, its daemon contained under gVisor); `dev seed`, `dev
   traffic burst`, and `dev shot` reached it, its pages showing its own
   data; a fake Codex run started in its run root asked to run the tests,
   was approved, and finished its turn; `dev down --release` stopped every
   unit and container of it and gave up the claim, while instance 0
   answered throughout. Instance 0's configuration and units were compared
   before and after the first step: unchanged. A fresh instance had no
   scratch folder for its contained daemon to mount; it is made now.
6. **The canary, as an instance.** Built, as the follower of `main`. The
   GitHub watch's `Follower` (now in `ren-gateway`'s `factory` module,
   beside the protocol it speaks) asks for a canary pass, per project
   watched on the repository, when its default branch's head moves from
   one seen before; never on a daemon's first sighting, so a promotion's
   restart starts none. The factory service takes `canary <project>` for
   the project it builds (another's is `not_built`), `promote` (the
   Release page's request, which the control API now asks at once), and
   `previews`, one pass at a time in that order; while it listens, the
   timer's pass hands it the canary, and a lock keeps two canary passes
   from overlapping. The canary's checks, release, and promotion are as
   they were, on top. Verified in the dev rehearsal (`--of dev`): the fake
   GitHub's `acme/ren` `main` moved (`fake-github.toml`), and the service
   started a canary pass 12 seconds later: it fetched `main`, checked its
   tip out, and handed on to that commit's own xtask, which, predating
   `[factory]`, refused the dev configuration (see the deviations). A
   whole pass of this commit (`--source checkout`) then built it in its
   container, started the canary on a copy of the dev history, and passed
   doctor, reads, and pages; the Release page's promotion request for it
   was answered `202` in 13 ms, and the service took it up at once (and
   dropped it, as the dev rehearsal is never promoted into). A promotion
   of production through it waits for the operator.

Phases 1 and 2 change nothing the operator sees; 3 is the first that
does. Each lands on its own and passes every check.

## Open questions

- **Memory.** Each instance is a daemon, a UI, and a copy of production's
  data. How many can run at once, and whether that is a count or a memory
  budget the policy enforces.
- **The build cache.** Sharing one cargo target folder across commits
  makes builds fast but lets one build's output reach another's; one per
  project, or per instance, and how it is cleaned.
- **A checkout's build.** Building uncommitted changes in a container
  means mounting the checkout read-only and building elsewhere; whether
  incremental builds stay fast enough for UI work.
- **Who may start one.** The operator, through the control API; whether a
  working agent may start an instance of its own branch, and with which
  data (a replica holds production's history).
- **Projects' instances.** A project's preview today is `ren project
  preview` in a worktree (a transient unit, not a pod). Whether projects'
  instances run as pods like Ren's, from the same build jobs.

## Deviations

- **Phase 1: the file and the API keep their shape.** The factory (the
  canary's build of `main`) writes `previews.json` and production's
  gateway (an older, promoted build) reads it, so its keys stay the ones
  previews had (`previews`, `key`), and `/api/projects/{name}/previews`
  and its wire types are unchanged. They change with phase 3, when the
  daemon writes the file it reads.
- **Phase 1: no unknown-outcome label.** A persisted new state would break
  the older reader. `Building`, as a later pass reads it, already is that
  state: the pass that was building it stopped before saying how it ended,
  and it is started again, never taken for running. The type says so.
- **Phase 1: the spec's other parts wait for a second value.** Data (a
  replica), upstream (placeholders), lifetime (followed, or until
  withdrawn), and project (Ren) each have one value today, implied by the
  `Source`; a field that can hold only one value decides nothing. Each
  becomes a field in the phase that gives it a second: data and upstream
  with phase 5's scenarios, lifetime and project with phase 3's on-demand
  instances.
- **Phase 2: fetched first, built offline, not relayed.** The plan gave a
  build a relayed run's network; the factory runs in xtask, with no relay.
  Instead the host downloads by the lockfiles, which runs none of the
  commit's code, and the build, where its code runs, has no network at
  all: stronger, with nothing more to run.
- **Phase 2: Ren's build stays xtask's.** Manifests have no build outputs
  (the binary and the UI the store keeps), so Ren's `.ren/project.toml`
  waits until projects' instances share these build jobs, and gains
  outputs then.
- **Phase 2: what a build still sees.** The repository's git folder (its
  history and configuration) and `~/.bun` (Bun's global installs and
  cache) are read-only in a build; with no network, nothing it reads
  leaves but through what it builds.
- **Phase 1: the factory's commands keep their names.** `cargo xtask
  canary previews` and `ren preview` stay as they are until phases 3 and
  4 replace them.
- **Phase 3: the factory starts them, not the daemon.** The plan had the
  daemon run the actions through Podman. That would give the daemon, which
  serves the network, the host's container engine and the builds' caches;
  instead it gets one socket, on which it can ask for a pass and nothing
  more, and the factory on the host keeps the Podman, the builds, and the
  backups it already ran. The file and the API therefore keep phase 1's
  shape (the factory still writes `previews.json`), and `ren preview` and
  the endpoints keep their names; `ren instance`, published addresses, and
  the UI's start wait for the instances' own API.
- **Phase 3: no lifetime or project field yet.** On-demand instances are
  still the commits asked for, until withdrawn, of Ren: each part still has
  one value.
- **Phase 4: `ren preview` and `preview-requests.json` stay.** They went
  in the plan because `ren instance` replaced them; with phase 3's
  factory, the asks are still how a commit is started on demand, and they
  go when the instances' own API comes.
- **Phase 4: every watched repository is followed.** The daemon does not
  know which project the factory previews (`canary serve --project`), so
  a move in any watched repository asks for a pass; one in another
  project's is a pass that changes nothing. A pull request's place in the
  list counts too: it is the order slots are given in.
- **Phase 4: the timer stays, to catch up.** A move seen while the factory
  is not listening is taken up by the timer's next pass.
- **Phase 5: built and started by xtask, on the host.** The operator
  chose it: a checkout is the operator's own work, and its edit-and-see
  loop needs the incremental build cache a container build would not
  keep. Commits and pull requests stay the factory's, built in containers.
- **Phase 5: units, not pods; `cargo xtask dev` stays.** An instance's
  processes stay transient units, so an instance can still run its daemon
  and runs on the host (`--contain none`); the command stays because it
  is what starts one, now per checkout, not a separate system beside
  instances. `ren instance` waits for the instances' own API.
- **Phase 5: the canary rehearses against instance 0** (`--of dev`), and
  the dev factory socket is instance 0's: one rehearsal is enough.
- **Phase 6: the canary is followed, not modeled as an `Instance`.** Its
  state stays `release.json` (a build, its checks, the policy's decision)
  rather than a slot in `previews.json`: production's older gateway reads
  that file's shape, and the canary has no slot. What made it an instance
  is that it follows a source (`main`) on a replica and starts when its
  source moves.
- **Phase 6: `[factory]` waits for the builds that read it.** A canary pass
  checks `main` against production's configuration, so the section goes
  into production's only after the promotion that ships it; the dev
  rehearsal of a `main` that predates it refuses the dev configuration.
