# Deployment

Run TGV as a background service on macOS or Linux, and reach a VPS deployment
through Tailscale.

## Configuration

[`tgv.example.toml`](../tgv.example.toml) documents every key; unknown keys are
rejected. Data defaults to `$XDG_STATE_HOME/tgv` when that variable is an absolute
path, otherwise `$HOME/.local/state/tgv`. CLI `--data-dir` takes priority over the
configured path; relative configured paths resolve beside the config file, while
relative CLI paths resolve against the current directory. Configuration is
immutable until restart; every call records its configuration revision.

The daemon serves forwarding, `/health`, and the management APIs on one listener;
its root URL returns 404. It is headless: building, testing, and installing it need
no Bun, Node, or frontend assets, and stopping the [UI](../ui/README.md) does not
interrupt forwarding.

## Background services

`tgv service render` generates a user-service definition for **launchd on macOS**
or **systemd on Linux**. It does not install, register, or start a service. Use a
stable installed binary, not a checkout's `target/debug/tgv`. For example:

```sh
cargo install --locked --path crates/tgv-cli --root "$HOME/.local"
(umask 077; mkdir -p "$HOME/.config/tgv")
cp -n tgv.example.toml "$HOME/.config/tgv/tgv.toml"
```

Review the configuration before starting. The commands below explicitly select
`~/.local/state/tgv`; change `--data-dir` to use an existing capture directory.
Stop any foreground daemon using that listener or directory first.

### macOS: launchd

Create the log directory before loading the agent: launchd opens its log file
before TGV can create storage directories.

```sh
(umask 077; mkdir -p "$HOME/Library/LaunchAgents" "$HOME/.local/state/tgv/logs")
"$HOME/.local/bin/tgv" --config "$HOME/.config/tgv/tgv.toml" \
  --data-dir "$HOME/.local/state/tgv" service render --manager launchd \
  --binary "$HOME/.local/bin/tgv" \
  --output "$HOME/Library/LaunchAgents/dev.tgv.gateway.plist"
plutil -lint "$HOME/Library/LaunchAgents/dev.tgv.gateway.plist"
launchctl bootstrap "gui/$(id -u)" "$HOME/Library/LaunchAgents/dev.tgv.gateway.plist"
"$HOME/.local/bin/tgv" --config "$HOME/.config/tgv/tgv.toml" doctor
```

Status: `launchctl print "gui/$(id -u)/dev.tgv.gateway"`.
Stop: `launchctl bootout "gui/$(id -u)/dev.tgv.gateway"`.
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
"$HOME/.local/bin/tgv" --config "$HOME/.config/tgv/tgv.toml" \
  --data-dir "$HOME/.local/state/tgv" service render --manager systemd \
  --binary "$HOME/.local/bin/tgv" \
  --output "$HOME/.config/systemd/user/tgv.service"
