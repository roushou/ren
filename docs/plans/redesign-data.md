# Redesign: data the designs need

The redesign's mockups (the design canvas, "Pages, restyled" and "Flow")
draw some things Ren does not send the UI yet. This lists them page by
page, each with where the data would come from, so each can become its own
change. The structure already in code on `redesign` (glyphs, layers, the
top bar, Pending, ⌘K) needs none of these.

Each row is one of three kinds:

- **UI** — the data reaches the browser already; only the UI is missing.
- **Gather** — Ren has the data, but not in the response the page reads.
- **New** — Ren does not record or work it out today.

Rows marked _unverified_ were read from the mockups, not checked against
the code; check them before starting.

## Session desk

| What the design shows                   | Kind               | Where it would come from                                         |
| --------------------------------------- | ------------------ | ---------------------------------------------------------------- |
| Context and cache share per turn        | Gather             | `TurnMetrics`, which exists only for a session with a recording. |
| How long an approval waited             | New _(unverified)_ | No wait time is recorded with an approval.                       |
| Lines added and removed per file change | New _(unverified)_ |                                                                  |

## Pending (top bar)

| What the design shows        | Kind   | Where it would come from                                                                           |
| ---------------------------- | ------ | -------------------------------------------------------------------------------------------------- |
| A failing pull request check | Gather | The checks Ren polls for each project's pull requests; Pending reads only runs, work and releases. |
| A client that is signed out  | Gather | Each client's sign-in state, which the Tools and sign-in data know.                                |

## Live

| What the design shows                                  | Kind   | Where it would come from                                         |
| ------------------------------------------------------ | ------ | ---------------------------------------------------------------- |
| "Waits for you" and its approval on a recording's card | Gather | Approvals live on runs; Live would need them per recording.      |
| Short tool summaries ("4 hits", "+38 −12")             | New    | Worked out from each tool's result; some tools may not allow it. |

## Traffic

| What the design shows                  | Kind   | Where it would come from                                                                               |
| -------------------------------------- | ------ | ------------------------------------------------------------------------------------------------------ |
| Rate limits as remaining-of-limit bars | New    | `AttemptView.limit_headers` keeps the raw headers; each provider's would need parsing.                 |
| A timeline of every call in the range  | Gather | Calls carry their times, but the page reads one page of calls; a range read may need its own endpoint. |

## Recordings

| What the design shows                   | Kind | Where it would come from          |
| --------------------------------------- | ---- | --------------------------------- |
| When calls clustered within a recording | New  | Hourly call counts per recording. |

## Recording and its report

| What the design shows                              | Kind                  | Where it would come from                                                        |
| -------------------------------------------------- | --------------------- | ------------------------------------------------------------------------------- |
| Turn headings and the turn panel on the calls page | Gather                | The recording's report, loaded with the calls.                                  |
| The context chart over a long recording            | Gather                | Every call's time and tokens at once; today they come a page at a time.         |
| New input per turn, beside cache read and written  | Gather _(unverified)_ | Possibly in the report's turn numbers; otherwise summed from the calls.         |
| A model's context limit ("of 200k")                | New                   | Ren does not know model limits; the designs now say "model limit not reported". |

## Usage

| What the design shows                   | Kind   | Where it would come from                                                 |
| --------------------------------------- | ------ | ------------------------------------------------------------------------ |
| Cache share per model                   | UI     | `cache_read` is in the usage tokens; the page doesn't show it per model. |
| Status codes per hour (a burst of 529s) | Gather | Traffic has each attempt's status; usage's hourly totals don't.          |

## Search

| What the design shows                           | Kind   | Where it would come from                                   |
| ----------------------------------------------- | ------ | ---------------------------------------------------------- |
| A hit count for each field, not only Everything | Gather | One search per field, or counts per field from one search. |

## Release

| What the design shows                                 | Kind                  | Where it would come from                           |
| ----------------------------------------------------- | --------------------- | -------------------------------------------------- |
| When production started its build ("running since …") | New                   | `ReleaseView` has production's commit but no time. |
| Each work item's commit in the Work columns           | Gather _(unverified)_ |                                                    |

## Work item

| What the design shows                                                                      | Kind              | Where it would come from                                          |
| ------------------------------------------------------------------------------------------ | ----------------- | ----------------------------------------------------------------- |
| A mark drawn where it sat on the page                                                      | New               | `WorkMarkBody` keeps the mark's size, not its position.           |
| Its session's turns and cost, its pull request's checks, the preview's port and build time | Gather            | The session, pull request and preview data, loaded with the item. |
| Method and status of each read behind the page                                             | UI _(unverified)_ | Reads are stored as text; check the text includes the status.     |

## Tools

| What the design shows                            | Kind | Where it would come from                                              |
| ------------------------------------------------ | ---- | --------------------------------------------------------------------- |
| Disk used, and what cleaning up would free       | New  | Generations carry no sizes.                                           |
| The last change's result, after the page reloads | New  | A change's result comes back once, when applied; Ren doesn't keep it. |

## Built on the redesign branch

These rows were here and are now built:

- **Session desk:** a duration on each step, read from the stored entries' times.
- **Recordings:** each recording's report in a slide-over beside the list, read when it opens; the clients' share of requests over the list.
- **Tools:** the newest generations side by side, each package's release in each.

## Not covered here

Integrations (the pull request and GitHub watch pages) was designed against
the deleted `github-ui` branch and is not on `main`; its gaps belong with
that work when it returns.
