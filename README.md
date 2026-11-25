<div align="center">

<img src="docs/wordmark.svg" alt="Ren" height="60">

<h3>A self-hosted software factory</h3>

Agents write the changes on your repositories, every branch gets a live
preview, and main reaches production through a canary. Every model call is
recorded.

[![CI](https://github.com/roushou/ren/actions/workflows/ci.yml/badge.svg)](https://github.com/roushou/ren/actions/workflows/ci.yml)
[![License: MIT or Apache-2.0](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue)](#license)

[Install](#install) · [Clients](docs/clients.md) · [Deploy](#deploy) ·
[Architecture](docs/architecture.md)

</div>

<img alt="Live" src="https://github.com/user-attachments/assets/964c0b9d-a14d-4cb0-a73e-6767e806c271" />

## Overview

Ren runs coding agents (Claude Code, Codex, Pi, and OpenCode) on the
repositories you register as projects. A project's work items go to agent
sessions; each branch gets a preview beside production; a canary builds,
serves, and checks main before a release is promoted. Every call the agents
make passes through Ren's gateway unchanged, with the client's own sign-in,
and is recorded, so you can see what each session did and what it cost.

Ren's daemon, UI, and agents run on your host, or on a VPS on your tailnet that
you reach from the machines you code on.

## What you get

**Build**

- **Projects:** a repository declares how it is built, served, checked, and
  deployed in `.ren/project.toml`.
- **Work items and sessions:** each work item is taken up by an agent session,
  in the project's folder or a worktree of its own.
- **Previews:** every pull request and every worktree's branch, served beside
  production.
- **Releases:** a canary builds and checks main; your policy or `promote`
  deploys it. Runs outlive the daemon, so an upgrade doesn't stop an agent.
- **Contained runs:** optionally, each agent run in a rootless Podman container
  of its own.
- **Packages:** runtimes and tools for the agents, verified and installed in
  generations you can roll back.

**Inspect**

- **Live traffic:** every call as it streams, grouped by recording and turn.
- **Full inspection:** readable or raw requests and responses, complete bodies to
  export.
- **Usage:** calls, errors, and tokens by hour, day, model, and recording.
- **Truthful outcomes:** interrupted and unknown endings never count as success.
- **Faithful forwarding:** payloads pass through unchanged unless you configure a
  transform, which is recorded; capture loss is counted.

**Operate**

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

Register a project and give an agent work in it:

```sh
ren project add --dir ~/dev/acme
ren runs start claude ~/dev/acme --checkout worktree --prompt "fix the failing test"
ren project release --project acme
```

Or launch a client through the gateway yourself, keeping its own sign-in:

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
ren recordings
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

| Where                          | How                                                                       | Guide                                                       |
| ------------------------------ | ------------------------------------------------------------------------- | ----------------------------------------------------------- |
| macOS                          | launchd agent, from `ren service render`                                  | [launchd](docs/deployment.md#macos-launchd)                 |
| Linux                          | systemd user service, from `ren service render`                           | [systemd](docs/deployment.md#linux-systemd)                 |
| VPS on your tailnet            | `cargo run --release -p ren-rs -- deployer install-vps --https-port 8443` | [VPS with Tailscale](docs/deployment.md#vps-with-tailscale) |
| Docker or Podman               | `docker compose up -d --build` in `deploy/container`                      | [Compose](docs/deployment.md#compose)                       |
| Podman, rootless under systemd | Quadlet files in `deploy/container/quadlet`                               | [Podman Quadlet](docs/deployment.md#podman-quadlet)         |

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

- [Projects](docs/deployment.md#projects): `.ren/project.toml`, previews, canary,
  and promotion
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
