# Deployment

Run Ren as a background service on macOS or Linux, and reach a VPS deployment
through Tailscale.

## Configuration

[`ren.example.toml`](../ren.example.toml) documents every key; unknown keys are
rejected. Data defaults to `$XDG_STATE_HOME/ren` when that variable is an absolute
path, otherwise `$HOME/.local/state/ren`. CLI `--data-dir` takes priority over the
configured path; relative configured paths resolve beside the config file, while
relative CLI paths resolve against the current directory. Configuration is
immutable until restart; every call records its configuration revision.

The daemon serves forwarding, `/health`, and the management APIs on one listener;
its root URL returns 404. It is headless: building, testing, and installing it need
no Bun, Node, or frontend assets, and stopping the [UI](../ui/README.md) does not
interrupt forwarding.

## Background services

`ren service render` generates a user-service definition for **launchd on macOS**
or **systemd on Linux**. It does not install, register, or start a service. Use a
stable installed binary, not a checkout's `target/debug/ren`. For example:

```sh
cargo install --locked --path crates/ren-cli --root "$HOME/.local"
(umask 077; mkdir -p "$HOME/.config/ren")
cp -n ren.example.toml "$HOME/.config/ren/ren.toml"
```

Review the configuration before starting. The commands below explicitly select
`~/.local/state/ren`; change `--data-dir` to use an existing capture directory.
Stop any foreground daemon using that listener or directory first.

### macOS: launchd

Create the log directory before loading the agent: launchd opens its log file
before Ren can create storage directories.

```sh
(umask 077; mkdir -p "$HOME/Library/LaunchAgents" "$HOME/.local/state/ren/logs")
"$HOME/.local/bin/ren" --config "$HOME/.config/ren/ren.toml" \
  --data-dir "$HOME/.local/state/ren" service render --manager launchd \
  --binary "$HOME/.local/bin/ren" \
  --output "$HOME/Library/LaunchAgents/dev.ren.gateway.plist"
plutil -lint "$HOME/Library/LaunchAgents/dev.ren.gateway.plist"
launchctl bootstrap "gui/$(id -u)" "$HOME/Library/LaunchAgents/dev.ren.gateway.plist"
"$HOME/.local/bin/ren" --config "$HOME/.config/ren/ren.toml" doctor
```

Status: `launchctl print "gui/$(id -u)/dev.ren.gateway"`.
Stop: `launchctl bootout "gui/$(id -u)/dev.ren.gateway"`.
Start again with `bootstrap` above. For a graceful restart, stop and then start;
avoid `kickstart -k`, which can interrupt capture. The agent starts at GUI login
and ends at logout; it is not an unattended boot-time system daemon.
Remove the plist after stopping to disable startup at future logins.

Logs go to `<data-dir>/logs/daemon.log`; `--log-dir` selects another existing
directory. `--label` changes the launchd label (name the plist to match).
Log files are not automatically rotated: stop the agent before rotating/removing
its log, then start it again. Capture retention does not limit daemon log size.

### Linux: systemd

Use systemd 240 or later with a user manager:

```sh
(umask 077; mkdir -p "$HOME/.config/systemd/user")
"$HOME/.local/bin/ren" --config "$HOME/.config/ren/ren.toml" \
  --data-dir "$HOME/.local/state/ren" service render --manager systemd \
  --binary "$HOME/.local/bin/ren" \
  --output "$HOME/.config/systemd/user/ren.service"
systemd-analyze --user verify "$HOME/.config/systemd/user/ren.service"
systemctl --user daemon-reload
systemctl --user enable --now ren.service
"$HOME/.local/bin/ren" --config "$HOME/.config/ren/ren.toml" doctor
```

