# Plan: hierarchy

Ren's domain has no hierarchy, so its UI has none either. The topbar has 11
links (Overview, Live, Sessions, Traffic, Recordings, Search, Usage, Tools,
Work, Release, Recorder), each a flat list of one kind of record, because
the API serves flat lists and nothing in the model says what owns what. The
factory built in [self-build.md](self-build.md) runs a loop (a project's
work is queued, sent to an agent, previewed, landed, checked, and promoted),
but no record names the project the loop is for, so the loop cannot be
shown as one.

This plan adds that record and the links under it, then the API that serves
the tree. The UI is redesigned on top of it, as its last phase. Each phase
says whether it is built. Record every deviation here, with its reason.

The principle: **every record Ren keeps either belongs to a project, or is
explicitly unowned and says why.** Nesting in the UI follows ownership in
the data. It is never invented in the view.

## What Ren keeps today

| Record                | Where                                                     | Owned by                                                                                |
| --------------------- | --------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Operator session      | `operator_sessions`                                       | nothing: it keeps a folder (`workdir`) and a client                                     |
| Run                   | `runs`                                                    | its session (`operator_session`)                                                        |
| Recording, turn, call | `recordings`, `turns`, `calls`                            | a client's conversation (`native_id`), which a run's `thread` names when Ren started it |
| Work item             | `work_items`                                              | nothing; once sent, it names the session that took it up (`operator_session`)           |
| Ren's release         | `<data>/release.json`, written by `cargo xtask canary`    | nothing: there is one                                                                   |
| A project's release   | `<data>/projects/<folder>/release.json`, by `ren project` | a repository's folder name                                                              |
| A project's contract  | `.ren/project.toml` in its repository                     | the repository                                                                          |
| Toolset               | `<data>/tools`                                            | the host                                                                                |

So the links that exist are **run → session** and **work item → session**,
and **call → run** through the client's conversation. Nothing points up from
a session, and a "project" exists only as a folder name the CLI is handed
(`--dir`). Two repositories with the same folder name share a release
folder. Ren's own release takes a separate code path and has its own tab,
although Ren is the first project the factory builds.

## The tree

```text
Project                       a repository Ren builds (Ren is one)
 ├─ Release                   main, the canary's verdict, production, the decision
 ├─ Worktree                 a worktree of it, with its slot's ports and data
 └─ Work item                 queued → sent → landed → shipped (or dismissed)
     └─ Session               the agent conversation that does it
         └─ Run               one client process
             └─ Turn → Call   what the gateway recorded
```

Records outside the tree stay, and each says why it is outside:

- **A scratch session** (a folder of its own, outside any project) belongs
  to no project. It stays a session, listed with the sessions that have none.
- **Traffic from clients Ren did not start** (a `claude` pointed at the
  gateway by hand) belongs to no run. The gateway exists for it, so it is
  first-class, under Inspect, never forced into a project.
- **The toolset, usage, and the recorder's health** belong to the host.

## Decisions

1. **A project is a stored, registered record, not a folder name.** The new
   `projects` table holds one row per project: a `ProjectId` (its own type),
   a unique `ProjectName` (a slug the operator picks, which names it in the
   CLI and in URLs), the repository's canonical absolute path (unique too),
   the remote and branch its canary follows (`origin`, `main` unless given),
   its factory, and `created_at` / `archived_at`. It is registered
   explicitly (`ren project add --dir <repo> [--name]`,
   `POST /control/v1/projects`), never inferred from a folder a session
   happens to open.
2. **How a project is built is a closed enum, `Factory`.** `Contract` reads
   `.ren/project.toml` and runs `ren project canary`. `Ren` is Ren itself:
   its canary is `cargo xtask canary` against a replica of production's
   data, which a contract cannot express yet, and it is promoted by
   `install-vps`. Each variant owns where its release is kept: `Contract`
   in `<data>/projects/<name>/`, keyed by the registered name rather than a
   folder name; `Ren` where it already is (`<data>/release.json`), so
   production's file does not move.
