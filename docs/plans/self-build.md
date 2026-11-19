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
   `main` was unprotected, so a change could land untested or on a stale
   base.
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
- **Ren owns worktrees.** A change's branch and worktree are made, named,
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
3. **Worktrees.** Built; `main` is guarded by required checks, not a queue.
   - A session can work in a worktree of its own (`checkout:
     "worktree"`, `ren runs start --checkout worktree`): a worktree of
     the repository named, in `<data>/worktrees/<repo>-<slot>`, on a new
     branch `ren/<repo>-<slot>-<id>`. Deleting the session removes it,
     unless it holds changes not committed, and deletes its branch only
     once merged. Redline's briefs always start a session in one: the
     `WorkPlace` guess is gone.
   - Each worktree's slot, the lowest free, gives it 100 ports and a data
     folder no other worktree holds, which its runs are told as
     `REN_WORKTREE`, `REN_WORKTREE_PORTS`, and `REN_WORKTREE_DATA`.
   - Builds share a compiler cache: `sccache` is in the catalog, and
     `[runs] worktree_env` gives every worktree's runs what the
     operator declares (`RUSTC_WRAPPER = "sccache"`).
   - Verified in the dev slot, whose projects are now repositories: a
     worktree session ran in its own worktree and branch with its
     variables, and deleting it removed the worktree, its data, and its
     merged branch.
   - `main` is protected by its ruleset: every change lands through a pull
     request, squashed, once **CI** and **Compatibility** pass on a branch
     up to date with `main`. No one bypasses it, agents included.
4. **Canary and promotion.** Built, but for real runs and a promotion
   into the installed service, which were not run (see below).
   - `cargo xtask canary run` fetches `main`, checks its tip out in Ren's
     own worktree (`~/.local/share/ren/factory`), and goes on with that
     commit's own xtask. It builds the commit once into
     `~/.local/share/ren/builds/<commit>` and starts that build as the
     canary (`ren-canary-*`, loopback `17787` and `17000`, `--publish` for
     the tailnet) on a fresh backup of production's history, with
     production's configuration but its own listener and data,
     `environment = "canary"`, no runs, and no operator key.
   - Its checks: `ren doctor` (with the build check), a sweep of every read
     of the read API over the copied history (a test matches every
     `ReadRequest` against it), and every page of the UI in a headless
     browser, which fails unless the UI answers it with a success (a
     project's pages are a project's in the copied history). A check that
     cannot run fails; a check left out is never a
     pass (`ren_release::Verdict`).
   - The pure `ren-release` crate decides: the operator's policy
     (`~/.config/ren/release.toml`: `manual`, `automatic`, or a daily UTC
     window) promotes a passed build that follows production's commit, and
     never one that failed, is current, would lose production's commits,
     or replaces a build not of a clean commit. Promotion is `install-vps
     --build <commit>`, the install's steps with the kept build; a failed
     one is rolled back by the install, marked, and not retried on its
     own. `canary promote` promotes by hand; `canary enable` runs a pass on
     a user timer.
   - The release is written to production's data folder; `/api/release`
     serves it, `ren release` prints it, and the UI's status line shows
   - The UI's Release view shows the builds side by side, the canary's
     checks and the pages it captured, the decision with a Promote (a
     request the factory's next pass carries out), the work each build
     shipped or waits to ship, and each project's release.
     how far production is behind `main` and the canary's verdict.
   - Verified with the dev slot as production (`--of dev`): a pass built
     the commit once, copied the slot's history, started the canary (its
     wordmark tagged `canary`, read-only), passed doctor, 19 reads, and all
     9 pages, and wrote the release; its decision was `unconfirmed`, since
     the slot ran a modified build. `ren release` and the dev UI's status
     line showed it.
   - Not built: one short real run per client. Whether two instances can
     drive one client sign-in, and how such runs are budgeted, are still
     open questions below.
   - Not run: a promotion into the installed service and the timer. They
     install into production, which stays a step the operator asks for;
     the install's own steps are the ones `install-vps` has always run.
