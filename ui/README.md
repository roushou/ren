# Ren UI

An optional SvelteKit 3, Svelte 5, and TypeScript dashboard for browsing traffic,
recordings, request/response payloads, and recorder health. It reads only the
daemon's read-only management API. It never reads SQLite or provider credentials,
and it never proxies inference: Claude Code, Codex, Pi, and OpenCode connect
directly to the Rust daemon. The daemon stays fully functional with the UI stopped.

```text
Claude Code / Codex / Pi / OpenCode ───> Rust daemon ──> Providers
Browser ──> optional SvelteKit server ──> Rust management API
```

Pages: Overview (`/`: one line that says how it stands, edged by the worst thing in it;
what needs attention in the last day, worst first, with repeats folded into a count:
exposed secrets, recording trouble, failed and cut-off model calls, then failed
bookkeeping such as quota probes, which is not the agents' work; the recordings active in
the last 15 minutes in the Live page's state colours; the day's calls (against the day
before), failed work calls, input tokens cached and fresh, and output tokens; and input
per hour with failed and cut-off calls ticked along the same hours, filling the rest of
the window; with nothing recorded in the last day it leads with how to connect a client), Live
(`/live`: the recordings active in the last 15 minutes as they work (or 5 minutes, an
hour, or four hours, chosen beside the active count; a window holding more than the page
reads at once is marked partial), on a four-by-two board
of cards or as rows: each recording's client and provider, its state (shown by a coloured edge down the card and
a glyph of its own: responding, waiting for a response, the client's turn, your turn when
a call finished its turn without asking for tools, idle, failed, warning, or unknown), and its current
turn's prompt, replies, and tool calls (each with what it touched, its result, and the
first line it returned; like calls in a row merge into one line with a count, and commands
drop the `cd` into the turn's directory), with what the turn read, wrote, and ran. A
card's header links to its recording; hovering it shows the card's actions: open the turn,
open the current call, pin the card so it stays on the board while its recording is active
(`?pin=`), and expand it (`?focus=`) over four slots with every call of the turn and its
tools' input and output (`Escape` closes it). This is below a band of
output tokens over the last 10 minutes, grouped by client, provider, or model (`?by=`):
one chart of fixed height that draws only what had output, the biggest four on their own
and the rest merged, stacked so its top edge is the total (**Ribbon**, the default) or
each in its own lane (**Area**). Its legend sits inside the chart; hovering a series
outlines its cards, and selecting it shows only its recordings (`?series=`). Then comes a **Now** strip of what happened
lately, newest first: new recordings, prompts, finished calls, failed tools, bad endings,
and rate limits, with calls still waiting for a response head kept first. Pinned recordings, then those with a call open, then the most recent, get
the eight cards; quiet ones dim. While a call streams, its card shows the end of its
reply so far, the tools it has begun, and whether the model is thinking (never stored); a
client that reports no turns shows only that. The full text appears once the call finishes
and its activity is derived),
Traffic (`/traffic`; wherever a recording appears, there and on a recording's page, in Search,
and in Usage, it is named by its client, its newest prompt, and its folder), Recordings (`/recordings`: each recording's client, its newest prompt, and its folder, what it
is doing now, and its size against the rest, grouped by day, with single-call recordings
that have no turn folded away; `/recordings/:id`, each recording's context and
cache report at `/recordings/:id/report` (with each turn's context drawn oldest to newest,
from where it started to where it ended), and what a turn did at `/turns/:id`: its prompt,
tool calls with their effects and reported results, files, commands (one row each,
opening to the whole command), and findings, with paths shown relative to the folder the
turn works in), Search
(`/search`: full-text search over prompts, replies, and tool input and output), Usage
(`/usage`: calls, failed work calls (with failed bookkeeping counted apart), and tokens
over the last day by hour, the last week, or all retained calls by local day, by model,
and the busiest recordings), and Recorder
(`/recorder`: whether recording is healthy, how close storage, calls, and history are to
their retention limits with the nearest marked as the one pruned for first, and only the
loss and error counters that are not zero; bytes past the capture policy are noted as
left out by design, never as loss, here, in the Recorder tab, and in the footer). The header names where the gateway runs
(its node and network, such as `ren · tailnet`, with the API's status dot) rather
than its address, and opens each client's `ren run` command, ready to copy. Addresses
there show with their identifying part masked (a tailnet's name, a public domain)
until the eye button reveals them for the page; copying always copies the real one. The bar under the tabs shows each page's scope and the controls that
change it, such as Traffic's client, recording, time, and model-calls-only narrowing;
when they don't fit on one line they fold behind a **Scope** button. Traffic shows
model calls only by default, hiding token counts, model listings, and WebSocket
handshakes until that switch is off. On Traffic and a recording page, `j` and `k` select
the next and previous call, `/` focuses search, and `Escape` closes the inspector. The selected call (`?call=`) and the filters live in the URL,
so links, reloads, and Back/Forward work. With **Live updates** on, the UI follows the
daemon's `/api/live` feed and refreshes when calls change. The UI requires management
API version 1 and shows an explicit mismatch error for any other version.

## Develop

Use Bun (version pinned in `package.json`) and Node 22.17+. Start the daemon
separately with `cargo run --locked -- serve`.

```sh
cd ui
bun install --frozen-lockfile
bun run dev
```

Open http://127.0.0.1:5173. Development and production run the same management
handlers; there is no separate Vite proxy.

## Build and run

```sh
bun run build
bun run start
```

Open http://127.0.0.1:3000. The [Node adapter](https://svelte.dev/docs/kit/adapter-node)
produces `build/`. Cargo never builds or embeds the UI.

| Variable                 | Default                          | Purpose                                                                         |
| ------------------------ | -------------------------------- | ------------------------------------------------------------------------------- |
| `REN_GATEWAY_URL`        | `http://127.0.0.1:8787`          | Daemon origin reached by the UI server                                          |
| `REN_PUBLIC_GATEWAY_URL` | Same as `REN_GATEWAY_URL`        | Daemon origin displayed in the header and copied client commands                |
| `REN_DEV_TRAFFIC_URL`    | Unset                            | Dev slot only: loopback origin of its traffic controls, for the dev toolbar     |
| `REN_CONTROL_TOKEN_FILE` | Unset (read-only)                | The daemon's `control.token` (absolute path); set, the Runs view drives runs    |
| `REN_CONTROL_LOGINS`     | Unset (anyone reaching the UI)   | Comma-separated Tailscale logins allowed to drive runs; set it behind Serve     |
| `REN_SOURCE_DIR`         | Unset (Redline cannot send)      | Ren's source checkout (absolute path, under `[runs]` roots), for Redline        |
| `HOST`                   | `127.0.0.1` with `bun run start` | UI listener address                                                             |
| `PORT`                   | `3000` with `bun run start`      | UI listener port                                                                |
| `ORIGIN`                 | Inferred from incoming request   | Set to the UI's external origin behind Tailscale Serve or another reverse proxy |

Gateway URLs must be HTTP(S) origins, without credentials, paths, queries, or
fragments, and must never point at the UI itself. `src/env.ts` validates them.
`.env` files work in development only: `node build` doesn't load them, so export
the variables or use a service environment file.

To inspect a VPS daemon from a UI running on your laptop:

```sh
REN_GATEWAY_URL=https://your-vps.your-tailnet.ts.net:8443 bun run start
```

The browser talks only to the UI origin. The UI server relays allowlisted GET/HEAD
management requests to the daemon. It forwards no browser cookies or credentials
and doesn't follow redirects. The UI has no authentication of its own: keep it on
loopback or behind private Tailscale access, like the daemon.

## Sessions

The Sessions view (`/sessions`) starts and drives sessions (Codex, Claude Code, or Pi in a folder
under the daemon's `[runs]` roots; see [sessions](../docs/api.md#sessions)): a sidebar of
folders and their sessions, each folder's `+` making one there at once, the conversation (its
recorded turns, then what streams in), its approvals with what each asks, and a prompt box whose
bar holds its settings (and, for a draft, its client). `/runs` links lead here. History stays read-only; runs
are the one thing the UI changes, and only when the UI server is given the
daemon's control token file:

```ini
REN_CONTROL_TOKEN_FILE=/home/you/.local/state/ren/control.token
REN_CONTROL_LOGINS=you@example.com
```

The UI server reads the token from the file on each request (the daemon writes a
new one when it starts), so it never reaches the browser, and it forwards only
`/control/runs*` to the daemon's `/control/v1/runs*`: list, start, prompt,
answer, interrupt, stop, and the update feed. A request must come from the UI's
own pages (`Sec-Fetch-Site: same-origin`, or an `Origin` naming this host, and a
JSON body), and with `REN_CONTROL_LOGINS` set, carry one of those logins in the
`Tailscale-User-Login` header Tailscale Serve adds. Without it, anyone who
reaches the UI can drive runs: leave it unset only for a UI on loopback. Without
`REN_CONTROL_TOKEN_FILE`, the view lists recorded runs and explains how to turn
it on.

### Redline

With the control token, every page has Redline (the status line's Redline
button, or `m`): mark what's wrong on the page for an agent to fix. While it is
on, the element under the pointer is outlined and named, a click marks it
instead of reaching the page, and you write a note in place; `Esc` leaves, and
the marks stay. A pane floating over everything (drag it by its header, or move
it with the arrow keys there; this browser keeps where you left it) shows the
brief exactly as the agent gets it: each note with the element's selector (from the nearest labelled
region), its text and size, and its source file and line when the UI runs as a
development build (`bun run dev`; a production build has none), and the commit
the daemon serving the page was built from. Send makes a session on the client
you choose, with the brief as its first prompt, and opens it. The session
always works in Ren's own source checkout, `REN_SOURCE_DIR`, since the marks
are on Ren's pages; `cargo xtask install-vps` sets it to the checkout it
installs from, and without it Redline marks but cannot send. The checkout must
be under the daemon's `[runs]` roots. The brief asks the agent to work on a new
`redline/…` branch in a git worktree and not to install anything; nothing
enforces that yet beyond the agent following it and the client's approvals.
The client last sent to is kept in this browser only.

## Payload previews

Payloads are rendered inert. Message text is formatted as Markdown (headings, lists,
code, tables, emphasis) by a small parser in `src/lib/markdown.ts` that builds
elements, never HTML: raw HTML stays text, links show their address but are not
clickable, and images are named but never loaded. Agent context wrapped in tags such
as `<system-reminder>` is folded under its name. Message cards are colored by role
(user, assistant, thinking, tool call or result, system), and context that repeats on
every call (system prompt, tool list) starts folded. Payloads stay in browser memory
and are never written to
browser storage; only the theme preference is stored. Partial, active, corrupt, and
expired captures are marked explicitly; a body cut at the daemon's per-body capture limit
reads as trimmed, in grey, not as loss. A parsed message doesn't prove model
completion or tool execution.

Preview limits (these limit display only, not forwarding or recording):

- At most 256 KiB and 4,096 contiguous chunks per direction, stopping at the first gap.
- Gzip, Zstandard, and Brotli decoding up to 512 KiB of output (8 MiB Zstandard
  window), with at most two concurrent preview tasks.
- Hex view up to 16 KiB. Readable view up to 400 cards, 512 KiB, and 2,000 SSE events.

Use `ren inspect --body … --output …` for complete captured bytes.

## VPS service

From a checkout on the VPS, `cargo xtask install-vps --https-port 8443 --ui-https-port 8444`
performs the steps below and upgrades the UI along with the daemon (see
[deployment](../docs/deployment.md#install-task)). To do it by hand:

The UI service is independent of the daemon service. The VPS needs Node only if the
UI runs there. Build in a checkout with Bun and Node, then deploy `build/`,
`package.json`, and `bun.lock` together to `~/.local/share/ren-ui`. Install
dependencies there with `bun install --production --frozen-lockfile --ignore-scripts`.
Don't deploy `.env` files, or `node_modules` built on another platform. Node must be
at the absolute path used in the unit, typically `/usr/bin/node`.

Install [`deploy/ren-ui.service`](../deploy/ren-ui.service) as
`~/.config/systemd/user/ren-ui.service` and create `~/.config/ren/ui.env`:

```ini
REN_GATEWAY_URL=http://127.0.0.1:8787
REN_PUBLIC_GATEWAY_URL=https://your-vps.your-tailnet.ts.net:8443
ORIGIN=https://your-vps.your-tailnet.ts.net:8444
```

Start it and publish it on a separate **unused** Tailscale Serve port:

```sh
systemctl --user daemon-reload
systemctl --user enable --now ren-ui.service
curl --fail http://127.0.0.1:3000/health
tailscale serve status
tailscale serve --bg --https=8444 http://127.0.0.1:3000
```

Clients keep using the daemon's port (8443). Preserve existing Serve routes, and
keep both ports private (never Funnel). For lingering, follow the
[daemon service instructions](../docs/deployment.md#vps-with-tailscale). Logs:
`journalctl --user -u ren-ui.service`. To upgrade, stop only `ren-ui.service`,
replace the build and dependencies, and start it again. To remove it, disable the
service and run `tailscale serve --https=8444 off`. Forwarding and recording are
unaffected either way.

## Verify

```sh
bun run format:check
bun run check
bun run test
bun run build
```

`tests/*.test.ts` holds focused unit tests that Node runs directly. There is no
browser integration suite: verify UI changes with synthetic traffic in the dev slot
(`cargo xtask dev up --ui`, `dev seed`, `dev shot`; see
[Deployment](../docs/deployment.md#dev-slot)).