3. **Ownership points up, once, and is set when the record is made.**
   - `operator_sessions.project`: a session made in a project's repository,
     or in a worktree of it (`<repo>-<slot>` of that repository), names
     the project, resolved when the session is created. A session may also
     be asked for by project (`"project": "<name>"`, a worktree of it)
     rather than by folder. A scratch session has none.
   - `work_items.project`: queueing a work item names its project. Redline
     marks pages of Ren's UI, so it defaults to Ren, and a page marked on a
     project's preview names that project. Sending it opens its session in
     that project.
   - Runs, turns, and calls get no new column: a run's project is its
     session's, and a call's run is the one whose `thread` names its
     conversation (a recording's `native_id`), as the store's activity reader already joins.
4. **Existing rows stay unowned, and are shown as unowned.** The migration
   cannot know which project an old session or work item was for, so both
   columns are nullable. A null reads as `Unassigned`, a typed state of its
   own, never rounded up to Ren. `ren project add` offers to assign a
   project's unassigned sessions whose folder is in its repository or one
   of its worktrees, since the folder says so. Work items are assigned by
   hand, because their marked page does not say which repository it was for.
5. **The API serves the tree, with collections under a project.** The flat
   reads stay, for Inspect, and gain a project filter. Project-scoped reads
   are added. `/api/release` and `/api/release/projects` fold into a
   project's own release read. This is a declared break: the UI and CLI
   change with it.
6. **The UI follows the tree.** There are three top-level destinations,
   with depth under each. The navigation is decided from the operator's
   tasks against this data, and rendered on dev-slot data before review. It
   is not drawn here.

| Today's link                                | Becomes                                                                                             |
| ------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| Overview                                    | **Projects**: each project, what is in flight, and what waits on the operator                       |
| Work, Release                               | a project's work items and release                                                                  |
| Sessions                                    | a work item's or project's sessions; sessions with no project are listed apart                      |
| Live, Traffic, Recordings, Search, Recorder | **Inspect**: one place over recorded calls, opened already scoped from a session, run, or work item |
| Tools, Usage, the recorder's health         | **Host**                                                                                            |

## Phases

1. **The project record.** Built.
   - Ownership of the types: `ProjectId` joins the other identities in
     `ren-recorder` (`id`). `ProjectName`, `Factory`, and the stored
     project go in a `ren-recorder` `project` module beside `work`, since
     the store and gateway consume them. `ren-project` stays the contract.
     Labels (`factory`: `ren`, `contract`) are pinned in `labels.txt`.
   - Migration `0004_projects.sql` adds `projects`. It only adds, so the
     compatibility gate passes without a declaration. Its fixture is pinned
     with the others.
   - `ren project add | list | archive`, and `POST`/`DELETE
     /control/v1/projects`. `ren project canary | promote | release` take
     `--project <name>` and resolve it through the registry, replacing
     `--dir`. A contract project's release folder moves from its folder
     name to its registered name. Only dev-slot data has one today, so no
     fallback is kept.
   - `install-vps` registers its checkout as the `Ren` project, named `ren`,
     if no project has that repository.
2. **Ownership.** Built.
   - Migration `0005_ownership.sql` adds nullable
     `operator_sessions.project` and `work_items.project`, with indexes.
   - Session creation resolves the project from its folder (decision 3) and
     accepts `"project"`. Queueing a work item takes `project`, defaulting
     to Ren from Redline. Sending checks that the session belongs to the
     same project.
   - `ren project add` offers the assignment in decision 4.
3. **The API.** Built.
   - Reads: `ReadRequest::Projects` and `ReadRequest::Project`. A project
     view holds its work items counted by state, its release summary, its
     sessions with a run starting or working, and the worktrees they work in.
     `WorkItems`, sessions, and `Runs` gain a project filter.
   - `/api/projects/{name}/release` replaces both release reads, as a
     declared break (`feat!` and a `BREAKING CHANGE:` footer). The Release
     view and `ren release` read it.
   - Wire types are generated for the UI as today.
4. **The UI.** Under way. The audit:

   | The operator's task                     | Today                                       | In the tree                                                                  |
   | --------------------------------------- | ------------------------------------------- | ---------------------------------------------------------------------------- |
   | See what is in flight and what waits    | nowhere: Overview is the gateway's traffic  | **Projects** (home): each project's work, runs, and release; what is pending |
   | Mark what is wrong (Redline)            | any page                                    | any page; the item is queued for Ren's project                               |
   | Send work, check a preview, see it land | Work                                        | Work, under Projects; a project's page lists its items by state              |
   | Watch and steer an agent                | Sessions                                    | Sessions, under Projects; a project's page lists its runs                    |
   | Check a canary and promote              | Release (Ren's); the CLI for other projects | Release (Ren's), and every project's release on its page                     |
   | Find out why a call or run went wrong   | Live, Traffic, Recordings, Search, Overview | **Inspect**: those five                                                      |
   | Keep the host                           | Tools, Usage, Recorder                      | **Host**: those three                                                        |

   Built in this round: the header names Projects, Inspect, and Host, and
   a row under it lists the current one's pages (`lib/view.ts`,
   `Views.destination` and `Views.pages`). Projects (`/`) lists each
   project with its release, open work, and runs, beside what is pending:
   a canary to promote, a preview to check, work still queued. A project's
   page (`/projects/{name}`) shows its release (lanes and checks), its
   work by state, and its runs, each opening in Work, Sessions, or
   Release. Overview, the gateway's traffic, moves to `/overview` under
   Inspect.

   Approvals are among what is pending: when this UI drives sessions, the
   Projects view reads the daemon's live runs (`/control/v1/runs`) and the
   current sessions, and lists each approval a working run waits on with
   its project and the session's first prompt, to approve, decline, or
   open. A UI that only reads shows none.

   A work item opens in a drawer over its project's page (`?item=` in the
   address: Escape, the scrim, or Back close it), with the same detail the
   Work view shows beside its list (`WorkItemDetail`).

   A work item's brief names its own project: Ren's (and every item from
   before projects) names its route's file and build and previews with
   `cargo xtask preview`; another project's says whose page it is and
   previews with `ren project preview`. Work sends a batch to a new session
   in its project's repository (`"project"` on session create), one
   project's items at a time.

   Inspect is scoped to a project by `?project=` on Traffic and
   Recordings (`/api/calls` and `/api/recordings` take `project`: a call is
   a project's when its recording's conversation is the thread of a run of
   a session working for it); a row under the pages names the scope and
   clears it, and switching between those two keeps it. A project's page
   links its calls and recordings.

   A project's page lists its current sessions, each with the worktree it
   works in (or its folder), the worktree's ports, whether it runs, and
   the work items it took up, which open in the drawer. A worktree is a
   property of its session (`worktree` on a session body), not a list of
   its own: a project holds sessions, and a session works in a worktree.
   Deviation: the plan named them the project's workspaces; "workspace"
   usually means something bigger than a project, so the concept is
   renamed to worktree, in its own change after this one.

