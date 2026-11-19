# Plan: self-build

Ren is built by agents that Ren itself runs. This plan makes that safe: an
agent can change Ren, the change is shown to work against production's own
data, and production upgrades to it without stopping the agents doing the
work. The same capability then serves projects built with Ren. This plan is
the design and the order it is built in; each phase says whether it is built.
Record every deviation here, with its reason.

The principle: **production only ever receives a build that has already run
against a copy of production's data, and an upgrade is a non-event for
everyone using it, the agents included.**

## What stands in the way

1. **Production is the factory and the product.** The agents changing Ren
   run inside the production daemon, so restarting the daemon stops them.
   `cargo xtask install-vps` waits until no run is starting, working, or
   stopping (`--idle-timeout`, 900 seconds by default, then it gives up),
   and the idle runs end with the old daemon. With several agents busy, a
   quiet moment may not come, and each upgrade ends their processes.
2. **No compatibility contract.** A merge may require a fresh data directory
   or change an API the deployed UI reads. Nothing can upgrade itself while
   any commit may do that.
3. **Nothing between a merge and production.** CI checks a commit on
   GitHub; the next step is a manual install into the instance the operator
   uses every day. The dev slot cannot fill the gap: one per host, fixed
   ports, fake providers and data.
4. **Agents share one checkout.** Redline's brief asks an agent in prose to
   use a git worktree when another session is open in the checkout
   (`ui/apps/web/src/lib/briefing.ts`, `WorkPlace`). Nothing enforces it,
   each worktree is a cold Rust build, and checks run in parallel collide.
   `main` is unprotected, so merges are not queued; CI already accepts
   `merge_group` events.
5. **Feedback goes one way.** A Redline brief reaches a session, and there
   the loop ends: the mark never learns which branch answered it, cannot be
   checked on that branch's build, and is never closed by a release.

## Decisions

- **One long-lived branch.** `main` is the trunk. No `dev` or release
  branch. Each change is a short-lived branch, squash-merged into `main` as
  one conventional commit.
- **Promote builds, not branches.** A commit is built once. The build that
  passed canary is the build production runs; nothing is rebuilt per
  instance.
- **One owner per store.** Every instance has its own data directory, ports,
  and Tailscale route. No instance opens another's storage; canary and
  previews get a copy of a production backup (`ren control backup`).
- **Only production runs the factory.** Agents working on Ren run in
  production. Canary and previews are what they build and check, never
  where they run, so an upgrade never removes the instance carrying it out.
- **Ren owns workspaces.** A change's branch and worktree are made, named,
  and removed by Ren, never improvised by an agent from a prompt.
- **Promotion is a policy.** Production takes a build that passed canary
  automatically, in a window, or on the operator's approval, as configured.
  An agent can prepare a promotion; the policy decides whether it needs a
  person. Until phase 4 is built, installing stays a separate step the
  operator asks for.
- **A way back that does not need Ren.** Production keeps its last good
  build and the backup taken before the upgrade. An upgrade that fails its
  checks is rolled back without any agent or UI, as `install-vps` does
  today.
- **Ren first, projects next.** Each part is built for Ren and described as
  a project's declared contract (build, run, check, quiesce), so a project
  built with Ren can declare its own. The generic form waits until Ren uses
  each part.

## Instances

| Instance   | Built from              | Data                                    | Lifetime                       | Used by                             |
| ---------- | ----------------------- | --------------------------------------- | ------------------------------ | ----------------------------------- |
| Production | the last promoted build | the real store                          | permanent                      | the operator and the working agents |
| Canary     | the tip of `main`       | a copy of production's latest backup    | permanent, replaced each merge | Ren's checks; the operator, to look |
| Preview    | one branch, on demand   | a production copy, or the fake provider | until its branch merges        | the operator, judging a change      |
| Dev slot   | the checkout            | fake                                    | while developing               | UI work and scripted scenarios      |

The dev slot exists (`cargo xtask dev`). Each instance names itself with
`[instance] environment`, which `/health` reports.

## Phases

1. **Compatibility contract.** Built. Enforced in CI, not by review, by
   the Compatibility workflow (`cargo xtask compat`):
   - Shipped migrations are frozen: each is pinned by the checksum a store
     keeps for it (`crates/ren-store/tests/fixtures/migrations.txt`), and
     the gate refuses a changed or removed pin. A schema change is expand
     then contract: a new migration that drops, renames, or deletes is a
     breaking change, read from its statements.
   - A store written at each earlier migration opens with the change's
     build and serves every read: each migration has a store fixture
     (`crates/ren-store/tests/fixtures/stores/<version>.sql`), recorded
     once through the recorder and frozen, and
     `crates/ren-store/tests/upgrade.rs` opens each with today's
     migrations and serves every `ReadRequest` and session read from it.
   - The `labels.txt` tables, the control refusals among them, are
     compared with the base: a label may be added, never removed.
   - The API's wire types are compared with the base by structure, parsed
     from the files ts-rs writes (`ui/apps/web/src/lib/wire`). A type
     reached from one of the gateway's request types
     (`crates/ren-gateway/tests/fixtures/requests.txt`, every type a route
     reads from a body or query string) must still read what older
     clients send: no new required field, no field removed (requests deny
     unknown fields), nothing narrowed. Any other type must still hold
     what older clients read: no required field removed, nothing made
     optional, nullable, or wider. A new label or variant in a response is
     compatible, as the UI keeps unknown labels and variants. A wire file
     the gate cannot read is a break.
   - A breaking difference passes only when the change's message declares
     it: a `!` in its header and a `BREAKING CHANGE:` footer naming what
     breaks and how an existing install moves on.
