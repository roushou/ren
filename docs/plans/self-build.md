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
- **A commit preview is the primitive.** A build is one commit's; an
  instance runs one build. A branch preview is a commit preview that follows
  its pull request's head, and the canary is the one that follows `main`'s.
  On-demand previews of a commit need nothing beyond that.
- **GitHub is observed, not trusted with Ren's state.** Ren reads pull
  requests, their checks, and their heads, and records what it saw. Work
  items, builds, and releases stay Ren's; GitHub only links to them.
- **Nothing on GitHub reaches the host.** Ren polls GitHub; it takes no
  webhooks, so no listener is public and a laptop works the same as the VPS.
- **Previews still run no agents.** Isolating a preview's runs from
  production (a microVM per run, such as Docker Sandboxes, or containers) is
  an open question, not part of these phases.
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

Phases 7 to 9 reshape this table into one kind of instance, described by a
spec (phase 7):

| Instance       | Built from                          | Data                 | Lifetime                                |
| -------------- | ----------------------------------- | -------------------- | --------------------------------------- |
| Production     | the last promoted build             | the real store       | permanent                               |
| Canary         | follows `main`'s head               | a copy of production | permanent; replaced on each new head    |
| Branch preview | follows an open pull request's head | a copy of production | until its pull request merges or closes |
| Commit preview | one commit, on demand               | a copy of production | until stopped, or evicted by the policy |
| Dev slot       | the checkout                        | a seeded scenario    | while developing                        |

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
     checked, and quiesced (`ren-manifest`, sans-IO): argv steps whose
     values name the worktree's ports, data, and folder through parsed
     placeholders. Its `packages` name the depot packages its steps need;
     every step runs with Ren's current package set first on its `PATH`, and a
     package lacking refuses the command (or, in a canary, fails its build).
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
7. **One kind of instance, in containers.** In progress: the canary and
   previews run in pods; one spec, the scenario data source, and the dev
   slot are not built.
   - Today the dev slot, previews, and the canary are three code paths
     (`xtask/src/dev/slot.rs`, `preview.rs`, `replica.rs`). Each one picks
     its own ports, unit names, data folders, and tailnet routes. They
     become one `Instance`, described by a spec that is data: the build it
     runs (a commit's kept build, or the checkout), its data source, and
     its name.
   - The data source is an enum: a copy of production's latest backup, or
     a seeded scenario (today's `dev seed`, with the fake provider). The
     dev slot becomes an instance of the checkout with a scenario. That
     gives it the same daemon, UI, and checks as every other instance,
     instead of a parallel set.
   - Each instance runs as one rootless Podman pod (the daemon and the
     UI), under the operator's systemd user, built from the kept build of
     its commit. The image packages the binary and UI bundle that
     `builds/<commit>` already holds; it never rebuilds them, so promoting
     builds rather than branches still holds. The image grows a UI layer;
     the published `Dockerfile` stays the daemon's.
   - The pod publishes one loopback port. Ren's tailnet route points at
     it, as `--publish` does now. Stopping an instance removes its pod and
     its volume; nothing is left in the user's units or folders.
   - Production stays a host install in this phase (phase 11 moves it):
     the laptop story and `install-vps`'s rollback do not change.
   - The canary and previews keep their rules: no runs, a placeholder for
     every operator key.
   - Built: `Replica` runs as the pod `ren-<name>` (`xtask/src/pod.rs`),
     its daemon and UI each a container seeing only the kept build it
     runs, read-only, and the replica's own configuration and data. Its
     start stops the transient units an older xtask ran it as, by name.
     Removing a worktree removes its preview's pod.
   - Verified with the dev slot as production, from a worktree folder of
     slot 9: `preview up` built the image once, started the pod, and served
     the copied history read-only on the slot's ports (`/health` said
     `preview`); inside the daemon's container, production's data and
     `~/.config` did not exist. A stand-in unit holding the daemon's port
     under the old unit name was stopped, and `preview down` removed the
     pod.
8. **GitHub, observed.** Built, but for posting anything to GitHub, which
   is phase 9's. Ren shows what is happening on GitHub,
   so the operator watches pull requests in Ren, not on github.com.
   - A sans-IO crate parses GitHub's REST answers into typed observations:
     a pull request (its number, head commit, base, mergeability, whether
     it is up to date with its base, review state), its check runs, and
     `main`'s head. Each type keeps an unknown variant with the raw value.
     A pull request's number is its own type, never a work item's ID.
   - The daemon polls each registered project's repository on an interval,
     with conditional requests (an unchanged answer costs no rate limit),
     and records what changed as observations. The HTTP client lives in
     the daemon's shell.
   - Ren holds its own fine-grained token, read-only in this phase, as an
     operator key: it is never read from `gh` or any other client's store,
     never recorded, and never in `Debug`, errors, or logs.
   - The UI lists each project's pull requests with their checks, the
     work items their `Work-Item:` trailers name, and their preview. A
     work item shows its pull request, which closes the loop phase 5
     started.
   - Built: `ren-github`, a self-contained GitHub client (the query,
     what an answer shows, the client, and the poller), the `[github]`
     section mapping projects to repositories and the daemon running the
     poller, `GET
     /api/projects/{name}/pulls`, and the project Overview's Pull requests:
     each open one with its checks, whether it can merge, its review, its
     branch, and the work items it names, drawn faded under the error when
     the newest poll failed. The dev slot's fake provider answers the query
     for `acme/web` and `acme/api` with invented pull requests.
   - Not built: a work item showing its pull request (the pull request
     shows its work items), and each pull request's preview, which is
     phase 9.
   - Verified in the dev slot: both fake projects' Overviews listed their
     pull requests from the fake GitHub (passing and approved, behind with
     checks running, conflicting, blocked, and a draft), and Ren's, not
     watched there, said so.
9. **Previews of every pull request, and of any commit.** Built, but for
   GitHub deployments and the canary as the preview of `main` (see below).
   - A preview's lifecycle is a pure, persisted state machine: requested,
     building, running, superseded, stopped, failed. Its transitions are
     `(State, Event) -> (State, Vec<Action>)`, tested without I/O.
   - A commit preview is requested by hand (`ren preview <commit>`, or
     from the UI) and builds that commit once, like the canary does.
   - A branch preview follows an open pull request: the factory's pass
     reads the pull requests phase 8 observed, previews each head it has
     not built, supersedes the head's previous preview, and stops it when
     the pull request merges or closes.
   - The canary is the branch preview of `main`, with checks and
     promotion on top; it builds `main`'s head as soon as a pass sees it.
   - How many previews run at once and which one is evicted first are
     policy, in `release.toml`. Each one is a daemon, a UI, and a copy of
     production, and the host's memory is finite.
   - The factory builds previews; the agent no longer runs `cargo xtask
     preview up`. This replaces the deviation "A preview is a replica, run
     by the agent".
   - Built: `ren-preview`, since folded into `ren-gateway`'s `preview`
     module (`Previews`, the slots and `want`, and
     `PreviewRequests`), `Git::fetch_commit`, and the factory's pass
     (`xtask/src/canary/previews.rs`), run by every canary pass and by
     `cargo xtask canary previews`; a pass keeps its previews' builds.
   - Verified with the dev slot as production and its `acme-web` as the
     project: a commit asked for was built once and started in
     `ren-preview-0`, answering as that commit; the fake pull requests'
     invented heads failed to fetch and were kept as failed, with why,
     and not tried again; a second pass changed nothing; withdrawing the
     ask gave its slot to the next pull request, whose failure left the
     slot empty.
   - Built: `GET /api/projects/{name}/previews`, `POST
     /control/v1/previews/ask` and `/withdraw`, `ren preview
     list|ask|withdraw`, and on the project Overview each pull request's
     preview (a link when the tailnet reaches it) and the Commit previews,
     with a field to ask for one and Withdraw. A running preview records
     whether the pass published it (`Reach`): the canary timer publishes
     only when enabled with `--publish`, and a preview on loopback alone
     says so rather than linking where no browser elsewhere can go.
   - Verified in the dev slot, whose fake GitHub now has `acme/ren`: the
     pass previewed an asked-for commit in slot 0 and recorded pull
     request #7's unfetchable head as failed in slot 1; Ren's Overview
     showed both; `ren preview ask`, `list`, and `withdraw` changed the
     asks through the dev daemon, and a second withdrawal was `404`.
   - Not built: GitHub deployments, which need a token that can write;
     and the canary as the branch preview of `main`: it keeps its own
     pass and ports, beside the previews.
   - With a token allowed to write deployments, each preview is posted to
     its pull request as a GitHub deployment, so its address appears on
     the pull request too. Merging from Ren is not part of this phase: it
     acts on GitHub for the operator and would always ask first.
10. **Release rules.** Planned. The canary proves behaviour, not only that
    it starts.
    - Today the Ren suite is doctor, reads, and pages: the build runs and
      answers. The suite grows checks that drive the canary as a client
      would, against the fake provider, each written as a `CheckKind` the
      `Suite` requires:
      - a turn through the gateway for each wire format, recorded and read
        back complete;
      - the fake provider's `cut`, `slow`, `error`, and `rate_limited`
        replies, each recorded with the outcome the lifecycle invariants
        require (interrupted, failed, never rounded up to success);
      - the way back: production's current build opens the store the
        canary's build migrated, so a rollback after promotion works.
    - Each rule is data in `ren-release`; a rule that cannot run fails, as
      every check does now.

11. **Production in a pod, each run in a container.** In progress: runs in
    containers are built (`[runs] hold = "container"`), and so are the
    daemon and the UI in containers (`install-vps --contain all`, `cargo
    xtask dev up --contain all`); production runs that way. Every instance runs the same
    way, so the canary checks what production runs, and a run sees only
    what it works on.
    - Runs first, while production is still a host install. A run host
      starts in a container of its own, `ren-run-<run>`, from the instance
      image: `Launcher::Container`, beside `Systemd` and `Direct`. It sees,
      at their host paths, only the run folder (spec, socket, spool,
      journal), the run's workdir read-write, the package generation the
      run is pinned to read-only, its client's own sign-in folders, and the
      SSH agent's socket. The run-host protocol does not change: the daemon
      reaches the socket in the run folder, and a held run outlives the
      daemon, as now.
    - Which folders a client keeps its sign-in in is the harness's to
      declare (`ren-harness`), beside its launch plan. A client whose
      folders are not declared does not start in a container.
    - Then production as the pod `ren-production`, from the instance image:
      its daemon and UI see its kept build read-only, its configuration,
      and its data folder read-write, on loopback `8787` and `3000` as now,
      so clients and Tailscale Serve do not change. The daemon starts run
      containers through the operator's rootless Podman socket, mounted
      into it.
    - Promotion swaps the pod: back up with the running daemon, stop the
      pod, start it on the new kept build over the same data, and wait for
      `/health`; on failure, start the previous build's pod again.
    - The new build reads production's configuration before anything
      stops, so a configuration the new build refuses fails the canary
      pass, not production. A configuration only the new build reads does
      not stop the backup: the backup is the running daemon's.
    - The dev slot becomes a pod of the checkout with its scenario, which
      closes phase 7.
    - `install-vps` stays the host install a laptop uses. Whether the VPS
      keeps it as a fallback is decided once the pod swap is verified.
    - Built: `Launcher::Container` (`ren-daemon`'s `host/held/container.rs`)
      starts `ren run-host` as its container's entrypoint, from a transient
      scope of its own (`ren-run-<run>.scope`), so its monitor outlives the
      daemon's unit. Each client's profile declares what it keeps under the
      home (`ren-harness`'s `HomeEntry`). The client's program is the file
      found on the run's path; anything else a run starts must be under
      `[runs.container]`'s paths. `cargo xtask dev up --hold container` holds
      the dev slot's runs this way.
    - Verified in the dev slot: a Codex run in a worktree saw its worktree,
      the repository's git folder, `.codex`, and the SSH agent, and not
      `~/.config` or the main checkout; it was taken up again through a
      daemon restart; stopping it removed its container and its scope.
    - Built: the daemon in a container (`DaemonContainer`, xtask), started
      by `install-vps --contain all` as `ren.service`'s foreground
      `podman run`, or as the dev slot's daemon unit. It starts its runs'
      containers through the operator's Podman API socket
      (`[runs.container] podman_socket`, `ren_container::Reach::Socket`), whose
      service watches them apart from the daemon. The install's pause and
      backup talk to the running daemon with the build being installed.
    - Deviation: no pod. The daemon and the UI are each one container that
      a systemd unit runs in the foreground, on the host's network: the unit
      keeps the instance across boots, restarts, logs, and rollback as it
      did, and a pod would add nothing on the host's network.
    - Verified in the dev slot with `--contain all`: the daemon in its
      container started a run's container through the socket; the run
      outlived a restart of the daemon's container and took a second turn;
      stopping it removed its container. The daemon's container did not see
      production's configuration or data, the main checkout, or `~/.ssh`.
    - Production switched: its daemon runs as `ren-production-daemon`, and
      a real Codex run in a container saw only its folder and `.codex`.
    - Built: the UI in a container (`UiContainer`, xtask), as
      `ren-ui.service`'s foreground `podman run` under `--contain all`, or
      the dev slot's UI unit. It runs its deployed copy on the instance
      image's Node and sees only that copy and the control token file, both
      read-only.
    - Not yet: inside a container the operator's uid is named for the
      image's user (`node`); Claude Code, Pi, and opencode runs in
      containers have not been driven.

12. **A run's envelope: what it sees, reaches, holds, and runs on.**
    Planned. Phase 11 limits what a run sees; it still reaches everything
    (`--network=host`: the internet, and every listener on the host,
    Ren's own included), holds the operator's credentials (the SSH agent,
    its client's sign-in), and shares the host's kernel. An agent that is
    misled by what it reads can send what it sees anywhere and push
    anywhere the operator can. The common practice for coding agents is a
    run with no network but a proxy that admits listed destinations,
    credentials kept outside the run or scoped to it, and a boundary
    stronger than a container's. Ren already carries every model call
    through its gateway, so a run needs no other way out to its models.
    - **`ren-relay` (new), `Relay`: the run's way out for everything that
      is not a model call.** A forward proxy (HTTP `CONNECT`) that admits a
      connection when its `RelayPolicy` lists the destination, passes the
      bytes through unread (TLS stays end to end), and reports a
      `Passage` once it closes: the run, the destination, the `Verdict`,
      and the bytes each way. Sans-IO core (`RelayPolicy::decide`, a
      `Destination`'s host pattern and port, `Verdict::{Admitted,
      Refused}` with the refusal's reason); the listener is its shell.
      Standalone, on none of Ren's crates, like `ren-github`. It is not the
      gateway's: the gateway reads model protocols and records calls; the
      relay passes opaque tunnels and records only who went where.
    - **`ren-sandbox` (new), `Sandbox`: one run's envelope, declared.**
      What it sees (`Sight`: the mounts phase 11 derives), reaches
      (`Reach`: the gateway's address and the relay's policy), holds
      (`Grant`s), and runs on (`Isolation`). It replaces `RunContainers`
      and `RunNeeds` (`ren-daemon`'s `host/held/container.rs`) and builds
      the `ren_container::Container` a run starts in. Its constructor
      refuses what a run must never have: the host's network, Ren's data
      or configuration, the SSH agent. Pure; on `ren-container`,
      `ren-relay`, and `ren-harness`.
    - **Reach is declared, in two places.** `ren.toml`'s `[runs.reach]`
      lists what every run may reach (package registries, the source
      host), and a project's manifest (`ren-manifest`) adds what its runs
      need. Each `Destination` says why it is listed (`Purpose`), so a
      refusal and the page that shows it can say which list lacks it.
    - **A run's network is its own.** A run starts on a private network
      with no route out: its only peers are the gateway and the relay,
      listening for it (Podman's internal network, or `pasta` with no
      outbound route). The client's base URL is the gateway's address on
      that network, and `HTTPS_PROXY` the relay's. `ren-container`'s
      `Placement` gains the private network beside `Host` and `Pod`.
    - **Passages are recorded beside calls.** The daemon hands each
      `Passage` to the recorder, which stores it with the run; a run's page
      shows where it went and what it was refused, as a call's page shows
      what it asked. A refused destination is one the operator can add,
      not a silent failure.
    - **Grants, not the SSH agent.** A run pushes and opens pull requests
      with a `Grant`: a credential scoped to one repository, for a short
      time, minted for that run outside it and handed to git through a
      credential helper that asks over the run's socket. The SSH agent is
      no longer mounted. Minting a repository-scoped token needs a GitHub
      App installation (`ren-github`); a personal token cannot be scoped
      per run.
    - **`Isolation` chooses the boundary.** `ren-container` gains
      `Runtime` (`Crun`, `Runsc`, `Kata`), a closed enum that sets Podman's
      `--runtime`. gVisor (`runsc`) intercepts a run's system calls in a
      kernel of its own in user space and needs no KVM; Kata gives each run
      a microVM and needs KVM (the VPS has `/dev/kvm`). Containers stay the
      default until each client is verified under the stronger runtime.
      (Built as `Runtime`, gVisor only: see below.)
    - Built: `ren-relay`. A holder's pass is a `Ticket`, the password of
      its proxy URL (`HTTPS_PROXY=http://run:<ticket>@relay`), which curl,
      git, cargo, and npm send as `Proxy-Authorization`; the relay knows
      no run, only the holder it was given. A listed name reaches only
      public addresses (not this host, a private or shared network,
      Tailscale's included); an address is reached only when listed as
      itself. Anything but `CONNECT` is refused. Verified with curl: a
      listed registry answered through the relay, TLS end to end, and an
      unlisted host was refused and reported. Not yet wired into the
      daemon or a run.
    - Built: relayed runs. `[runs.container] network = "relayed"` starts
      a run's container with no network (`ren-container`'s
      `Placement::Isolated`); its run host opens a `Doorway` on its
      loopback for the gateway (at the gateway's own address, so the
      client's base URL is unchanged) and for the relay (`3128`), each
      reaching a Unix socket the daemon listens on in `<data>/runs/.reach`
      (`Doors`), mounted read-only. The daemon starts the relay on
      loopback and, per run, mints a ticket (`Relaying`), issues its pass,
      and gives the client `HTTPS_PROXY`, `HTTP_PROXY`, and `NO_PROXY`;
      the run's spec keeps the pass, so a daemon that takes the run up
      again issues it again. A run reaches `[runs.container] reach` and what
      its client needs (`ren-harness`'s `ClientNeeds`: what it keeps and
      where it connects itself). Passages are logged, refusals as
      warnings. The dev slot's contained runs are relayed.
    - Verified in the dev slot (`--contain all`): a run's container had
      only its loopback; the fake agent's turn reached the gateway through
      its doorway; through the relay it reached the listed registry, and
      unlisted hosts were refused and logged; directly, it reached neither
      the internet nor the host's other listeners. The run outlived a
      restart of the daemon's container, which issued its pass again: it
      reached the registry and took a second turn.
    - Built: passages recorded. The daemon hands each `Passage` the relay
      reports to the recorder (`Recorder::passage`), which records it as a
      run fact (`RunFact::Passed`, a `PassageRecord`: what was asked, how
      it ended as one `PassageEnd`, through which destination, and the
      bytes of one that closed); the store keeps it in `run_passages`
      (migration 0007), deleted with its run, and serves a run's passages
      grouped by what was asked and how it ended
      (`/api/runs/{id}/passages`). A session's inspector lists where its
      runs reached ("Reached"), refusals first. Refusals are still logged
      as warnings; what closed is only recorded.
    - Verified in the dev slot (`--contain all`): through a run's pass,
      the listed registry twice, two unlisted hosts, and a plain `GET`
      were recorded and read back grouped; the session's inspector listed
      the three refusals first and the registry with its count.
    - Built: grants. With `[github.app]` (an App's id, its key file, and
      the REST API), the daemon reads a relayed run's checkout's `origin`
      (`ren_git::Git::remote_url`, `RepositorySlug::of_remote`); a
      repository listed under `[[github.repository]]` is granted
      (`RelayedSpec.grant`, kept for a rejoin): the run may reach
      `github.com:443`, and its git is given `ren credential` as its
      helper for github.com (with the repository's path), the grants
      socket (`REN_GRANTS`, `<data>/runs/.reach/grants.sock`), and
      GitHub's SSH addresses read as HTTPS. The helper passes git's ask
      with the run's proxy URL; the daemon (`Relaying::serve_grants`)
      answers the run its ticket names with a token for its repository
      (`ren_github::Installations`: the App signs its own RS256 token, the
      repository's installation is looked up, and a token is minted for
      that repository alone, contents and pull requests writable), reused
      for 45 minutes (`Grants`); any other ask gets nothing. A relayed
      run's container never mounts the SSH agent. The daemon's container
      mounts the App's key read-only.
    - Verified in the dev slot (`--contain all`, a fake App on the fake
      provider): in a relayed run in `acme-api` (origin `acme/api`),
      `git credential fill` got the grant for `acme/api` and nothing for
      `acme/other`; `git ls-remote git@github.com:acme/api.git` went over
      HTTPS through the relay to GitHub with the grant (refused there, as
      the fake token is), recorded as a passage to `github.com:443`; the
      container had no `SSH_AUTH_SOCK`.
    - Deviation: the repository is read from the checkout's `origin`, not
      the session's project, and granted only when `[[github.repository]]`
      lists it: the run host's start knows the folder, not the project.
    - Deviation: shown on the session's page, not a run's: the UI shows a
      session with its runs, so the inspector merges its runs' passages.
    - Deviation: `[runs.container] reach`, not `[runs.reach]`: only a
      container's run is relayed, so what it reaches is the container's
      setting.
    - Not yet: grants need a GitHub App the operator creates and installs;
      none exists yet, so production's relayed runs cannot push. `gh` is
      not given a grant (git only), so a run cannot open a pull request
      itself yet. The watch still reads with `token_env`;
      Claude Code, Codex, Pi, and opencode have not been driven relayed,
      and what each connects to itself is declared from their sign-in
      flows, not observed.
    - Built: the runtime. `ren-container`'s `Runtime` (`HostKernel`,
      `Gvisor`) rides on `Placement::Isolated` alone, so neither a
      container on the host's network nor a pod's can take one; the
      daemon's `RunReach::Relayed` carries it, and
      `[runs.container] runtime = "gvisor"` without `network = "relayed"`
      is refused (`ConfigError::RunRuntimeNotRelayed`). Under gVisor a
      run's container starts with `runsc`, `--ignore-cgroups` (rootless, it
      cannot make Podman's cgroup), `--network=none` (its loopback; rootless
      it can take no network of the host's), and `--host-uds=all` (the run
      host serves the daemon on a socket in its folder, and reaches out
      through its doors). A daemon in a container starts runs through
      Podman's API socket, which cannot be told a runtime, so gVisor runs
      go through a Podman service of their own, `ren-gvisor` (xtask's
      `GvisorService`: socket-activated user units, `podman` started with
      `Engine::service_args`), which `install-vps --contain all` and the
      dev slot set up when gVisor is asked for; `podman_socket` names it.
      Before each start through a socket, `Engine::start` asks the service
      which runtime it runs (`podman info`) and refuses a container whose
      runtime differs (`EngineError::Runtime`). A run whose client cannot
      start is now logged with why. `cargo xtask dev up --runtime gvisor`
      runs the slot's contained runs under it.
    - Verified in the dev slot: with the daemon on the host (`--contain
      runs`), runs reported gVisor's kernel and took their turns; with the
      daemon in a container (`--contain all`) through `ren-gvisor`, ten
      runs started in two bursts of five all took their turns, each
      container's runtime `runsc`; a run's relay reached the listed
      registry, `example.com` was refused and recorded, nothing was reached
      directly, and `podman exec` from Podman's own service still worked; a
      run outlived a restart of the daemon's container and took a second
      turn.
    - Deviation: gVisor from its own apt repository (gVisor 20261116), not
      Ubuntu's `runsc` (20240729, built with Go 1.25): Ubuntu's panicked in
      its kernel (`assertTaskGoroutine`) on about one start in four, seen
      in its debug log.
    - Deviation: no Kata. Ubuntu does not package it, and it needs
      `/dev/kvm`, which the operator's user cannot open; a variant nothing
      can run is left out.
    - Deviation: gVisor only for relayed runs. A container's network could
      be given per container only by annotation, which needs
      `--allow-flag-override`, and then Podman's own commands on it (`exec`,
      from a service without that flag) are refused; relayed runs are the
      ones narrowed for a stronger runtime anyway.
    - Not yet: production's configuration does not ask for gVisor; real
      clients (Claude Code, Codex, Pi, opencode) driven under it.
    - Deviation: `Purpose` is not the relay's. The relay records the
      destination that admitted a tunnel; why it is listed belongs to where
      it is listed (`[runs.reach]`, a manifest), so `ren-sandbox` keeps it.
    - Order: the relay and the private network first (the largest gain,
      and the gateway already carries the model calls), then passages on
      the run's page (done), then grants (done), then the runtime (done). Each lands on its own
      and is verified in the dev slot before production.
    - Not decided: the client's own sign-in. A run mounts its client's
      sign-in folder read-write, so it holds a credential that outlives it.
      Keeping that credential outside the run means the gateway sets it on
      each model call, which breaks Ren's rule that clients sign in
      themselves; keeping the rule means a run can read its client's
      sign-in. Until this is decided, the envelope declares the client's
      folders as phase 11 does.

Phases 1 and 2 come first: without them, upgrading automatically only breaks
things faster. Each phase is useful on its own. Phases 7 and 8 do not depend
on each other; phase 9 needs both, and phase 10 needs only the canary.
Phase 12 needs phase 11: it narrows the containers phase 11 runs.

## Open questions

- Canary's real runs use the operator's client sign-ins on the same host.
  Ren never reads them, but whether two instances can drive one client
  sign-in at once is unchecked for each client.
- Allowance: how canary's real runs are budgeted, and whether a failure in
  them blocks promotion or only warns.
- Whether previews need real clients at all, or the fake provider is enough
  for the UI changes they are mostly for.
- Whether a run keeps its client's sign-in (phase 12): the gateway could
  set the client's credential on each model call so a run never holds it,
  at the cost of Ren's rule that clients sign in themselves.
- Whether production's runs move to gVisor (phase 12): gVisor and
  `ren-gvisor` are ready on the VPS, but each client is to be verified
  under it first, and production's runs are relayed first.

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
- **GitHub is read with one GraphQL query, not REST.** A repository's
  open pull requests, their merge state, review, and checks are one
  GraphQL query (`ren_github::Watch`), where REST needs a request per pull
  request for its merge state, reviews, and checks. GraphQL has no
  conditional requests, but one query a minute costs about 60 of the
  5000 points an hour a token has.
- **What was seen of GitHub is kept in memory, not stored.** The gateway's
  `PullBoard` holds each repository's last answer, or the failure since,
  and a restarted daemon polls again at once. GitHub stays the record of
  its own pull requests; Ren stores nothing it would have to reconcile.
- **A replica's pod shares the host's network.** `ren` listens only on
  loopback, and a rootless pod's published port reaches the container's
  own address, never its loopback. With the host's network, the pod keeps
  the loopback ports and tailnet routes a replica had; what it gains is
  its filesystem: it sees only the build it runs and its own data.
- **One image for every commit, the build mounted.** The plan had the
  image package each build. The image (`deploy/instance/Containerfile`)
  is Node, the C runtime, and CA certificates; a pod mounts the kept
  build read-only, so nothing is rebuilt per commit and a build stays the
  one the canary checked.
- **The dev slot keeps its units, for now.** It drives agent runs, each
  held by a run host in a systemd user unit of its own, against fake
  clients on the host; in a container, runs would need the isolated
  runtime this plan defers.
- **The gate is its own workflow.** It reads the pull request's title and
  body, so it runs again when they are edited, which would otherwise
  rerun all of CI.
