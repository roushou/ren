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
The systemd unit uses `KillMode=mixed`: SIGTERM goes to the daemon alone. Runs
a run host holds ([below](#runs-through-a-restart)) are not in the unit and go
on. Any other [run](api.md#runs) the daemon ends itself, after its calls, and
records as `drained` (a kill Ren sent, so the exit is usually `signal 9`). A
run whose process exits before the daemon reaches it is recorded as `exited`
with the exit seen. Whatever is still running when the stop timeout passes is
killed with the rest of the unit.

#### Runs through a restart

With `[runs] hold = "auto"` (the default) and the daemon running under systemd
on Linux, each run's client is started by a run host, `ren run-host`, in a
transient user unit of its own, `ren-run-<run>`. Stopping, restarting, or
upgrading `ren.service` leaves the client working, mid-turn: the host keeps its
stdin open and every byte it writes (in `<data>/runs/<run>`). The next daemon
takes each run up where the last one left it, from what the last one journaled,
and records a `run_detached` and a `run_rejoined` observation around the time
no daemon watched it. A run whose host cannot be taken up is stopped and
recorded as `recovered`. The run's model calls made while no daemon listened
fail like any call to a stopped gateway, and its client retries them as it
retries any. `hold = "systemd"` holds runs wherever `systemd-run` works;
`hold = "off"`, and every host without systemd (macOS), ends runs with the
daemon as above. `ren runs ls` says which: each run's `tenure` is `held` or
`bound`.

`hold = "container"` holds each run the same way, but starts its run host in a
rootless Podman container of its own, `ren-run-<run>`, as the operator
(`--userns=keep-id`), on the host's network, and removed once the host exits.
The container sees, each at its host path: the run's folder; its workdir and
the repository's git folder (a worktree's common one); this `ren`; the client's
program as found on the run's path; what the client keeps under the home
(`.claude` and `.claude.json`, `.codex`, `.pi`, opencode's config, data, and
state folders), where it exists; and the SSH agent's socket. Nothing else of
the home, Ren's configuration and data, or other runs. `[runs.container]` names
the image (the instance image) and adds what every run sees: the host's
programs (`/usr`) and toolchains read-only, shared caches read-write. A program
the run starts that is in none of these is not found in its container.
A run is refused, and the daemon logs why, when anything it would see is
Ren's data folder or holds it, or Ren's configuration file or holds it, or
when `[runs.container]` names a path inside Ren's data; only what Ren made for
the run there (its run folder, its worktree, and the doors) is seen.

`[runs.container] network = "relayed"` takes the host's network away from a
run: its container has only its loopback (`--network=none`). There its run host
listens on the gateway's address and on `127.0.0.1:3128`, and passes each
connection through a Unix socket in `<data>/runs/.reach`, which the container
sees read-only, to a listener of the daemon's that serves model forwarding
alone (never the read or control APIs) and to its relay (`ren-relay`). The
client's base URL is unchanged; `HTTPS_PROXY` and `HTTP_PROXY` name the relay,
with a ticket minted for the run as the password, and `NO_PROXY` keeps loopback
direct. The relay lets a run connect only to where `[runs.container] reach`,
its client (Codex's and Claude Code's sign-in refresh, opencode's model
catalogue), its project, and its grant list, a name only at public addresses, and passes the connection through
unread. Every connection is logged with its run; a refused one is a warning
that names the destination and why. A project lists what its runs need in
its `.ren/project.toml` (`reach = ["registry.npmjs.org:443"]`, host names only): Ren reads it
from the repository's default branch on GitHub, for a repository watched under
`[[github.repository]]`, never from a run's checkout, which the run could
change; a manifest that does not read lists nothing, with a warning. Each
recorded passage says which list let it through. A run held through a daemon restart keeps
its pass: the daemon keeps it in `<data>/runs/.admitted`, which no run sees, and
issues it again from there. A relayed run
never holds the SSH agent. It pushes with a grant instead: with `[github.app]`
set (a GitHub App installed on the repositories, allowed to write contents and
pull requests), a run working in the registered folder of a project under
`[[github.repository]]`, or in a worktree of it, may reach `github.com:443`, and its git
asks `ren credential` for a token, which asks the daemon over
`<data>/runs/.reach/grants.sock`. The daemon mints, with the App's key, a token
for that repository alone, lasting an hour, and hands it out for 45 minutes. A
checkout cloned over SSH is fetched and pushed over HTTPS through the relay.
The App's key file must be readable by the daemon; `install-vps --contain all`
mounts it into the daemon's container read-only.

`[runs.container] runtime = "gvisor"` runs a relayed run's processes under
gVisor (`runsc`): its system calls are served by a kernel of its own in user
space, not the host's. It needs no KVM and runs rootless, with no network but
its loopback, so it needs `network = "relayed"`. Install gVisor from its own
apt repository (gvisor.dev/docs/user_guide/install); Ubuntu's `runsc` package
(20240729, built with a newer Go than it supports) panics in its kernel on some
starts. Podman's API socket cannot choose a container's runtime, so a daemon in
a container starts gVisor runs through a Podman service of their own,
`ren-gvisor` (socket-activated, at `/run/user/<uid>/ren-gvisor/podman.sock`),
which `install-vps --contain all` sets up when the configuration asks for
gVisor; set `[runs.container] podman_socket` to it. Before each start through
a socket, the daemon checks the service runs the run's runtime, and refuses
the run if not. The default, `host_kernel`, is Podman's own runtime.

Systemd limits repeated startup failures to five in a minute; fix the cause and
run `systemctl --user reset-failed ren` before restarting if it hits that limit.
Launchd throttles respawns to at most once every 10 seconds. Neither manager's
"running" status proves recording health: use `ren doctor` after starting.

`install-vps --contain all` runs the daemon itself in a container too,
`ren-production-daemon`, as `ren.service`'s main process (`podman run` in the
foreground): it sees, each at its host path, only the installed `ren` and its
configuration (read-only), its data, its run roots, places, and scratch folder,
`[runs.container]`'s paths, what every client keeps, the Podman socket's
folder, and the SSH agent. It needs its runs held in containers through the
operator's Podman API socket (`[runs] hold = "container"`,
`[runs.container] podman_socket = "/run/user/<uid>/podman/podman.sock"`);
the install turns on `podman.socket` and builds the instance image. The unit
keeps what `ren.service`'s drop-ins set (keys, the search path), which the
container inherits. Upgrades, their pause and backup, the rollback, and
`ren doctor` are as for the host install; the pause and backup talk to the
running daemon with the build being installed, which reads the configuration as
the new daemon will. With `--ui-https-port`, the UI runs in a container too,
`ren-production-ui`, as `ren-ui.service`'s main process, on the instance
image's Node: it sees only its deployed copy and the control token file
`ui.env` names, both read-only, and takes `ui.env` from its unit. The choice
is saved with the publishing flags.

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
no remote branch. It builds the release binary while the old daemon keeps
serving, creates `~/.config/ren/ren.toml` only if missing, and checks the
configuration with Ren's own parser before stopping anything.

