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

1. **Compatibility contract.** Not built. Enforced in CI, not by review:
   - Shipped migrations are frozen. A schema change is expand then
     contract: a column is never dropped or tightened in the release that
     stops writing it.
   - A check opens a store made by the previous release's schema, filled
     with synthetic rows, with the change's build, and reads it through
     every `ReadRequest`.
   - The read and control APIs' wire types, the `labels.txt` tables, and
     the control refusals are compared with the previous release. A
     breaking difference fails the check unless the change carries a
     migration note naming what it breaks and how an existing install
     moves on.
2. **Runs outlive the daemon.** Not built. Each agent process runs in its
   own transient systemd user unit, holding its client's stdio on a socket
   the daemon reconnects to when it starts. The supervisor's run machine
   gains a reattaching state; the recorder records the gap as a gap, never
   as a quiet turn. A restart then pauses admission, waits for running
   calls, and restarts, with no wait for turns. `install-vps` and `dev up`
   stop waiting for runs. macOS keeps today's behaviour until launchd has
   the same.
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

None yet.
