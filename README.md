<div align="center">

<img src="docs/wordmark.svg" alt="TGV" height="60">

<h3>Self-hosted gateway and flight recorder for coding agents</h3>

See every request, response, tool call, and token from Claude Code, Codex, Pi, and OpenCode
as it happens, without changing a byte of the traffic.

[![CI](https://github.com/roushou/tgv/actions/workflows/ci.yml/badge.svg)](https://github.com/roushou/tgv/actions/workflows/ci.yml)
[![License: MIT or Apache-2.0](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue)](#license)

[Install](#install) · [Clients](docs/clients.md) · [Deploy](#deploy) ·
[Architecture](docs/architecture.md)

</div>

<img alt="TGV's Traffic view in the dark theme: a session's calls, with a response rendered in the inspector" src="https://github.com/user-attachments/assets/799b9c5b-6cc3-479b-afcf-d00db5263211" />

## Overview

TGV sits between your coding agents and their providers. `tgv run claude` (or
`codex`, `pi`, or `opencode`) points a client at TGV for one session without editing its
configuration. TGV forwards each request unchanged with the client's own
sign-in, records the request and response on the side, and serves that history
to the CLI, a read-only API, and the optional web UI, on the same machine or
from another one on your tailnet.

## What you get

- **Live traffic:** every call as it streams, grouped by session and turn.
- **Full inspection:** readable or raw requests and responses, complete bodies to
  export.
- **Usage:** calls, errors, and tokens by hour, day, model, and session.
- **Truthful outcomes:** interrupted and unknown endings never count as success.
- **Faithful forwarding:** payloads pass through unchanged unless you configure a
  transform, which is recorded; capture loss is counted.
- **Named users:** who made each call, by Tailscale login or user token, with
  per-user limits and blocks.
- **Operator controls:** pause, block, or cancel from the CLI or another machine.
- **Opt-in failover:** retry a rate-limited call with your own API key for that
  provider (not Codex), only when that is safe.
- **Your machine, your data:** your clients' own sign-in; history stays on your
  host.

## Install

With the Rust toolchain pinned in `rust-toolchain.toml`:

```sh
cargo install --path crates/tgv-cli --locked
```

Add `--features tui` for `tgv tui`, a terminal view of live sessions.

## Run

```sh
tgv serve
```

TGV listens on `127.0.0.1:8787` and stores data in `$XDG_STATE_HOME/tgv` (or
`~/.local/state/tgv`). [`tgv.example.toml`](tgv.example.toml) documents the
configuration (`tgv --config <file> serve`).

Launch a client through it, keeping the client's own sign-in:

```sh
tgv doctor
tgv run claude
tgv run codex
tgv run pi -- --model anthropic/<model-id>
tgv run opencode
```

When the daemon runs on another machine, such as a VPS on your tailnet, save its
URL once on the machine you code from, and the same commands use it:

```sh
tgv init --gateway https://your-vps.your-tailnet.ts.net:8443
```

Browse what was recorded:

```sh
tgv sessions
tgv history --limit 10
tgv inspect <call-id>
tgv tail
tgv tui          # built with --features tui
```

With a saved gateway, these read the remote daemon.

Let your agents look it up themselves through the
[MCP server](docs/api.md#mcp-server):

```sh
claude mcp add --scope user tgv -- tgv mcp
```

The optional UI lives in [`ui`](ui/README.md).

## Deploy

`tgv serve` runs in the foreground; for a background service, pick one:

| Where                          | How                                                               | Guide                                                       |
| ------------------------------ | ----------------------------------------------------------------- | ----------------------------------------------------------- |
| macOS                          | launchd agent, from `tgv service render --manager launchd`        | [launchd](docs/deployment.md#macos-launchd)                 |
| Linux                          | systemd user service, from `tgv service render --manager systemd` | [systemd](docs/deployment.md#linux-systemd)                 |
| VPS on your tailnet            | `cargo xtask install-vps --https-port 8443`                       | [VPS with Tailscale](docs/deployment.md#vps-with-tailscale) |
| Docker or Podman               | `docker compose up -d --build` in `deploy/container`              | [Compose](docs/deployment.md#compose)                       |
| Podman, rootless under systemd | Quadlet files in `deploy/container/quadlet`                       | [Podman Quadlet](docs/deployment.md#podman-quadlet)         |

The daemon always listens on loopback. Remote clients reach it through
Tailscale Serve, from the host or from a sidecar container; TGV has no public
listener.

## Screenshots

<details>
<summary>Light theme</summary>

<img alt="TGV's Traffic view in the light theme" src="https://github.com/user-attachments/assets/58a6a4aa-265a-49ef-ac98-c8f7ee76b3be" />
</details>

<details>
<summary>Overview</summary>

<img width="2880" height="1800" alt="TGV's Overview: health, live calls, problems, and the last 24 hours" src="https://github.com/user-attachments/assets/9762a3c8-ae96-4de4-be7e-6cc0039c8703" />
</details>

<details>
<summary>Sessions</summary>

<img width="2880" height="1800" alt="TGV's Sessions: calls, turns, errors, and tokens per session" src="https://github.com/user-attachments/assets/3888ef16-6e8b-476a-b5a0-3f08105d1b64" />
</details>

## Documentation

- [Clients](docs/clients.md): launching and configuring Claude Code, Codex, Pi, and OpenCode
- [Recording](docs/recording.md): capture, inspection, backup, and retention
- [History and API](docs/api.md): CLI history commands, the MCP server for
  agents, and the management API
- [Policy](docs/policy.md): buffering, rules, failover, named users, and operator
  controls
- [Deployment](docs/deployment.md): launchd, systemd, containers (Docker and
  Podman), and Tailscale
- [Architecture](docs/architecture.md): how TGV is built
- [AGENTS.md](AGENTS.md): contributor guidance

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