Use `systemctl --user status ren`, `stop ren`, or `restart ren` for lifecycle
management. Read logs with `journalctl --user -u ren`; journal policy controls
their retention. To disable startup, use `systemctl --user disable --now ren`.
For unattended VPS operation after logout, enable user lingering with
`loginctl enable-linger "$USER"` (host policy may require an administrator).
Startup at boot relies on the user unit plus lingering; a host reboot has not been
exercised. The static [`deploy/ren.service`](../deploy/ren.service) is an
alternative for the standard paths above. Check lingering with
`loginctl show-user "$USER" -p Linger`. The VPS needs only Ren and Tailscale;
install and sign in to coding clients on the machine where you use them. Provider
credentials are not installed on the VPS.

Run the isolated Linux service check before installing or upgrading:

```sh
cargo build --release --locked
python3 scripts/smoke-systemd.py --binary target/release/ren
```

It requires a non-root user with a working systemd user manager. It renders and
verifies a uniquely named runtime unit, tests crash restart and all three provider
routes against local mocks, stops gracefully, then checks private storage and
history persistence across restart. It removes its unit and temporary state without
changing `ren.service`, lingering, Tailscale, or provider credentials by default.
To also exercise the HTTPS proxy with fixture credentials, pass `--https-port 8443`
before that port is used by the persistent deployment. This optional check adds a
Serve route only on an unused port, routes mock requests through HTTPS, checks that
proxy identity headers do not reach the upstream, and removes that route afterward.
Run it as the existing Tailscale operator, or add `--sudo-tailscale` to use
non-interactive sudo only for Serve configuration. It never changes operator
permissions. Other Serve routes remain untouched.

### Service behavior and upgrades

Both definitions restart after failure, run without a shell, use owner-only file
creation permissions, disable core dumps, and allow 30 seconds for SIGTERM shutdown.
The daemon lets running calls continue for 10 seconds, cancels what remains,
then has a separate 10-second budget to drain the recorder.
Systemd limits repeated startup failures to five in a minute; fix the cause and
run `systemctl --user reset-failed ren` before restarting if it hits that limit.
Launchd throttles respawns to at most once every 10 seconds. Neither manager's
"running" status proves recording health: use `ren doctor` after starting.

The renderer validates configuration and the executable, resolves paths to absolute
paths, and fixes the data directory through a CLI override. It rejects an ephemeral
listen port. Config files remain referenced, so ordinary settings take effect on
restart; a changed data-directory setting requires regenerating the definition.
No shell environment, provider credentials, or configuration contents are embedded.
It does not alter Tailscale, firewall rules, or listener exposure.

Omit `--output` to review the definition on stdout. With `--output`, Ren publishes
a complete synced file with mode 0600 on Unix and refuses existing files or symlinks.
Parent directories must already exist. For updates, render to a new file, stop the
service, replace the old definition, and reload/start it. Stop before replacing
the binary too; keep your configuration and capture directory. Service removal
does not delete captures.

The optional macOS smoke test uses a unique temporary launchd label, a local mock
provider, and temporary storage. It checks startup, crash restart, forwarding, and
graceful capture drain, then removes the job. It requires a logged-in GUI session:

```sh
cargo build --locked
python3 scripts/smoke-launchd.py
```

References: [Apple launchd jobs](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html),
the local `launchd.plist(5)` and `launchctl(1)` manuals, and
[systemd service semantics](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html).

## VPS with Tailscale

### Install task

From a checkout on the VPS, as the user that owns the service, with Rust and a
connected Tailscale:

```sh
cargo xtask install-vps --https-port 8443 --ui-https-port 8444
```

It runs the steps below and is safe to repeat: run it again from the commit to
deploy to upgrade. After a successful run it saves its publishing flags
(`--https-port`, `--ui-https-port`, `--sudo-tailscale`) to
`~/.config/ren/install-vps.toml`, so later runs need no flags:
`cargo xtask install-vps`. Giving any publishing flag replaces the saved set. It refuses a checkout whose tracked files differ from the
commit (`--allow-dirty` installs a `-dirty` build) and warns when the commit is on
no remote branch. It builds the release binary (with `ren tui`) while the old daemon keeps
serving, creates `~/.config/ren/ren.toml` only if missing, and checks the
configuration with Ren's own parser before stopping anything.