5. **Work items and Redline.** Built, but for screenshots: work items,
   from a mark to the build that ships it, and previews of their change.
   - A work item is what was marked to be done: the page (its path, route,
     size, theme, and the build that served it), the noted marks, and what
     else was on screen (the errors the page logged and the API reads
     behind it, kept by the UI's layout). Its lifecycle is a pure state
     machine (`ren_recorder::work::WorkState`): queued, sent to a session,
     landed on `main` as a commit, shipped in the build production runs, or
     dismissed. It is stored in `work_items` (migration 0002, pinned with
     its fixture) and served at `/api/work`; the control API queues and
     moves it on.
   - Every Redline brief records a work item, names it, and carries its
     evidence; Later keeps one queued. The Work view lists them and sends
     queued items together as one brief to a new session in a worktree of
     its own (phase 3).
   - Marks on a recording, a call, a transcript step, or an error are marks
     on the page that shows it: its path names the subject.
   - Verified in the dev slot, in a headless browser driven through the real
     UI: a mark kept for Later was stored with its route, the build, and the
     page's reads; two queued items were sent as one brief to a new session
     in its own worktree, whose first prompt held both; a Redline Send
     recorded its item as sent to the session it made.
   - The factory closes them: a brief asks for a `Work-Item: <id>`
     trailer in each commit and in the pull request's description, which a
     squash merge keeps; each canary pass (and `cargo xtask canary work`)
     lands every sent item a commit on `main` names, and ships every landed
     item whose commit production's clean build holds, so the mark closes
     with the revision that shipped it.
   - Verified with the dev slot as production, run from a throwaway commit
     naming a sent item: the item landed as that commit and shipped in the
     slot's build, in one pass; an item landed earlier shipped in a later
     pass. Both ended on the Work view's archived shelf.
   - A preview runs a worktree's branch on a copy of production's
     history (`cargo xtask preview up`, which a brief asks the agent to
     run): the canary's replica, on the worktree slot's first two ports,
     recorded on the sent work items the branch names, so the Work view
     opens the marked page on the branch and in production side by side.
     Deleting the worktree's session stops it.
   - Verified with the dev slot as production: a preview of this branch,
     run as the dev worktree `acme-web-1`, served `/recordings` on the
     slot's copied history with its preview tag, was recorded on that
     worktree's two items (the Work view showed "On the branch"), and
     stopped when the session was deleted.
   - Deleting a worktree's session takes its preview off the session's
     work items too, so no link leads to a port nothing serves.
   - Not built: a screenshot with the marks (a page cannot capture itself
     without the browser asking, or a library).
6. **Projects.** Built: a project's contract, checks, previews, canary,
   and promotion, shown in the Release view; with no timer of its own.
   - A repository declares in `.ren/project.toml` how it is built, served,
     checked, and quiesced (`ren-project`, sans-IO): argv steps whose
     values name the worktree's ports, data, and folder through parsed
     placeholders. Its `tools` name the depot packages its steps need;
     every step runs with Ren's current toolset first on its `PATH`, and a
     tool lacking refuses the command (or, in a canary, fails its build).
   - `ren project check` runs its checks in a worktree; `ren project
     preview up` builds it, serves it on the worktree's ports as
     `ren-preview-<worktree>-serve`, and records the preview on the work
     items the branch names, as `cargo xtask preview` does for Ren.
     Deleting the worktree stops it.
   - Verified in the dev slot, whose fake projects now declare a contract:
     in a new `acme-web` worktree, its check passed, its preview served
     the page on the slot's first port and was recorded on the session's
     work item, and deleting the session stopped it.
   - `ren project canary` checks a project's `main` as Ren's canary does:
     in a worktree the factory keeps for it, it runs the build, serves it,
     and runs each declared check against it, into a verdict whose suite
     requires all of them (`ren_release::Suite::Project`). The operator's
     policy for the project (`[projects.<name>]` in `release.toml`) decides,
     through the same `Decision` and `Lineage` rules as Ren's, whether its
     `[deploy]` step runs; a failed deploy is not retried on its own.
     `ren project promote` deploys by hand; `ren project release` prints
     where it stands.
   - Verified with the dev slot's `acme-web`: a first pass was
     `unconfirmed` (nothing deployed yet) and was promoted by hand; under an
     automatic policy a new commit was checked and deployed in one pass; a
     commit whose check failed was not deployed, leaving production one
     commit behind.
   - Not built: a timer for project passes (run `ren project canary` from
     any scheduler).

Phases 1 and 2 come first: without them, upgrading automatically only breaks
things faster. Each phase is useful on its own.

## Open questions

- Canary's real runs use the operator's client sign-ins on the same host.
  Ren never reads them, but whether two instances can drive one client
  sign-in at once is unchecked for each client.
- Allowance: how canary's real runs are budgeted, and whether a failure in
  them blocks promotion or only warns.
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
- **Worktrees are found by where they are, not recorded.** A session's
  folder in the worktrees folder, named `<repo>-<slot>`, is a worktree;
  nothing new is stored, and a slot is free once its folder is gone.
- **The compiler cache is the operator's to declare.** Ren knows no
  language: it installs `sccache` like any utility and passes
  `worktree_env` to worktrees' runs.
- **The run host is `ren` itself.** A hidden `ren run-host` command, so it
  is built, installed, and upgraded with the daemon; a host started by an
  older build goes on running that build, which is why its frames keep
  what they do not know.
- **Required checks, not a merge queue.** GitHub offers its merge queue
  only to repositories an organization owns, and `ren` is a user's. A
  branch must instead be up to date with `main` before it merges, which
  keeps the queue's guarantee (nothing lands untested against the current
  `main`); what is lost is the automatic retest, so a waiting pull request
  is updated by hand (`gh pr update-branch`) after `main` moves.
