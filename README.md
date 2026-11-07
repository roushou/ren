# TGV

A personal LLM gateway for Claude Code, Codex, and Pi. TGV forwards native
provider traffic unchanged, using your own Anthropic and OpenAI subscriptions,
and records it for inspection through a CLI, a read-only API, and an optional
web UI. It runs on a laptop or on a VPS reached through Tailscale.

<img width="2880" height="1800" alt="tgv-dark" src="https://github.com/user-attachments/assets/799b9c5b-6cc3-479b-afcf-d00db5263211" />

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

<img width="2880" height="1800" alt="tgv-light" src="https://github.com/user-attachments/assets/58a6a4aa-265a-49ef-ac98-c8f7ee76b3be" />
</details>

<details>
<summary>Overview</summary>

<img width="2880" height="1800" alt="tgv-overview" src="https://github.com/user-attachments/assets/9762a3c8-ae96-4de4-be7e-6cc0039c8703" />
</details>

<details>
<summary>Sessions</summary>

<img width="2880" height="1800" alt="tgv-sessions" src="https://github.com/user-attachments/assets/3888ef16-6e8b-476a-b5a0-3f08105d1b64" />
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