It then replaces a running daemon only between calls and, for runs bound to the
daemon, between their turns. Runs a run host holds go on through the restart
([runs through a restart](#runs-through-a-restart)) and are not waited for. It
waits until no call is running and no bound [run](api.md#runs) is starting,
working (also while it waits for an approval), or stopping, naming each run it
waits for (up to `--idle-timeout`, 900 seconds by default, then stops without
changing anything). `--drain-runs` restarts without waiting for bound runs:
their turns are lost and their processes end with the daemon; running calls
are still waited for. It then pauses admission so new calls get `503` with
`retry-after: 10` and clients wait (a run prompted between the check and the
pause is waited for too). Idle bound runs end with the old daemon: before
stopping it, it lists each with how to continue it.
Writing to its session (from the UI's Sessions page or
`ren runs say <session> <prompt>`) starts its client again, which resumes the
conversation where it left off; a run from a daemon without sessions is
resumed with `ren runs start <client> <dir> --resume <thread>`. It runs `ren control backup`, stops the service, moves the old
binary aside to `~/.local/bin/ren.rollback`, installs the new one, rewrites the unit
only if it changed, starts the service, enables lingering, and runs `ren doctor`.
It requires doctor's build check to confirm the daemon runs the build just
installed. When that passes, the old binary is removed; when any of these steps
fails, it stops the service, puts the old binary and its unit back, starts it,
and checks it with `ren doctor` again, then reports the failure. A build that
migrated the database may leave the old binary unable to open it; the install
then says so and names the backup. It then resumes admission, also when a step
failed. A daemon an
operator had already paused stays paused. Its own pause is marked in
`<data>/install-pause` (the operator revision the pause made) until it
resumes, so an install stopped between the two leaves a pause the next one
recognizes and lifts; once anything else changes admission, the pause is the
operator's.

Started inside `ren.service` (by an agent a run drives, say), whose restart
would stop it before it resumes, it carries on in the background as the
transient unit `ren-install-vps` and returns at once. The install waits for
every run's turn to end, the agent's own included, so it restarts the daemon
after that turn ends; its output goes to the journal
(`journalctl --user -u ren-install-vps -f`). With `--https-port`, it adds a Serve
route only when that port is free, accepts one that already proxies to the
configured listener, and refuses a port used by anything else or exposed through
Funnel. `--sudo-tailscale` uses `sudo -n` for that step; `--skip-backup` skips the
backup. With no
`--https-port` given or saved, it leaves Tailscale untouched. Review the configuration after the
first run.

With `--ui-https-port` (which needs `--https-port`), it also deploys the
[UI](../ui/README.md#vps-service). It needs Bun and Node on `PATH`. Before
stopping anything, it builds `ui/` and stages the build with its production
dependencies in `~/.local/share/ren-ui.next`. After the daemon is upgraded, it
writes `~/.config/ren/ui.env` if missing (the daemon on loopback, both
Tailscale URLs, and `REN_SOURCE_DIR`, this checkout, for Redline) and adds
`REN_SOURCE_DIR` to an existing one that names none, stops `ren-ui.service`, moves the deployed copy
aside to `ren-ui.rollback`, swaps in the new one, installs the unit from
`deploy/ren-ui.service` with the Node found on `PATH`, starts it, and checks the
UI's `/health`, which relays the daemon's. When it answers, the old copy is
removed; when it does not, the old copy and its unit are put back and started,
and the install reports the failure. It then publishes the UI on that port
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
(Chat Completions). Its UI's Redline sends to the fake `acme-web` project
under the run root, standing in for Ren's checkout, so the slot's fake agents
never work in the real one. Its replies read like real use: invented engineering answers
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
invalid file falls back to the defaults. The generator serves its controls on
`127.0.0.1:13001`, never published, for `dev traffic`.

The slot's daemon has runs on, with `~/.local/share/ren-dev/work` as its root
(`acme-api` and `acme-web` in it), and the dev UI drives them. Its `codex`,
`claude`, `pi`, and `opencode` are fakes (`cargo xtask dev agent`, first on the
dev daemon's `PATH`), never the real clients: each speaks its client's protocol
(Codex's app server, Claude Code's stream-json print mode, Pi's RPC mode,
OpenCode's ACP mode), runs a command for prompts that mention testing, running,
or building and changes files for ones that mention fixing, editing, changing,
adding, or writing (Codex and Claude Code ask first; Pi asks for no approvals;
OpenCode asks when the run's approval policy says so), and sends each
turn's model call through the dev gateway to the fake provider, so a run's
turns are recorded under its recording. The slot holds its runs as the
installed service does ([runs through a restart](#runs-through-a-restart)), so
`dev up` restarts its daemon with their turns going on. For runs bound to the
daemon, `dev up` does as `install-vps` does: it waits for every one starting,
working, or stopping (up to `--idle-timeout`, 300 seconds by default;
`--drain-runs` does not wait), and names the idle ones that end with the
restart, which can be continued. It does not wait for calls, since the fake
traffic never stops. `dev down` and `dev reset` also stop the slot's run hosts.

`--ui` publishes the preview with the same Serve step as `install-vps` (with
`sudo -n` if it last did, or `--sudo-tailscale`); `--local` keeps it on
loopback. `down` removes each preview route only while it still points at the
slot. `shot` needs a headless Chromium: `REN_CHROME`, `chrome-headless-shell` or
`chromium` on `PATH`, or Playwright's downloaded headless shell.

### Canary and promotion

`cargo xtask canary` checks `main`'s tip before production takes it, and
promotes it as you decide ([the plan](plans/self-build.md), phase 4). One
pass:

```sh
cargo xtask canary run              # fetch main, build, check, promote by policy
cargo xtask canary promote          # install the canary's build now, once it passed
ren release                         # main, production, the canary, and the decision
cargo xtask canary enable --every 10   # a pass every 10 minutes, on a user timer
cargo xtask canary disable
cargo xtask canary down             # stop the canary
```

A pass fetches `main` from `origin` and checks its tip out, detached, in
Ren's own worktree, `~/.local/share/ren/factory`, then goes on with that
commit's own xtask, so a commit is checked by its own checks. It builds the
commit once into `~/.local/share/ren/builds/<commit>` (the release `ren`
and the UI's deployable copy; the newest few are kept). It backs up
production's history with the running daemon (`ren control backup`) and
starts the build as the canary on a copy of it: the rootless Podman pod
`ren-canary`, its daemon and UI each a container (`ren-canary-daemon`,
`ren-canary-ui`; `podman logs` reads them), on loopback `17787` and `17000`,
with `~/.config/ren-canary` and `~/.local/state/ren-canary`, both replaced on
every pass. Each container is started from a transient systemd scope of its
own (`ren-pod-canary-daemon`, `ren-pod-canary-ui`), so it outlives the pass
that started it, the timer's included. A container sees only the build it runs, read-only, and the
canary's own configuration and data, each at its host path; it runs as the
operator's user and shares the host's network, so `ren` keeps its loopback
listener. The image, `localhost/ren-instance` (Node, the C runtime, and CA
certificates, from `deploy/instance/Containerfile`), holds no build of Ren:
it is built once, the first time a pass needs it. Its configuration is production's with its own listener, `[instance]
environment = "canary"`, no `[runs]`, and a placeholder for every operator
key, so it drives no runs and forwards no operator-keyed call; its UI only
reads. `--publish` also publishes it on the tailnet, gateway `9445` and UI
`9446`, with the install's saved Tailscale choice.

It then checks the canary: `ren doctor` (nothing failed, and the daemon runs
the build its CLI is), every read of the read API over the copied history
(each listing, then one item of each), and every page of the UI in a headless
Chromium (as `dev shot` finds it; the captures are kept in
`~/.local/share/ren-canary/shots`). A page fails unless the UI answers it,
after its redirects, with a success, so an error page is a failure; a
project's pages are those of a project in the copied history (Ren's, else the
first), and a page with nothing to show yet is skipped. A check that cannot
run fails. It writes
where the release stands to production's data folder (`release.json`): `main`'s
tip, the build production runs and how many commits it is behind, the
canary's build and each check, and the decision. Ren's project's release
(`/api/projects/ren/release`, or whatever name it is registered under)
serves it, `ren release` prints it (another project's with `--project`), the UI's Projects page shows it in a line, and Ren's Release page (`/projects/ren/release`)
shows all of it: the three builds, each check, the pages the canary
captured (kept in production's data folder, `release-shots`), what the
policy decided, the work items production's build shipped and those on
`main` waiting for it, and each project's release. The view's Promote asks
for the canary's passed build; the next pass installs it, as `canary
promote` would, and drops a request that is no longer the canary's passed
build. With `canary enable`, that is within one timer period.

When the build passed and follows production's commit, the policy in
`~/.config/ren/release.toml` decides:

```toml
promote = "manual" # the default: `cargo xtask canary promote`
# promote = "automatic"      # as soon as it passes
# promote = "02:00-05:00"    # as soon as it passes inside this daily UTC window
```

Promoting runs `install-vps --build <commit>` with the saved publishing
flags: the install's own steps (the wait for calls and bound runs' turns, the
pause, the backup, the build check, and the rollback on failure), with the
kept build instead of building the checkout. No policy promotes a build that failed its checks, that production already runs, or that lacks commits production has, nor one over a production build that is not a clean commit of `main`; `canary promote` promotes any build that passed. A promotion that fails is rolled back by the install and marked on the build, and is not tried again unless you promote
it. `install-vps --build <commit>` also installs a kept build by hand.

Every pass also closes work items ([work items](api.md#work-items)): it
lands each sent item that a commit in `main`'s recent history names in a
`Work-Item: <id>` trailer, waits for production to store the landings, and
ships each landed item whose commit production's clean build holds, through
production's own control API with its control token (read from its data
folder and handed to curl in a private file). `cargo xtask canary work` does
only this, against this checkout's commit as `main` (or `--main <commit>`).
A production whose daemon keeps no work items yet is left alone.

`canary enable` writes `ren-canary.service` and `ren-canary.timer` to
`~/.config/systemd/user`, running `cargo xtask canary run` from the checkout
it was enabled in; follow a pass with `journalctl --user -u ren-canary`.
`--of dev` rehearses all of it with the dev slot as production: its fake
history, its `release.json`, and never a promotion.

### Previews

`cargo xtask preview` runs one worktree's branch on a copy of production's
history, so its change is judged on the page it was marked on, beside
production's. It runs in a worktree Ren made: it reads `REN_WORKTREE`,
which the worktree's runs are told, refuses a folder that is not one of
Ren's worktrees named `<repo>-<slot>`, and takes the data folder and ports
Ren gives that slot. A brief asks the agent to start one once its change is
ready:

```sh
cargo xtask preview up --publish   # build, copy production's history, start, record
cargo xtask preview down --publish # stop, unpublish, take it off the work items
```

`up` builds the branch (debug) into `<data>/preview`, backs up production's
history with production's own `ren`, and starts the build as a replica, as
the canary is: the pod `ren-preview-<worktree>`, its daemon on the slot's
first port and its UI on the next, with production's configuration but
`environment = "preview"`, no runs, no operator key, and a UI that only reads.
`--publish` publishes both on the tailnet on the same port numbers, with the
install's saved Tailscale choice. It then records the preview on every sent
work item the branch's recent commits name (`Work-Item: <id>`), and on each
`--work <id>`: a work item's page opens the marked page on it beside
production's.
`--of dev` copies the dev slot instead. Deleting the worktree's session
stops its preview with it, and takes it off the session's work items.

#### Previews of pull requests and commits

Every canary pass also brings the factory's own previews up to date
(`cargo xtask canary previews` runs just that): one for each commit the
operator asked to preview, newest ask first, then one for each of Ren's
open pull requests that is not a draft, as production last saw them on
GitHub (`[github]`), most recently updated first. As many run as
`release.toml` allows:

```toml
previews = 2 # 1 to 8; each is a daemon, a UI, and a copy of production's history
```

Each runs in a slot: slot `n`'s daemon listens on loopback `18100 + 2n` and
its UI on the next port, published on the tailnet on `9450 + 2n` and the next
with `--publish`, in the pod `ren-preview-<n>`, with `~/.config/ren-previews/<n>`
and `~/.local/state/ren-previews/<n>`. The canary timer publishes previews only when it was enabled with
`--publish` (`cargo xtask canary enable --publish`); a preview started
without it answers on loopback alone, and says so. A pass fetches each new commit from
`origin` by its id (a pull request's head included), builds it once into
`~/.local/share/ren/builds` from its own worktree,
`~/.local/share/ren/preview-factory`, and starts it on a fresh copy of
production's history, as the canary is. A pull request whose head moves is
previewed again in its slot; one that closes, or a commit no longer asked
for, is stopped. A build or start that fails is recorded with why, its slot
left empty, and is not tried again for that commit. While production has
not seen its pull requests (no answer from GitHub yet), the previews of pull
requests are kept as they are. What runs is written to production's data
folder (`previews.json`); the commits asked for are read from
`preview-requests.json` beside it.

### Projects

A repository declares how Ren builds, serves, checks, and quiesces it in
`.ren/project.toml` at its root, and a worktree of it then gets checks and
a preview as Ren's own do:

```toml
packages = ["node"]

[[build]]
run = ["npm", "ci"]

[serve]
run = ["node", "build/index.js"]
env = { PORT = "{port}", HOST = "127.0.0.1" }
open = "http://127.0.0.1:{port}"
health = "/health"

[[check]]
name = "tests"
run = ["npm", "test"]

[quiesce]
run = ["curl", "-fsS", "-X", "POST", "http://127.0.0.1:{port}/drain"]

[deploy]
run = ["./deploy.sh", "production"]
```

Every command is an argv, never a shell line. Arguments and values may
name the worktree's first port `{port}`, another `{port+N}` (within its
100), its data folder `{data}`, and its folder `{worktree}` (`{workspace}`,
its name before the rename, still reads the same).

Every step, the serve unit's included, runs with Ren's package set (`ren
packages`, see [clients.md](clients.md#packages-for-agents)) first on its `PATH`,
then the caller's, as Ren's runs do: the current generation, named when
the command starts (`==> Packages: generation 3 (node 26.13.1, ...)`).
`packages` names the packages the steps need. One Ren installs must be in
that generation, so steps run the release Ren pins, not another copy on
the host; one Ren only adopts (git) must be on the host's `PATH`; and a
package the catalog does not know is added as a `[[packages]]` in the
daemon's `ren.toml`, as for Go:

```toml
[[packages]]
id = "go"
kind = "runtime"
program = "go"

[packages.release]
version = "1.25.4"

[[packages.release.artifact]]
platform = "linux_x64"
url = "https://go.dev/dl/go1.25.4.linux-amd64.tar.gz"
sha256 = "<64 hex digits from go.dev/dl>"
format = "tar_gz"
program_at = "go/bin/go"
```

A package lacking refuses `check`, `preview`, and `promote` before any step
runs, each with what to do (`ren packages install node`); in a canary pass it
fails the build check. `ren packages gc` keeps the generations runs use, not
the one a preview or canary serves with: after a change, start them again
before collecting.

```sh
ren project check                  # run the declared checks; fails when one does
ren project preview up --publish   # build, serve on the worktree's ports, record
ren project preview down --publish # quiesce, stop, take it off the work items
```

`check` runs in the worktree it is started in (or `--dir`; outside a
worktree a step that names a port does not run). `preview up` runs the
build steps, starts the serve step as `ren-preview-<worktree>-serve` (with
the caller's `PATH`, so programs installed in your home are found), waits
for its health path, and records the preview on the sent work items the
branch's commits name (`Work-Item: <id>`) or `--work` gives, through this
host's daemon. With `--publish` it publishes the port its page opens on, as
`install-vps` does: a free port is published, and one Serve already uses
for anything else, or exposes through Funnel, is refused; `down` removes
the route only while it still points at the preview. Like Ren's, it works
in a worktree read from `REN_WORKTREE` alone. Deleting the worktree's
session stops it and takes it off its work items. The dev slot's fake
projects declare one: a static page served by Python, deployed by copying
it into a folder under the canary's data.

A project Ren builds is registered first, by name
([the plan](plans/hierarchy.md)); `install-vps` registers its checkout as
`ren`, Ren itself, unless a project is already built as Ren (a promotion
installs from the factory's own worktree), and the dev slot registers its
fake projects:

```sh
ren project add --dir ~/dev/acme    # its folder's name unless --name; --factory contract by default
ren project list                    # name, factory, repository, archived
ren project archive acme            # keeps its name and repository; not built
ren project unarchive acme
ren project adopt acme              # its sessions from before it was registered
```

Adding a project already registered just so does nothing; a name or a
repository another project holds is refused. Then it gets a canary and
promotion too, by `--project`, or the project whose repository is the
current folder:

```sh
ren project canary --project acme   # fetch main, build, serve, check, deploy by policy
ren project promote --project acme  # deploy the canary's passed build now
ren project release --project acme  # main, production, the canary, and the decision
```

Ren itself (`--factory ren`) is refused there: its canary is `cargo xtask
canary`. Each pass and each promotion also closes the project's work items
as Ren's canary does: a sent item lands when a commit on `main` names it
(`Work-Item: <id>`), and a landed item ships once production runs a build
that holds it (in the next pass after it lands). A pass fetches the project's `main` (`--remote`, `--branch`) and checks its
tip out, detached, in a worktree the factory keeps for the project, in
Ren's worktrees folder, so its slot's ports are its own. It runs the build
steps, starts the serve step there and waits for its health path, then
runs each declared check against it; a check that cannot run fails. The
verdict, the build last deployed (none before the first promotion), and the
operator's policy make the project's release, kept in
`<data>/projects/<name>`. The policy is the project's table in
`~/.config/ren/release.toml`, by its registered name, manual unless named:

```toml
[projects.acme]
promote = "automatic" # or "manual", or a daily UTC window like "02:00-05:00"
```

When it promotes (a passed build that follows the one deployed), the
manifest's `[deploy]` step runs in the checked build, and production is then
that commit; a deploy that fails is marked and not tried again unless you
promote it. A project that declares no deploy step is never promoted. The
canary stays served on its worktree's port until the next pass.

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
`ren-ui.service` (see [the UI's sessions](../ui/README.md#sessions)); `install-vps` keeps
an existing `ui.env` as it is, only adding `REN_SOURCE_DIR` (the checkout it installs
from, where [Redline](../ui/README.md#redline) sends its briefs) when the file names none.
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