2. **Runs outlive the daemon.** Built (`[runs] hold`, `auto` by default).
   - Each run's client is started by a run host, `ren run-host`, in a
     transient systemd user unit of its own (`ren-run-<run>`), outside
     `ren.service`. The host keeps the client's stdin open, appends all it
     writes to the run folder's spool, writes its exit, and serves one
     daemon at a time over a framed socket, from any offset.
   - The daemon journals every input it gives a held process before the
     process sees it: its start, how far its output was read, the
     operator's commands, idle stops. The next daemon replays the journal
     and the spool through the same pure `Process` machine and takes the
     run up exactly where the last one left it: the codec's requests,
     approvals, and turn included.
   - The recorder records the gap: `run_detached` as a daemon stops,
     `run_rejoined` as the next takes the run up. The store leaves held
     runs open; a run that cannot be taken up is stopped as `recovered`.
   - `install-vps` and `dev up` wait only for runs bound to the daemon
     (`RunSnapshot::tenure`); calls are still waited for, and admission is
     still paused around the restart.
   - macOS, and any daemon not under systemd, keeps runs bound, as before.
   - Verified in the dev slot under systemd: a fake Codex run waiting on an
     approval kept its process through `systemctl --user restart
     ren-dev-daemon` and through `cargo xtask dev up`, which did not wait
     for it; the new daemon took it up within a second, the approval was
     answered through it, and the turn completed and was recorded, with
     `run_detached` and `run_rejoined` around the restart. Stopping the run
     ended its unit and removed its folder.
3. **Workspaces.** Not built.
   - Every change gets a branch and worktree Ren makes from `main` and
     removes after it merges or is dropped. Redline's `WorkPlace` choice
     goes away: a brief always lands in its own worktree.
   - Builds share a compiler cache, so a new worktree is not a cold build.
   - Ren allocates each worktree's ports and data folders, so checks and
     previews cannot collide.
   - Merges go through a queue: each change is rebased on the tip, checked,
     and squash-merged in turn. GitHub's merge queue on a protected `main`
     is the first form.
4. **Canary and promotion.** Not built.
   - Each merge to `main` is built once. The build is installed as canary
     on a fresh copy of production's latest backup.
   - Canary runs its checks: `ren doctor`, every `ReadRequest` against the
     copied history, one short real run per client (only when the harness
     or supervisor changed, at most hourly), and `dev shot`'s screenshots.
   - A passing build is promoted under the operator's policy with
     `install-vps`'s steps: paused admission, backup, swap, the build check,
     and rollback on failure.
   - `ren` and the UI report each instance's build and how far production
     is behind `main`.
5. **Work items and Redline.** Not built.
   - A mark becomes a work item carrying what was on screen: the revision,
     route, screenshot, the data shown, the API response behind it, and
     console errors. Marks are not limited to the UI: a recording, a call,
     a transcript step, or an error can be marked.
   - A work item is carried out in its own workspace (phase 3). Its preview
     opens the marked page on the branch's build, beside production's.
   - Accepting it sends it through the queue and canary. The mark closes
     with the revision that shipped it, linked to the recordings of the
     work that made it.
   - Marks can wait in a queue and be sent as one batch.
6. **Projects.** Not built. A project declares its build, run, checks, and
   quiesce steps, and gets workspaces, previews, canary, and promotion as
   Ren does.

Phases 1 and 2 come first: without them, upgrading automatically only breaks
things faster. Each phase is useful on its own.

## Open questions

- Canary's real runs use the operator's client sign-ins on the same host.
  Ren never reads them, but whether two instances can drive one client
  sign-in at once is unchecked for each client.
- Allowance: how canary's real runs are budgeted, and whether a failure in
  them blocks promotion or only warns.
- Where the build is kept between stages: a content-addressed folder per
  commit, like the depot's installs, is the likely form.
- Whether previews need real clients at all, or the fake provider is enough
  for the UI changes they are mostly for.

## Deviations

- **The base is `main`, not a release.** Ren has no tagged releases, and
  production follows `main`, so the contract compares a change with the
  revision it lands on: a pull request's base, or `main` before a push.
- **Store fixtures are recorded, not written by an earlier build.** Each
  is recorded once, through the recorder at the migration it belongs to,
  and frozen. Building every earlier release in CI would be slower and
  would need its toolchain; a frozen fixture checks the same upgrade.
- **A type's direction is read, not declared per type.** The gateway pins
  only its request types, found in its sources by the extractors that read
  them; every type is classed by what reaches it. A type reached from a request
  and from a response is compared both ways.
- **No reattaching state; a journal and a replay instead.** The plan had
  the run machine gain a reattaching state and the host relay the client's
  stdio. Relaying alone is not enough: each codec keeps state only in
  memory (JSON-RPC requests waiting, approvals asked, the turn), and a
  daemon rebuilt without it would read answers to requests it never sent.
  The daemon journals its inputs, and the next one replays them through the
  same pure machine, so no machine changed. A journal that falls 4096
  entries behind or fails is broken, and its run is drained like a bound
  one.
- **The run host is `ren` itself.** A hidden `ren run-host` command, so it
  is built, installed, and upgraded with the daemon; a host started by an
  older build goes on running that build, which is why its frames keep
  what they do not know.
- **The gate is its own workflow.** It reads the pull request's title and
  body, so it runs again when they are edited, which would otherwise
  rerun all of CI.
