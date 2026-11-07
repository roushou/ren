<div align="center">

<img src="docs/wordmark.svg" alt="TGV" height="60">

<h3>Self-hosted gateway and flight recorder for coding agents</h3>

See every request, response, tool call, and token from Claude Code, Codex, and Pi
as it happens, without changing a byte of the traffic.

[![CI](https://github.com/roushou/tgv/actions/workflows/ci.yml/badge.svg)](https://github.com/roushou/tgv/actions/workflows/ci.yml)
[![License: MIT or Apache-2.0](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue)](#license)

[Install](#install) · [Clients](docs/clients.md) · [Deployment](docs/deployment.md) ·
[Architecture](docs/architecture.md)

</div>

<img alt="TGV's Traffic view in the dark theme: a session's calls, with a response rendered in the inspector" src="https://github.com/user-attachments/assets/799b9c5b-6cc3-479b-afcf-d00db5263211" />

## Overview

TGV sits between your coding agents and their providers. `tgv run claude` (or
`codex`, or `pi`) points a client at TGV for one session without editing its
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
- **Faithful forwarding:** payloads pass through unchanged; capture loss is
  counted.
- **Operator controls:** pause, block, or cancel from the CLI or another machine.
- **Opt-in failover:** retry a rate-limited call with an Anthropic API key, only
  when that is safe.
- **Your machine, your data:** your clients' own sign-in; history stays on your
  host.

## Install

With the Rust toolchain pinned in `rust-toolchain.toml`:

```sh
cargo install --path crates/tgv-cli --locked
```

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
```

With a saved gateway, these read the remote daemon.

The optional UI lives in [`ui`](ui/README.md).

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

- [Clients](docs/clients.md): launching and configuring Claude Code, Codex, and Pi
- [Recording](docs/recording.md): capture, inspection, backup, and retention
- [History and API](docs/api.md): CLI history commands and the management API
- [Policy](docs/policy.md): buffering, rules, failover, and operator controls
- [Deployment](docs/deployment.md): launchd, systemd, and Tailscale
- [Architecture](docs/architecture.md): how TGV is built
- [AGENTS.md](AGENTS.md): contributor guidance

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
