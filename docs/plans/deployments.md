# Plan: deployments

Ren runs more of itself than production: the canary, previews, and dev
instances. [`instances.md`](instances.md) made them start on demand, but
they are still three systems, and most of them live in `xtask`, a
developer's task runner run from a checkout through cargo. Production's
builds, canary, promotion, and previews run as `cargo xtask canary serve`
(the "factory"). Dev instances run with a fake world: a fake model
provider, fake agent clients, fake GitHub, and a traffic generator. That
world started as demo scaffolding when Ren was a gateway, and it has since
grown into infrastructure.

This plan makes one thing of them. A **deployment** is a commit, running,
on data it was given. Deployments differ only in what picks their commit
and what seeds their data. Each phase says whether it is built. Record
every deviation here, with its reason.

The principle: **everything is a deployment of a commit.** Nothing is a
slot, and nothing runs from a checkout.

## What exists

| Code                                                     | Lines | What it is                                                                                             |
| -------------------------------------------------------- | ----- | ------------------------------------------------------------------------------------------------------ |
| `xtask/src/canary`, `builds.rs`, `build_job.rs`          | ~2.2k | Builds per commit, the canary's pass, checks, promotion, the factory service                           |
| `replica.rs`, `pod.rs`, `daemon_container.rs`, `unit.rs` | ~1.5k | A build started on a copy of production's data, in a pod; the daemon's container; systemd units        |
| `xtask/src/preview.rs`                                   | ~0.4k | A worktree's preview (self-build phase 5), a third way to run a branch                                 |
| `xtask/src/install`, `daemon.rs`                         | ~2.8k | Production's install and upgrade (drain, swap, check, resume)                                          |
| `xtask/src/dev/agent`, `dev/provider`, `dev/work.rs`     | ~5.3k | The fake world: fake Codex, Claude Code, Pi, and OpenCode; a fake provider for every wire; fake GitHub |
| `xtask/src/dev/traffic`, `ui/.../devtraffic.ts`          | ~1.1k | Live fake traffic, and the dev toolbar that tunes it                                                   |
| `xtask/src/dev/seed`                                     | ~0.9k | Scenarios, played as calls through a gateway to the fake provider                                      |
| `xtask/src/dev/{instance,index,claims,mod}.rs`           | ~1.9k | Dev instances: claimed indexes, ports, units, a hand-built daemon configuration                        |
| `xtask/src/compat`                                       | ~2.0k | The compatibility gate CI runs                                                                         |

`xtask` is about 19k lines. The compatibility gate is what xtask is for.
The rest is production runtime or a simulator. An instance's configuration
is derived three times (`DevInstance`, `Replica`, `DaemonContainer`), with
no one owner.

What points the right way, and is kept:

- `Build`, `Verdict`, `Policy`, `Release` (then `ren-release`, now
  `ren-gateway`'s `release` module).
- The instances' state machine (`ren-gateway`'s `instance`): pure,
  persisted, a stale report refused.
- Builds in containers, once per commit, kept in `builds/<commit>`;
  promotion installs that same build.
- Pods: an instance's daemon and UI, each a container that sees only its
  build and its data.
- The followers: previews follow ready pull requests, and the canary
  follows `main` (`ren-gateway`'s `factory::Follower`, pure).
- The canary's checks: doctor, every read, every page.

## Decisions

- **A deployment is a commit, a trigger, and a seed.**
  - **Trigger:** _chosen_ (a commit the operator picks, kept until
    replaced or stopped), _follows a branch_ (its tip, replaced as it
    moves), or _follows a pull request_ (its head while it is ready, gone
    when it closes).
  - **Seed:** _production's data_ (production itself), _a copy of
    production's latest backup_, or _a scenario_ (invented data, written
    into an empty store).
  - Production is chosen, on production's data. The canary follows `main`
    on a copy, and its checks gate promotion. A preview follows a pull
    request, or is chosen, on a copy or a scenario. There is no dev
    deployment: a dev environment that is always up to date is the canary,
    and a change seen before it merges is a preview of its branch's
    commit.