Each phase is verified in the dev slot, whose fake projects are registered
as projects (and the slot's checkout as `Ren`), and by the compatibility
gate against `main`.

## Open questions

- **Ren's canary as a contract.** `Factory::Ren` exists because the
  contract cannot say "check against a replica of production's data".
  Teaching it that, so Ren is a `Contract` project like any other, would
  remove the variant. It is not needed for the tree.
- **Where sessions with no project live.** Sessions with no project are
  listed apart. Whether that list sits under Inspect or Host is decided by
  the UI audit.
- **One repository per project.** A project spanning several repositories,
  or several projects in one monorepo, is not modelled. The repository
  path is unique per project until a case asks otherwise.
- **Renames.** Whether a project's name can change, given that it keys its
  release folder and appears in URLs. It cannot change until a case asks.
- **Per-project usage.** Usage could be attributed to a project through
  run → session → project. Host keeps the global view until the UI audit
  asks for more.

## Deviations

- **Phase 1: the remote and branch a canary follows stay flags.** The doc
  had the project record hold them. They stay `ren project canary
  --remote` and `--branch`, so `ren-recorder` takes no dependency on
  `ren-git` for its `Branch` type; a project record can gain them when a
  timer runs canaries unattended.
- **Phase 1: the CLI reads projects from the store.** The read API serves
  projects in phase 3. Until then `ren project list`, and the `--project`
  lookups, read the store directly beside the running daemon, as other
  read-only commands can. Registering still goes through the control API.
- **Phase 1: adding a project registered just so is done.** `ren project
  add` with the same name, repository, and factory as a registered project
  succeeds without a request, so `install-vps` and the dev slot register on
  every run. A different project with the name or repository is refused.
- **Phase 1: registration is checked, then recorded.** Writes are
  recorded asynchronously, so the gateway checks the name and repository
  against the store before it records one, and the store keeps the first
  project to take either (`ON CONFLICT DO NOTHING`) if two are accepted at
  once. `ren project add` waits for its project to be stored and fails if
  another took the name meanwhile.
- **Phase 1: "project" already named a place.** `GET
  /control/v1/runs/projects` lists the folders a session can start in, as
  the wire type `Project` (`ren-supervisor`'s `Project`): a place found
  under the configured roots, not a registered project. Renaming it would
  change the API, so phase 1 names its new types apart
  (`RegisterProjectRequest`, `ProjectAnswer`); phase 2, where a session is
  asked for by project, renames that listing to places. Done in phase 2:
  `GET /control/v1/runs/places` answers `{places, scratch}` with `Place`
  and `PlaceGit` (`ren runs places`), a declared break.
- **Phase 2: a session's project is its own fact.** `SessionFact::Created`
  is recorded by the supervisor and by `ren run`'s tap alike; the project
  is a separate `SessionFact::Assigned`, recorded by the gateway right
  after (and when a draft moves, and on adoption), so the supervisor needs
  no project list. A session `ren run` makes is unassigned until adopted.
- **Phase 2: a queued item's project is a query parameter.**
  `WorkOrderBody` is what was marked, and is also read back as the item's
  `order`; the project is not part of it, so queueing takes
  `?project=<name>`.
- **Phase 2: a send is refused only when both projects are known and
  differ.** An unassigned item or session, from before projects or
  outside them, is sent as before.
- **Phase 2: adoption is its own command.** The doc had `ren project add`
  offer it; `add` says how many sessions in the repository have no
  project, and `ren project adopt` (`POST /control/v1/projects/{name}/adopt`)
  assigns them, with the project's worktrees matched by folder name
  (skipped when another project's repository shares it).
- **Phase 3: project reads are their own routes.** The doc had
  `ReadRequest::Projects` and `ReadRequest::Project`. A project's read
  joins the store with the release files the factory writes, which a store
  read does not reach, so `/api/projects`, `/api/projects/{name}`, and
  `/api/projects/{name}/release` are gateway routes, as the release reads
  were. The filters are store reads: `project` on `/api/runs` and
  `/api/work`, and on the control API's session list.
- **Phase 3: a project's activity is counts.** The project read holds its
  work items by state, its sessions not archived, and its runs not ended;
  the items, sessions, and runs themselves are the filtered lists, and its
  worktrees are its sessions' folders, so no separate list is kept.
- **Phase 3: `ren project list` reads the API; resolving stays local.**
  `list` reads `/api/projects` (and so works against a remote gateway).
  `--project` lookups for `canary` and `promote`, which build a repository
  on this host, still read the store beside the daemon. `ren release`
  takes `--project`, Ren's by default.
- **Phase 4: Work, Sessions, and Release stay pages under Projects.**
  Decision 6 put them inside a project. A project's page lists its work
  and runs and links into them, while the pages themselves still list
  every project's; moving each inside its project is a later round, from
  the real page, so nothing they do is lost on the way.
- **Phase 4: Overview stays, under Inspect.** It is the gateway's traffic
  at a glance, not the factory's, so `/` became Projects and Overview moved
  to `/overview`.
- **Phase 4: a project's page is one page of sections.** The mockups had
  tabs; with its release, work, and runs each short, sections read in one
  scroll, and the tabs wait until a section outgrows it.