It then replaces a running daemon only between calls and between runs' turns. It
waits until no call is running and no [run](api.md#runs) is starting, working
(also while it waits for an approval), or stopping, naming each run it waits for
(up to `--idle-timeout`, 900 seconds by default, then stops without changing
anything). `--drain-runs` restarts without waiting for runs: their turns are lost
and their processes end with the daemon; running calls are still waited for. It then pauses
admission so new calls get `503` with `retry-after: 10` and clients wait (a run
prompted between the check and the pause is waited for too). Idle runs end with
the old daemon: before stopping it, it lists each with how to continue
its conversation (from the UI's Runs page, `ren tui`, or
`ren runs start <client> <dir> --resume <thread>`), which a new run resumes where
it left off. It runs `ren control backup`, stops the service, keeps the old
binary as `~/.local/bin/ren.previous`, installs the new one, rewrites the unit
only if it changed, starts the service, enables lingering, and runs `ren doctor`.
It requires doctor's build check to confirm the daemon runs the build just
installed, then resumes admission, also when a later step failed. A daemon an
operator had already paused stays paused. With `--https-port`, it adds a Serve
route only when that port is free, accepts one that already proxies to the
configured listener, and refuses a port used by anything else or exposed through
Funnel. `--sudo-tailscale` uses `sudo -n` for that step; `--skip-backup` skips the
backup. On a failed `doctor` it stops with the rollback instructions. With no
`--https-port` given or saved, it leaves Tailscale untouched. Review the configuration after the
first run.

With `--ui-https-port` (which needs `--https-port`), it also deploys the
[UI](../ui/README.md#vps-service). It needs Bun and Node on `PATH`. Before
stopping anything, it builds `ui/` and stages the build with its production
dependencies in `~/.local/share/ren-ui.next`. After the daemon is upgraded, it
writes `~/.config/ren/ui.env` only if missing (the daemon on loopback, and both
Tailscale URLs), stops `ren-ui.service`, keeps the deployed copy as
`ren-ui.previous`, swaps in the new one, installs the unit from
`deploy/ren-ui.service` with the Node found on `PATH`, starts it, and checks the
UI's `/health`, which relays the daemon's. It then publishes the UI on that port
under the same rules as the daemon's port. With no `--ui-https-port` given or saved,
the UI is left untouched.

### Dev slot

`cargo xtask dev` runs this checkout's build beside the installed service, with
fake providers and fake data, for development and for previewing a change before
it is installed:

```sh
cargo xtask dev up --ui      # build, then start the slot and its UI, and publish them
cargo xtask dev seed         # play a scenario once (sessions, users, long, errors, websocket, all)
cargo xtask dev traffic      # show the live traffic settings
cargo xtask dev traffic set --rate 60 --pace slow --replies tools=50,error=5
cargo xtask dev traffic pause    # or resume; burst 20 sends 20 calls now
cargo xtask dev shot         # screenshots of the UI's pages, in target/dev-shots
cargo xtask dev status
cargo xtask dev reset        # remove the slot's data and start again
cargo xtask dev down         # stop the slot and remove the preview's routes
```

|          | Dev slot                                                                | Installed service                         |
| -------- | ----------------------------------------------------------------------- | ----------------------------------------- |
| Units    | `ren-dev-provider`, `ren-dev-daemon`, `ren-dev-traffic`, `ren-dev-ui`   | `ren`, `ren-ui`                           |
| Loopback | fake provider `18999`, daemon `18787`, UI `13000`, traffic API `13001`  | daemon `8787`, UI `3000`                  |
| Tailnet  | `9443` (gateway), `9444` (UI), with `dev up --ui`                       | your `--https-port` and `--ui-https-port` |
| Files    | `~/.config/ren-dev`, `~/.local/state/ren-dev`, `~/.local/share/ren-dev` | `~/.config/ren`, `~/.local/state/ren`     |

The units are transient systemd user units, stopped by name. The dev
configuration is regenerated on every `up`. It sets `[instance] environment =
"development"`, which the gateway reports in `/health`: the UI's logo gets a dev
badge, and `ren doctor` and `ren run` name the instance. Every
provider in it is the fake one, which
accepts any credentials and stores nothing, so no real credential or prompt
reaches the slot. It names two users, `alice` (by the Tailscale login
`alice@example.com`) and `ci` (by a fixed fake token), and two direct
operator-key entries with a fake key: `team-key` (only `ci`) and `chat-key`
(Chat Completions). Its replies read like real use: invented engineering answers
about an invented project, with realistic model names, IDs, and error messages.
The generated traffic picks a reply kind (tool work, a long or slow reply, a cut
stream, an error, a rate limit) with an `x-dev-reply` request header, which the
gateway forwards but never records; a request without it, as from a real client,
gets an ordinary text answer. Tool work is a turn of several calls, sent one after
another: each request declares its client's own tools (Claude Code's `Read`,
`Edit`, `Write`, and `Bash`; Codex's `read_file`, `apply_patch`, and `shell`; Pi's
and OpenCode's lowercase ones), the provider calls one of them, and the next
request carries that call and its result back, so turns read files, write them,
and run commands as real ones do. Like the real providers, it reports prompt-cache
usage when a request asks for caching, thinks (Messages) or reasons
(Responses) before answering when asked to, and stops at a request's
`max_tokens`. The generated requests ask as coding agents do, and about one
Claude Code call in ten is a side call with no turn (a title or a quota probe).
So a real client can run against the slot, on the host or, after `dev up --ui`,
from the tailnet:

```sh
ren run claude --gateway http://127.0.0.1:18787
ren run claude --gateway https://<node>:9443
```

The slot always has live traffic: the `ren-dev-traffic` unit sends calls
through the dev gateway as `~/.config/ren-dev/traffic.toml` says (running or
paused, calls a minute, the fake provider's pace, and weights over replies,
harnesses, and callers). The generator and the fake provider read the file again
for every call, so `dev traffic set`, `pause`, and `resume` apply at once; an
invalid file falls back to the defaults. The dev UI also has them in a floating
toolbar (bottom right; it folds into a small tab): pause, rate, pace, a burst, the
mix, and single slow, tool, error, or rate-limited calls. The generator serves them
on `127.0.0.1:13001`, never published; the UI server forwards the toolbar's
requests there only when `REN_DEV_TRAFFIC_URL` is set, as the slot sets it, and
only a development gateway's UI shows the toolbar.

The slot's daemon has runs on, with `~/.local/share/ren-dev/work` as its root
(`acme-api` and `acme-web` in it), and the dev UI drives them. Its `codex`,
`claude`, and `pi` are fakes (`cargo xtask dev agent`, first on the dev
daemon's `PATH`), never the real clients: each speaks its client's protocol
(Codex's app server, Claude Code's stream-json print mode, Pi's RPC mode), runs
a command for prompts that mention testing, running, or building and changes
files for ones that mention fixing, editing, changing, adding, or writing
(Codex and Claude Code ask first; Pi asks for no approvals), and sends each
turn's model call through the dev gateway to the fake provider, so a run's
turns are recorded under its session. `dev up` restarts the daemon as
`install-vps` does for runs: it waits for every run starting, working, or stopping
(up to `--idle-timeout`, 300 seconds by default; `--drain-runs` does not wait), and
names the idle runs that end with the restart, which can be continued. It does not
wait for calls, since the fake traffic never stops.

`--ui` publishes the preview with the same Serve step as `install-vps` (with
`sudo -n` if it last did, or `--sudo-tailscale`); `--local` keeps it on
loopback. `down` removes each preview route only while it still points at the
slot. `shot` needs a headless Chromium: `REN_CHROME`, `chrome-headless-shell` or
`chromium` on `PATH`, or Playwright's downloaded headless shell.

### Manual steps

Build on the VPS with Rust and `cargo build --release --locked`, or copy an
executable built for the VPS's Linux architecture; macOS binaries do not run on
Linux. A host running a prebuilt executable needs no Bun or Node. Install the
release binary and use the systemd workflow above, keeping Ren bound to
`127.0.0.1:8787`. Confirm local health before adding HTTPS:

```sh
systemctl --user is-enabled ren.service
systemctl --user is-active ren.service
loginctl show-user "$USER" -p Linger
"$HOME/.local/bin/ren" --config "$HOME/.config/ren/ren.toml" doctor
ss -ltn 'sport = :8787'
tailscale serve status
```

Use an unused HTTPS port on the node. The example below uses **8443**, allowing an
existing Serve application on 443 to keep its current configuration. Do not reset
Serve or replace an occupied port. Run as the existing Tailscale operator, or with
`sudo` if host permissions require it:

```sh
tailscale serve --bg --https=8443 http://127.0.0.1:8787
tailscale serve status
```

`--bg` persists the route across Tailscale restarts and host reboots. User lingering
keeps systemd's user manager running after logout and starts it at boot. Neither
requires keeping an SSH session open. HTTPS certificates must already be enabled
for the tailnet, or Tailscale will provide its setup flow.

On the client machine, save the private HTTPS **root URL** printed by Serve once
(see [Clients](clients.md#diagnose-and-launch)), then launch as usual:

```sh
ren init --gateway https://your-vps.your-tailnet.ts.net:8443
ren doctor
ren run claude
ren run codex
ren run pi -- --model openai/<model-id>
```

`--gateway` or `REN_GATEWAY` still override the saved URL for one command.

This URL is the headless daemon, not the UI. Launchers append the native route
prefixes; manual client configurations use `/anthropic`, `/codex`, or `/openai/v1`
as appropriate. A successful `doctor` verifies the HTTPS connection and
recording/storage health without sending inference requests. Confirm forwarding
with mocks or an explicitly chosen client request separately.

For the optional UI, run its independent Node server on loopback port 3000 and
publish it on a second **unused** Serve port (for example 8444). Set
`REN_GATEWAY_URL=http://127.0.0.1:8787`,
`REN_PUBLIC_GATEWAY_URL=https://your-vps.your-tailnet.ts.net:8443`, and
`ORIGIN=https://your-vps.your-tailnet.ts.net:8444` in the UI service environment.
The public gateway setting keeps copied client commands pointed at the daemon.
See the [UI service instructions](../ui/README.md#vps-service) for installation.
To drive [runs](api.md#runs) from the UI, also add
`REN_CONTROL_TOKEN_FILE=<data dir>/control.token` and
`REN_CONTROL_LOGINS=<your Tailscale login>` to `~/.config/ren/ui.env` and restart
`ren-ui.service` (see [the UI's runs](../ui/README.md#runs)); `install-vps` keeps
an existing `ui.env` as it is.
The UI may instead run on your laptop with `REN_GATEWAY_URL` set to the daemon's
Tailscale URL; no UI process is required on the VPS.

Tailnet access rules govern both forwarding and inspection; allow only intended
users/devices to reach this port. Ren has no separate gateway login; [named
users](policy.md#named-users) identify callers and can be limited or blocked, but
do not replace tailnet access rules. Keep it private
with Serve, not Funnel or a public reverse proxy. The gateway removes Tailscale
identity/capability and standard forwarding headers before contacting providers;
provider authentication remains unchanged. Inspect `tailscale serve status --json`
to confirm this port has no `AllowFunnel` entry set to true.

To withdraw only Ren's endpoint, preserving other Serve applications:

```sh
tailscale serve --https=8443 off
systemctl --user disable --now ren.service
```

For upgrades, keep a copy of the previous executable, stop Ren, replace the binary,
and start it again. Run local and remote `doctor` checks afterward. A restart
gives active requests 10 seconds to finish and then cancels them, so upgrade
between sessions. Before upgrades that change the database schema, run
`ren control backup` or stop the service and copy the complete state directory; do
not downgrade a migrated database without restoring a matching backup.

References: [Tailscale Serve CLI](https://tailscale.com/docs/reference/tailscale-cli/serve),
[Serve access and identity](https://tailscale.com/docs/features/tailscale-serve), and
[systemd user lingering](https://www.freedesktop.org/software/systemd/man/latest/loginctl.html).

## Containers

The [`Dockerfile`](../Dockerfile) builds the daemon image for Docker and Podman
alike. It runs `ren serve` as a non-root user (uid 65532) with its data in
`/data/ren`, on the `/data` volume, and contains only the binary, the C runtime, and CA certificates.
The UI is not included. Build it from the checkout:

```sh
docker build -t localhost/ren:latest .
podman build -t localhost/ren:latest .
```

The listener stays on loopback inside the container; Ren refuses any other
address. Don't publish a port or use host networking. Instead, a Tailscale
container shares Ren's network namespace and serves it over HTTPS on the
tailnet with Tailscale Serve. That sidecar is a separate
tailnet device (named `ren` by default) and serves on its own port 443. Serve
adds the identity headers that `control.tailscale_logins` checks, and nothing
else can reach the listener. Its route is
[`deploy/container/tailscale-serve.json`](../deploy/container/tailscale-serve.json),
with Funnel off.

Both setups below need a configuration file. Copy `ren.example.toml` and review
it; keep `listen` on loopback and leave `data_dir` unset, so the data lands in
the volume. Operator API keys go in the container environment, never in the
file. The sidecar needs a Tailscale [auth key](https://tailscale.com/kb/1085/auth-keys)
for its first start only; without one, its log prints a login URL.

Containers keep the daemon's shutdown budget (10 seconds for running calls,
then capture drain), so both setups allow 30 seconds before killing it. Upgrade
by rebuilding the image and restarting Ren between sessions, after
`ren control backup` when the schema changes. Don't use `podman auto-update` or
an image watcher: they restart whenever the image changes, mid-stream.

### Compose

[`deploy/container/compose.yaml`](../deploy/container/compose.yaml) runs with
`docker compose` or `podman compose`. From `deploy/container`:

```sh
cp ../../ren.example.toml ren.toml
chmod 0644 ren.toml
echo 'TS_AUTHKEY=tskey-auth-...' > .env
docker compose up -d --build
docker compose exec ren ren --config /etc/ren/ren.toml doctor
```

`ren.toml` must be readable by the container's user, hence `0644`; it holds no
secrets. `.env`, `ren.toml`, and `*.env` there are ignored by git and by image
builds. For operator keys, uncomment `env_file: ren.env`. Run other commands the
same way, for example `docker compose exec ren ren control backup`.
Logs: `docker compose logs ren`.

### Podman Quadlet

On Linux, the [Quadlet files](../deploy/container/quadlet) are the recommended
Podman setup: rootless, supervised by your systemd user manager like
`ren.service`, with logs in the journal. They define a `ren` pod, its
`ren-tailscale` and `ren-gateway` containers (units `ren-tailscale.service` and
`ren-gateway.service`, which leaves the name `ren.service` to the host unit),
and the `ren-data` and `ren-tailscale` volumes. The pod maps your user to the
image's user, so `ren.toml` may stay `0600` and Ren's files belong to you; read
the volume through `podman exec` or `podman unshare`, since Podman keeps its top
directory private. Requires Podman 5.0 or later.

```sh
podman build -t localhost/ren:latest .
(umask 077; mkdir -p "$HOME/.config/ren" "$HOME/.config/containers/systemd")
cp -n ren.example.toml "$HOME/.config/ren/ren.toml"
cp deploy/container/tailscale-serve.json "$HOME/.config/ren/"
(umask 077; echo 'TS_AUTHKEY=tskey-auth-...' > "$HOME/.config/ren/tailscale.env")
cp deploy/container/quadlet/* "$HOME/.config/containers/systemd/"
systemctl --user daemon-reload
systemctl --user start ren-pod.service
podman exec ren-gateway ren --config /etc/ren/ren.toml doctor
```

Quadlet units are generated, so they are started rather than enabled; their
`[Install]` section starts them at login, and at boot with lingering
(`loginctl enable-linger "$USER"`). Logs: `journalctl --user -u ren-gateway`.
Stop with `systemctl --user stop ren-pod.service`; Ren stops before Tailscale.