- **Uncommitted work is not deployed.** To see a change, commit it to its
  branch and deploy that commit. Branches are squashed on merge, so work
  commits cost nothing. The edit-and-see loop stays fast through each
  project's incremental build cache, not through a separate host build.
- **One state machine, `Deployments`.** The instances' `Instances` and the
  canary's place in `release.json` become one pure, persisted machine,
  `(Deployments, Event) -> (Deployments, Vec<Action>)`. Each deployment has
  a trigger, a seed, a commit, and a state: building, starting, running,
  superseded, stopping, stopped, failed with why, or unknown when its
  outcome was lost. An unknown outcome is never rounded up to running. The
  machine lives in `ren-gateway` beside `release`'s `Build` and `Policy`:
  the gateway reads it, and the deployer writes it.
- **The deployer is a component, not glue.** A new crate, `ren-deployer`,
  primary type `Deployer`, owns deploying and does its I/O openly. It builds
  commits in containers, seeds their data, starts their pods, runs the
  canary's checks, and promotes. It ships in the `ren` binary as `ren
  deployer` commands and runs as `ren-deployer.service`, installed and
  upgraded with the build it belongs to, never from a checkout. It replaces
  the factory service, and `[factory]` becomes `[deployer]`. The section
  has never been set in production's configuration, so it gets no alias.
  Layering: `ren-deployer` depends on daemon and container (the gateway's
  `release` and `deployment` types come through the daemon), and `ren-cli`
  depends on it.
- **One owner for an instance's configuration.** The deployer derives a
  deployment's daemon configuration from production's, once: its ports,
  data folder, environment, placeholder keys, and no runs. `Replica`,
  `DevInstance`, and `DaemonContainer`'s separate derivations go.
- **A build checks itself.** The canary hands off to the commit's own
  xtask today so that a commit is checked by its own checks. Instead the
  built `ren` carries its checks (`ren check`: doctor, reads, pages), so
  the deployer runs the commit's own checks without running its checkout.
- **Seeding writes the store, not traffic.** A scenario is invented data
  in a file. The built `ren` writes it into an empty store through the
  recorder and the store (`ren store seed <scenario>`), so the commit's own
  store code writes it and no provider answers anything. Previews on a
  scenario show seeded calls, sessions, and runs, and no live activity.
- **The fake world is deleted.** That means the fake provider, the fake
  agent clients, fake GitHub, the traffic generator, the dev toolbar,
  `Environment::Development`, and the dev badge. Tests that need a double
  keep a small one beside what they test, as `ren-supervisor`'s tests
  already do. No crate's tests use xtask's fakes today: only xtask's own
  canary, replica, and checks import `dev` (the screenshots and an error
  type).
- **Deployments start no runs.** A deployment other than production
  starts no agent runs, as replicas do now. Driving agents in a preview
  needs relayed runs in production first (self-build plan, phase 12).
- **xtask keeps repository tasks.** That means the compatibility gate,
  and bootstrapping a first install from a checkout. That bootstrap builds
  once and hands over to `ren deployer`, which owns every later upgrade.

## Phases

Bottom-up, each passes every check on its own. A change that is more than
a move lands before the move that needs it.

1. **Seeding into the store.** Built. `ren seed --seed <n>` invents a
   history from a number (`ren-cli`'s `seed`, pure, with a purity test)
   and writes it through the recorder into the store, the daemon stopped:
   sessions from Claude Code, Codex, Pi, and opencode, each turn's tool
   calls in its client's own tool names and wire format, named users by
   login and by token, an unlisted login, a rate limit, an upstream
   error, a broken stream, and long prompts, over the last three days.
   The store works sessions, turns, tools, and tokens out of the bytes, as
   it does for real calls. `cargo xtask dev seed --seed <n>` and `dev up
   --seed <n>` run the instance's own `ren seed`, the daemon stopped
   meanwhile. Verified: one seed always invents the same history and
   another a different one (tests); a scratch store seeded in under a
   second read back 100 calls in 15 sessions from all four clients, four
   kinds of caller, and every failure; this checkout's dev instance, reset
   and seeded, rendered Traffic, a session, Usage, and every other page
   (`dev shot`) from that history.
2. **`Deployments`.** Built, as three commits. `ren-release`'s `Revision`
   (a commit and whether tracked files differed) is a git fact the recorder
   and store also use: it is `ren_git::SourceRevision`. The rest of
   `ren-release` (`Build`, `Verdict`, `Policy`, `Release`, `Decision`) is
   read and served by the gateway and used only above it: it is
   `ren-gateway`'s `release` module, and the crate is gone. The compatibility
   gate reads every crate's label table as one, so a vocabulary moving
   between tables is no change. Then `ren-gateway`'s `instance` module is
   `deployment`: `Deployments` of `Deployment`s in their `DeploymentState`,
   each picked by a `Trigger`, `FollowsPull` or `Chosen` (`ChosenCommits`).
   `previews.json` and `preview-requests.json` keep their shape. Verified
   at each commit: the same 1229 tests pass (the release labels test folded
   into the gateway's), lint, and the compatibility gate against the commit
   before reports no change to labels, fixtures, or the API.
3. **`ren-deployer`.** Built. The factory's code first stopped reaching into
   xtask's `dev` (the page camera is its own module; the dev rehearsal is
   found by instance 0's folders), then moved, unchanged, into
   `crates/ren-deployer` (the same 1229 tests, the same help for every
   moved command, no change at the compatibility gate). Then it became
   `ren deployer` in the `ren` binary: `serve`, `canary`, `preview`, and
   `install-vps`, building from `--checkout` (this folder by default), not
   the folder the binary was compiled in; xtask keeps only `compat` and
   `dev`. A canary pass builds `main`'s tip and goes on with that build's
   own `ren deployer`. A build asks rustup which cargo builds the checkout,
   so the commit's toolchain file decides. A rehearsal's pass prunes no
   builds (the folder is shared with production's canary). The service the
   daemon asks is the deployer: `[deployer] socket`
   (`$XDG_RUNTIME_DIR/ren-deployer/deployer.sock`), `ren-deployer.service`
   running the installed `ren deployer serve`, the gateway's `deployer`
   module (`Deployer`, `DeployerRequest`, `DeployerAnswer`), the daemon's
   `DeployerSocket`, and the crate's primary type `Deployer`. Verified: a
   whole canary pass of this branch through `ren deployer canary run
   --source checkout --of dev` built it in a container, started the canary,
   and passed doctor, 19 reads, and every page, with production's builds
   kept; then the dev daemon, configured with `[deployer]`, asked a served
   `ren deployer serve` for a preview of that commit, which ran and
   answered `/health` as that commit, a preview, and was stopped when
   withdrawn.
4. **Chosen previews, seeded.** Built. A deployment has a `Seed`: a copy
   of production's latest backup, or the history a `SeedNumber` invents.
   `ren deploy <commit> [--seed N] [--work ITEM]...` chooses a commit (a
   revision of this folder's repository, `HEAD`, or a full id), `ren deploy
   stop` withdraws it, and `ren deployments` lists each deployment in its
   slot with its seed; they replace `ren preview`. A seeded deployment
   starts on an empty store its own build writes with `ren seed`
   (`Replica::seed`). Once it runs, its address is recorded on each work
   item it was chosen for, and taken off when no deployment previews the
   item any longer. A commit the repository already holds (an agent's
   unpushed one, in a worktree) is not fetched. The agents' brief says to
   commit and run `ren deploy HEAD --work <id>`; the worktree preview
   (`ren deployer preview`, its pod, and the daemon's removal of it) is
   gone. Verified: the state machine's tests (another seed starts again,
   other work items are only recorded, a copy is left out of the file),
   the chosen file's tolerant read of bare commits, and in the dev
   rehearsal: `ren deploy <commit> --seed 7 --work <item>` through the dev
   daemon and a served deployer started the build in a slot, seeded 100
   calls (served by its read API), answered `/health` as that commit, a
   preview, and recorded its address on the sent work item; `ren deploy
   stop` stopped it and took the record off.
5. **The dev world goes.** Built. `xtask/src/dev` is deleted: dev
   instances, their claims and index, the fake provider, the fake agents,
   fake GitHub, the traffic generator, and the scenario player, with their
   49 tests; xtask keeps `compat` and depends only on `ren-git`. The UI's
   orphaned `devtraffic.ts` goes (its toolbar went with the redesign), and
   so do `Environment::Development`, its badge, and its doctor note. The
   deployer's rehearsal against the dev slot (`--of dev`, `Production`)
   goes with it: the deployer serves the installed production only. The
   removed label is declared in the commit for the compatibility gate. The
   docs and AGENTS.md say a change is seen in a preview of its commit (`ren
   deploy HEAD --seed <n>`). Verified: lint, the 1182 tests (the 1231
   before, less the 49 that were the dev world's), the UI's 384, and the
   compatibility gate, whose report names only `instance::Environment`
   losing `development`.
6. **Production is a chosen deployment.** Built, as three commits.
   Production runs the commit chosen for it: the policy's or the operator's
   promotion of the canary's build, or `ren deploy <commit> --production`
   (the control API's `release/promote`, as the Release view's Promote).
   It takes any kept build that passed the canary's checks or ran
   production, so an earlier one rolls production back: each such build is
   marked in its folder, `release.json` lists them (`passed`, left out when
   none), and the gateway refuses any other with `409`. `ren deployer
   install-vps` installs once: where `ren.service` exists it refuses, its
   `--build` is gone, and `ren deployer settings` restarts production under
   changed publishing settings with the kept build it runs. The deployer's
   worktrees are `~/.local/share/ren/deployer/main` and `deployer/preview`,
   moved once from `factory` and `preview-factory` with `git worktree
   move`. Verified: the release's and the gateway's tests (a passed earlier
   build is taken, a failed or unknown one refused), the builds' marks and
   pruning, the install's refusal of a checkout over an installed service,
   a worktree moved and checked out again, lint, the 1185 tests, and the
   compatibility gate (no change at each commit). Not yet verified: a
   promotion of production through `ren deployer`, which the operator runs
   once this merges and production moves over.

## Open questions

- **The build cache.** One incremental cache per project makes work
  commits quick to deploy, but a contributor's pull request writes into
  the cache the operator's builds read. Options are one cache for trusted
  sources and a fresh one per untrusted build, or one per trigger.
- **What a scenario holds.** Calls and sessions for certain. Whether runs,
  work items, projects, and releases are seeded too, which needs the store
  to write each of them outside its live paths.
- **How the deployer upgrades itself.** It is part of the build it
  promotes, so it restarts into the new build after production's swap.
  The open question is what happens to a pass running at that moment.
- **Memory.** Each deployment is a daemon, a UI, and its data, so whether
  the policy caps a count or a memory budget.

## Deviations

- **Phase 1: generated, not scenario files.** The plan moved the
  scenarios into data files and wrote them through the recorder. Writing
  a scenario's calls needs each call's answer, which only the fake
  provider made, and stating every call as data would have repeated what
  the gateway works out. Instead a number generates the history, answers
  included, and the recorder takes it as the gateway would hand it over.
  There are no scenario names; `--seed <n>` replaces `--scenario`.
- **Phase 1: `ren seed`, not `ren store seed`.** `ren` has no `store`
  group, and one command does not need one.
- **Phase 1: calls only.** Agent runs, work items, and projects are not
  seeded; the open question of what a seed holds stands. The xtask
  scenario builders stay only for the traffic generator and the fake
  agents (now `Call`'s constructors), and go with them in phase 5.
- **Phase 2: no `ren-release`.** The plan moved the state machine into
  `ren-release` to keep it free of I/O; a pull request's number is
  `ren-github`'s, which does I/O, and staying pure is no reason for a
  crate. `ren-release` folded into `ren-gateway` instead, where the
  deployments now sit beside the release types.
- **Phase 2: `Seed` and the canary wait.** A deployment's seed has one
  value until phase 4 adds scenarios, and the canary has no building or
  running state to merge (`release.json` holds a verdict), so each joins
  `Deployments` with the phase that gives it one: seeds in phase 4, the
  canary when the deployer runs it (phase 3).
- **Phase 3: one move, not one per module.** The modules depend on each
  other in a web (the canary on builds, replicas, install, and checks;
  install on builds and the daemon's container), so they moved together,
  unchanged, after one commit that cut their ties to `dev`. The move is a
  file move and nothing else, which the comparison checks.
- **Phase 3: one configuration owner comes with phase 5.** Of the three
  derivations, `DevInstance`'s is the dev world's and goes with it;
  `DaemonContainer` reads a configuration rather than deriving one. What
  is left is `Replica`'s, already the one owner of a deployment's
  configuration.
- **Phase 3: the handed-off pass is checked once `main` carries it.** A
  timed pass fetches `main` and runs its build's `ren deployer`, which
  `main` lacks until this merges; the pass itself was checked through
  `--source checkout`.
- **Phase 3: the folders on disk keep their names.** The deployer's
  worktrees (`~/.local/share/ren/factory`, `preview-factory`) are
  production's state; renaming them would leave the old worktrees behind.
  They are renamed with phase 6, when production is a deployment.
- **Phase 3: production moves over by hand, once.** Its timer still runs
  `cargo xtask canary run` in the operator's checkout. After this merges:
  promote production to a `main` that has `ren deployer` (by hand, `cargo
  run --release -p ren-rs -- deployer canary run` from a checkout of
  `main`, then `canary promote`), then `ren deployer canary enable`
  rewrites the timer and writes `ren-deployer.service` around the installed
  `ren`, and `[deployer] socket` goes into production's configuration.
  Until then the timer's unit pins `RUSTUP_TOOLCHAIN=1.99.0` (set by hand on
  2026-11-25): its pass runs a toolchain's cargo directly, so each `rustc`
  went through rustup's proxy from the crate's own folder, and a crate
  shipping a toolchain file (`atoi`, Rust 1.57) switched compilers mid-build.
  `canary enable` drops the pin; the deployer's builds run the toolchain's
  own `rustc` in their container and need none.
- **Phase 4: the API keeps its paths.** `/control/v1/previews/ask` and
  `/withdraw` and `/api/projects/{name}/previews` keep their names and gain
  fields (an ask's `seed` and `work`, a preview's `seed` and `work`); the
  files keep their shapes for production's older gateway (a copy of
  production is written as before). Renaming the routes is a change of its
  own.
- **Phase 4: the seed is not a scenario.** The plan named scenarios; phase
  1 made an invented history a number, so a deployment's seed is a copy of
  production or a `SeedNumber`.
- **Phase 1: checked by its pages, not side by side.** The history is
  invented afresh, not the scenarios' calls replayed, so the two seeds
  differ by design; the pages were checked against the new one.
- **Phase 5: no rehearsal production.** The deployer rehearsed against the
  dev slot as production (`--of dev`), which xtask's dev world made. The
  operator chose to drop it rather than install a second `ren` to stand
  in: the deployer serves production only, so a change to the deployer is
  tried against production, and this phase's check of a preview on an
  invented history rendering every page waits for production to move over
  to `ren deployer`.
- **Phase 5: older plans keep their history.** `instances.md`,
  `self-build.md`, and `attachments.md` record what was built then,
  `cargo xtask dev` included, and are left as written.
- **Phase 6: production is not a row of `Deployments`.** Production's
  record stays `release.json` and its chosen commit the promotion request
  (`release-promote.json`): production's own, older gateway reads
  `previews.json` strictly, slot by slot, and a production row in it would
  stop that gateway reading the previews. The canary stays there too.
- **Phase 6: what production may take.** The operator chose kept builds that
  passed the canary's checks or ran production, rollback included, over
  only the canary's build or any commit. Builds made before marks existed
  count only while the release names them the canary's passed build.
- **Phase 6: `ren deployer settings`.** With upgrades only to kept builds,
  publishing settings needed a way in that builds nothing; the operator
  chose a command of its own over `install-vps` re-applying them.
- **Phase 6: no xtask install.** `cargo xtask install-vps` went with phase
  3; the first install is `cargo run --release -p ren-rs -- deployer
  install-vps` from a checkout, as the docs already said.
- **Phase 6: the Release view lists no earlier builds.** Rolling back is
  `ren deploy <commit> --production`; the UI shows `passed` nowhere yet.
