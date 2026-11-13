<div align="center">

<img src="docs/wordmark.svg" alt="Ren" height="60">

<h3>Self-hosted gateway and flight recorder for coding agents</h3>

See every request, response, tool call, and token from Claude Code, Codex, Pi, and OpenCode
as it happens, without changing a byte of the traffic.

[![CI](https://github.com/roushou/ren/actions/workflows/ci.yml/badge.svg)](https://github.com/roushou/ren/actions/workflows/ci.yml)
[![License: MIT or Apache-2.0](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue)](#license)

[Install](#install) · [Clients](docs/clients.md) · [Deploy](#deploy) ·
[Architecture](docs/architecture.md)

</div>

<img alt="Live" src="https://github.com/user-attachments/assets/964c0b9d-a14d-4cb0-a73e-6767e806c271" />

## Overview

Ren sits between your coding agents and their providers. `ren run claude` (or
`codex`, `pi`, or `opencode`) points a client at Ren for one session without editing its
configuration. Ren forwards each request unchanged with the client's own
sign-in, records the request and response on the side, and serves that history
to the CLI, a read-only API, and the optional web UI, on the same machine or
from another one on your tailnet.

## What you get

- **Live traffic:** every call as it streams, grouped by recording and turn.
- **Full inspection:** readable or raw requests and responses, complete bodies to
  export.
- **Usage:** calls, errors, and tokens by hour, day, model, and recording.
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

The executable is `ren`; the crates.io package is named `ren-rs`.

With the Rust toolchain pinned in `rust-toolchain.toml`:

```sh
cargo install --path crates/ren-cli --locked
```

## Run

```sh
ren serve
```

Ren listens on `127.0.0.1:8787` and stores data in `$XDG_STATE_HOME/ren` (or
`~/.local/state/ren`). [`ren.example.toml`](ren.example.toml) documents the
configuration (`ren --config <file> serve`).

Launch a client through it, keeping the client's own sign-in:

```sh
ren doctor
ren run claude
ren run codex
ren run pi -- --model anthropic/<model-id>
ren run opencode
```

When the daemon runs on another machine, such as a VPS on your tailnet, save its
URL once on the machine you code from, and the same commands use it:

```sh
ren init --gateway https://your-vps.your-tailnet.ts.net:8443
```

Browse what was recorded:

```sh
ren sessions
ren history --limit 10
ren inspect <call-id>
ren tail
```

With a saved gateway, these read the remote daemon.

Let your agents look it up themselves through the
[MCP server](docs/api.md#mcp-server):

```sh
claude mcp add --scope user ren -- ren mcp
```

The optional UI lives in [`ui`](ui/README.md).

## Deploy

`ren serve` runs in the foreground; for a background service, pick one:

| Where                          | How                                                               | Guide                                                       |
| ------------------------------ | ----------------------------------------------------------------- | ----------------------------------------------------------- |
| macOS                          | launchd agent, from `ren service render --manager launchd`        | [launchd](docs/deployment.md#macos-launchd)                 |
| Linux                          | systemd user service, from `ren service render --manager systemd` | [systemd](docs/deployment.md#linux-systemd)                 |
| VPS on your tailnet            | `cargo xtask install-vps --https-port 8443`                       | [VPS with Tailscale](docs/deployment.md#vps-with-tailscale) |
| Docker or Podman               | `docker compose up -d --build` in `deploy/container`              | [Compose](docs/deployment.md#compose)                       |
| Podman, rootless under systemd | Quadlet files in `deploy/container/quadlet`                       | [Podman Quadlet](docs/deployment.md#podman-quadlet)         |

The daemon always listens on loopback. Remote clients reach it through
Tailscale Serve, from the host or from a sidecar container; Ren has no public
listener.

## Screenshots

<details>
<summary>Light theme</summary>

<img alt="Light" src="https://github.com/user-attachments/assets/34745d90-2de2-44d4-9862-6c0f7c799a1a" />
</details>

<details>
<summary>Traffic</summary>

<img alt="Traffic" src="https://github.com/user-attachments/assets/353fcb72-b273-41c4-866a-157d05e0771f" />
</details>

<details>
<summary>Recordings</summary>

<img width="1470" height="838" alt="Recordings" src="https://github.com/user-attachments/assets/ca3eede3-7045-415a-bd64-61d3fe778b32" />
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
- [Architecture](docs/architecture.md): how Ren is built
- [AGENTS.md](AGENTS.md): contributor guidance

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