systemd-analyze --user verify "$HOME/.config/systemd/user/tgv.service"
systemctl --user daemon-reload
systemctl --user enable --now tgv.service
"$HOME/.local/bin/tgv" --config "$HOME/.config/tgv/tgv.toml" doctor
```

Use `systemctl --user status tgv`, `stop tgv`, or `restart tgv` for lifecycle
management. Read logs with `journalctl --user -u tgv`; journal policy controls
their retention. To disable startup, use `systemctl --user disable --now tgv`.
For unattended VPS operation after logout, enable user lingering with
`loginctl enable-linger "$USER"` (host policy may require an administrator).
Startup at boot relies on the user unit plus lingering; a host reboot has not been
exercised. The static [`deploy/tgv.service`](../deploy/tgv.service) is an
alternative for the standard paths above. Check lingering with
`loginctl show-user "$USER" -p Linger`. The VPS needs only TGV and Tailscale;
install and sign in to coding clients on the machine where you use them. Provider
credentials are not installed on the VPS.

Run the isolated Linux service check before installing or upgrading:

```sh
cargo build --release --locked
python3 scripts/smoke-systemd.py --binary target/release/tgv
```

It requires a non-root user with a working systemd user manager. It renders and
verifies a uniquely named runtime unit, tests crash restart and all three provider
routes against local mocks, stops gracefully, then checks private storage and
history persistence across restart. It removes its unit and temporary state without
changing `tgv.service`, lingering, Tailscale, or provider credentials by default.
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
run `systemctl --user reset-failed tgv` before restarting if it hits that limit.
Launchd throttles respawns to at most once every 10 seconds. Neither manager's
"running" status proves recording health: use `tgv doctor` after starting.

The renderer validates configuration and the executable, resolves paths to absolute
paths, and fixes the data directory through a CLI override. It rejects an ephemeral
listen port. Config files remain referenced, so ordinary settings take effect on
restart; a changed data-directory setting requires regenerating the definition.
No shell environment, provider credentials, or configuration contents are embedded.
It does not alter Tailscale, firewall rules, or listener exposure.

Omit `--output` to review the definition on stdout. With `--output`, TGV publishes
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

Build on the VPS with Rust and `cargo build --release --locked`, or copy an
executable built for the VPS's Linux architecture; macOS binaries do not run on
Linux. A host running a prebuilt executable needs no Bun or Node. Install the
release binary and use the systemd workflow above, keeping TGV bound to
`127.0.0.1:8787`. Confirm local health before adding HTTPS:

```sh
systemctl --user is-enabled tgv.service
systemctl --user is-active tgv.service
loginctl show-user "$USER" -p Linger
"$HOME/.local/bin/tgv" --config "$HOME/.config/tgv/tgv.toml" doctor
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

On the client machine, use the private HTTPS **root URL** printed by Serve:

```sh
tgv doctor --gateway https://your-vps.your-tailnet.ts.net:8443
tgv run claude --gateway https://your-vps.your-tailnet.ts.net:8443
tgv run codex --gateway https://your-vps.your-tailnet.ts.net:8443
tgv run pi --gateway https://your-vps.your-tailnet.ts.net:8443 -- --model openai/<model-id>
```

This URL is the headless daemon, not the UI. Launchers append the native route
prefixes; manual client configurations use `/anthropic`, `/codex`, or `/openai/v1`
as appropriate. A successful `doctor` verifies the HTTPS connection and
recording/storage health without sending inference requests. Confirm forwarding
with mocks or an explicitly chosen client request separately.

For the optional UI, run its independent Node server on loopback port 3000 and
publish it on a second **unused** Serve port (for example 8444). Set
`TGV_GATEWAY_URL=http://127.0.0.1:8787`,
`TGV_PUBLIC_GATEWAY_URL=https://your-vps.your-tailnet.ts.net:8443`, and
`ORIGIN=https://your-vps.your-tailnet.ts.net:8444` in the UI service environment.
The public gateway setting keeps copied client commands pointed at the daemon.
See the [UI service instructions](../ui/README.md#vps-service) for installation.
The UI may instead run on your laptop with `TGV_GATEWAY_URL` set to the daemon's
Tailscale URL; no UI process is required on the VPS.

Tailnet access rules govern both forwarding and inspection; allow only intended
users/devices to reach this port. TGV has no separate gateway login. Keep it private
with Serve, not Funnel or a public reverse proxy. The gateway removes Tailscale
identity/capability and standard forwarding headers before contacting providers;
provider authentication remains unchanged. Inspect `tailscale serve status --json`
to confirm this port has no `AllowFunnel` entry set to true.

To withdraw only TGV's endpoint, preserving other Serve applications:

```sh
tailscale serve --https=8443 off
systemctl --user disable --now tgv.service
```

For upgrades, keep a copy of the previous executable, stop TGV, replace the binary,
and start it again. Run local and remote `doctor` checks afterward. A restart
gives active requests 10 seconds to finish and then cancels them, so upgrade
between sessions. Before upgrades that change the database schema, run
`tgv control backup` or stop the service and copy the complete state directory; do
not downgrade a migrated database without restoring a matching backup.

References: [Tailscale Serve CLI](https://tailscale.com/docs/reference/tailscale-cli/serve),
[Serve access and identity](https://tailscale.com/docs/features/tailscale-serve), and
[systemd user lingering](https://www.freedesktop.org/software/systemd/man/latest/loginctl.html).
