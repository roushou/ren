# TGV UI

An optional SvelteKit 3, Svelte 5, and TypeScript dashboard for browsing traffic,
sessions, request/response payloads, and recorder health. It reads only the
daemon's read-only management API. It never reads SQLite or provider credentials,
and it never proxies inference: Claude Code, Codex, and Pi connect directly to the
Rust daemon. The daemon stays fully functional with the UI stopped.

```text
Claude Code / Pi / Codex ───────────────> Rust daemon ──> Providers
Browser ──> optional SvelteKit server ──> Rust management API
```

Pages: Overview (`/`: a health line, calls in flight and sessions active in the last
15 minutes, what needs attention in the last day, and the last 24 hours' usage), Live
(`/live`: the sessions active in the last 15 minutes as they work (or 5 minutes, an
hour, or four hours, chosen beside the active count; a window holding more than the page
reads at once is marked partial), on a four-by-two board
of cards or as rows: each session's client and provider, its state, and its current
turn's prompt, tool calls, and replies, below a band of output tokens per client, drawn as a row each (**Area**) or stacked into
one chart whose top edge is the total (**Ribbon**), and a **Now** strip of what happened
lately, newest first: new sessions, prompts, finished calls, failed tools, bad endings,
and rate limits, with calls still waiting for a response head kept first. Sessions with a call open, then the most recent, get the
eight cards; quiet ones dim. Text appears once each call finishes and its activity is
derived; whether a call is waiting for its response head or streaming is shown as it
happens),
Traffic (`/traffic`), Sessions (`/sessions`, `/sessions/:id`, each session's context and
cache report at `/sessions/:id/report`, and what a turn did at `/turns/:id`: its prompt,
tool calls with their effects and reported results, files, commands, and findings), Search
(`/search`: full-text search over prompts, replies, and tool input and output), Usage
(`/usage`: calls, errors, and tokens over the last day by hour, the last week, or all
retained calls by local day, by model, and the busiest sessions), and Recorder
(`/recorder`). The bar under the tabs shows each page's scope and the controls that
change it, such as Traffic's client, session, time, and model-calls-only narrowing;
when they don't fit on one line they fold behind a **Scope** button. Traffic shows
model calls only by default, hiding token counts, model listings, and WebSocket
handshakes until that switch is off. On Traffic and a session page, `j` and `k` select
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
| `TGV_GATEWAY_URL`        | `http://127.0.0.1:8787`          | Daemon origin reached by the UI server                                          |
| `TGV_PUBLIC_GATEWAY_URL` | Same as `TGV_GATEWAY_URL`        | Daemon origin displayed in the header and copied client commands                |
| `HOST`                   | `127.0.0.1` with `bun run start` | UI listener address                                                             |
| `PORT`                   | `3000` with `bun run start`      | UI listener port                                                                |
| `ORIGIN`                 | Inferred from incoming request   | Set to the UI's external origin behind Tailscale Serve or another reverse proxy |

Gateway URLs must be HTTP(S) origins, without credentials, paths, queries, or
fragments, and must never point at the UI itself. `src/env.ts` validates them.
`.env` files work in development only: `node build` doesn't load them, so export
the variables or use a service environment file.

To inspect a VPS daemon from a UI running on your laptop:

```sh
TGV_GATEWAY_URL=https://your-vps.your-tailnet.ts.net:8443 bun run start
```

The browser talks only to the UI origin. The UI server relays allowlisted GET/HEAD
management requests to the daemon. It forwards no browser cookies or credentials
and doesn't follow redirects. The UI has no authentication of its own: keep it on
loopback or behind private Tailscale access, like the daemon.

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
expired captures are marked explicitly. A parsed message doesn't prove model
completion or tool execution.

Preview limits (these limit display only, not forwarding or recording):

- At most 256 KiB and 4,096 contiguous chunks per direction, stopping at the first gap.
- Gzip, Zstandard, and Brotli decoding up to 512 KiB of output (8 MiB Zstandard
  window), with at most two concurrent preview tasks.
- Hex view up to 16 KiB. Readable view up to 400 cards, 512 KiB, and 2,000 SSE events.

Use `tgv inspect --body … --output …` for complete captured bytes.

## VPS service

From a checkout on the VPS, `cargo xtask install-vps --https-port 8443 --ui-https-port 8444`
performs the steps below and upgrades the UI along with the daemon (see
[deployment](../docs/deployment.md#install-task)). To do it by hand:

The UI service is independent of the daemon service. The VPS needs Node only if the
UI runs there. Build in a checkout with Bun and Node, then deploy `build/`,
`package.json`, and `bun.lock` together to `~/.local/share/tgv-ui`. Install
dependencies there with `bun install --production --frozen-lockfile --ignore-scripts`.
Don't deploy `.env` files, or `node_modules` built on another platform. Node must be
at the absolute path used in the unit, typically `/usr/bin/node`.

Install [`deploy/tgv-ui.service`](../deploy/tgv-ui.service) as
`~/.config/systemd/user/tgv-ui.service` and create `~/.config/tgv/ui.env`:

```ini
TGV_GATEWAY_URL=http://127.0.0.1:8787
TGV_PUBLIC_GATEWAY_URL=https://your-vps.your-tailnet.ts.net:8443
ORIGIN=https://your-vps.your-tailnet.ts.net:8444
```

Start it and publish it on a separate **unused** Tailscale Serve port:

```sh
systemctl --user daemon-reload
systemctl --user enable --now tgv-ui.service
curl --fail http://127.0.0.1:3000/health
tailscale serve status
tailscale serve --bg --https=8444 http://127.0.0.1:3000
```

Clients keep using the daemon's port (8443). Preserve existing Serve routes, and
keep both ports private (never Funnel). For lingering, follow the
[daemon service instructions](../docs/deployment.md#vps-with-tailscale). Logs:
`journalctl --user -u tgv-ui.service`. To upgrade, stop only `tgv-ui.service`,
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
browser integration suite: verify UI changes manually with synthetic traffic through
a separately running daemon.