- **A timer, not a hook on each merge.** A pass fetches `main` on a user
  timer and builds a tip it has not built; tips that land between two
  passes are checked together. Nothing on GitHub needs to reach the host.
- **The release is a file the factory writes.** The factory (xtask) writes
  `release.json` to production's data folder and the gateway only reads it,
  so the daemon runs no builds and holds no factory state.
- **Build identity moved to `ren-release`.** `Build`, `Revision`, and
  `Commit` left the gateway for the new crate that owns how builds move,
  so the gateway, the CLI, and xtask share one commit type for releases.
- **The UI's evidence, not the server's.** What was on screen is what the
  browser had: the errors the page logged and its reads of the API, as
  resource timing saw them. The API responses themselves are not copied:
  the reads name them, and the agent can read them again.
- **`ReadBody` is no longer a wire type.** Adding the work reads widened
  the union of every read's answer, which the compatibility gate rightly
  calls a break for a type decoded whole. No client decodes it whole, so
  it stopped being written to the UI's wire types, declared as a break.
- **A trailer, not a branch, says what landed.** Squash merges leave no
  trace of a branch on `main`, so the change names its items itself, in a
  `Work-Item:` trailer the brief asks for. It works for any merge style and
  any host, and needs only git.
- **A preview is a replica, run by the agent.** The canary's instance
  became `Replica`, which a preview reuses on its worktree's ports; the
  agent starts it from its worktree, which already holds the branch and
  its build, rather than production building branches.
- **Ren keeps its own steps.** Ren's preview and canary stay in xtask:
  they copy production's history and drive two processes, more than a
  contract's serve step says. A project's contract covers one process
  served on its worktree's ports.
- **The gate is its own workflow.** It reads the pull request's title and
  body, so it runs again when they are edited, which would otherwise
  rerun all of CI.
