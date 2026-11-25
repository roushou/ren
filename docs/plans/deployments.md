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

- `ren-release`, pure: `Build`, `Verdict`, `Policy`, `Release`.
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
  machine lives in `ren-release` beside `Build` and `Policy`, the lowest
  crate that owns the rules. The gateway reads it, and the deployer writes
  it.
- **The deployer is a component, not glue.** A new crate, `ren-deployer`,
  primary type `Deployer`, is the shell around `ren-release`. It builds
  commits in containers, seeds their data, starts their pods, runs the
  canary's checks, and promotes. It ships in the `ren` binary as `ren
  deployer` commands and runs as `ren-deployer.service`, installed and
  upgraded with the build it belongs to, never from a checkout. It replaces
  the factory service, and `[factory]` becomes `[deployer]`. The section
  has never been set in production's configuration, so it gets no alias.
  Layering: `ren-deployer` depends on daemon, container, and release, and
  `ren-cli` depends on it.
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

1. **Seeding into the store.** Not built. `ren store seed` writes a
   scenario into an empty store through the recorder and the store, with a
   fixed clock and sequential ids. The scenarios move from
   `xtask/src/dev/seed/scenario.rs` into data files. `cargo xtask dev
   seed` calls it in place of playing calls through the fake provider.
   Verified by comparing the dev UI's pages, and the read API's answers,
   seeded both ways.
2. **`Deployments`.** Not built. `ren-gateway`'s `Instances` moves into
   `ren-release` as `Deployments`, with `Trigger` and `Seed`. The
   canary's deployment is part of it, and its `release.json` is unchanged.
   The files keep the shapes production's older gateway reads until
   production runs a build that reads the new one. Verified by the state
   machine's tests (legal and illegal transitions, a lost outcome) and the
   files' pinned shapes.
3. **`ren-deployer`.** Not built. Each of xtask's runtime modules moves
   into the crate in its own commit, bottom-up: build jobs and builds,
   pods and units, the configuration derivation (one owner, replacing
   three), the canary and its checks (`ren check`, with the page shots),
   the previews pass, then the service. `ren deployer serve` replaces
   `cargo xtask canary serve`, and `[deployer]` replaces `[factory]`.
   Verified by comparison against the commit before: the pinned labels,
   the test count, `ren --help`, and a whole canary pass in a rehearsal
   whose production is a scenario-seeded deployment.
4. **Chosen previews, seeded.** Not built. `ren deploy <commit> --seed
   <scenario | replica>` starts a chosen preview and answers with its
   state and address. `ren deployments` lists them, and `ren deploy stop`
   stops one. The worktree preview (`xtask/src/preview.rs`) and `ren
   preview ask`'s file go. Verified by deploying a branch's commit on a
   scenario and reading its pages and `/health`.
5. **The dev world goes.** Not built. Delete `xtask/src/dev` (instances,
   claims, index, the fake provider and agents, traffic, seed's player),
   `devtraffic.ts` and the toolbar, and `Environment::Development` with its
   badge and doctor note. The removed label is sent in `/health`, so the
   commit says so for the compatibility gate. The docs and the memory of
   how changes are previewed change with it. Verified: lint and tests
   pass, the compatibility gate's report names only the removed label,
   and a preview of this commit on a scenario renders every page.
6. **Production is a chosen deployment.** Not built. `install-vps`'s
   upgrade (drain, swap, check, resume) moves into the deployer as
   deploying the chosen commit to production. Promotion is choosing the
   canary's build. `cargo xtask install-vps` shrinks to the first install.
   Verified by a promotion of production through `ren deployer`, which the
   operator runs.

## Open questions

- **The build cache.** One incremental cache per project makes work
  commits quick to deploy, but a contributor's pull request writes into
  the cache the operator's builds read. Options are one cache for trusted
  sources and a fresh one per untrusted build, or one per trigger.
- **What a scenario holds.** Calls and sessions for certain. Whether runs,
  work items, projects, and releases are seeded too, which needs the store
  to write each of them outside its live paths.
- **`ren-release`'s primary type.** With `Deployments` in it, the crate
  may be better named for that type than for `Release`.
- **How the deployer upgrades itself.** It is part of the build it
  promotes, so it restarts into the new build after production's swap.
  The open question is what happens to a pass running at that moment.
- **Memory.** Each deployment is a daemon, a UI, and its data, so whether
  the policy caps a count or a memory budget.

## Deviations

None yet.
