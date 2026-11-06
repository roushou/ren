# Clients

How to point Claude Code, Codex, and Pi at TGV, and how TGV attributes their
traffic to sessions and turns.

Live subscription traffic has been exercised with Claude Code and Codex. Pi has
been validated against local mock providers; live Pi subscription access is
unvalidated.

## Diagnose and launch

With the daemon running, use session-only launch commands:

```sh
tgv doctor
tgv doctor --json
tgv run claude
tgv run codex -- --model <model-id>
tgv run pi -- --model anthropic/<model-id>
tgv run codex --dry-run
```

`doctor` checks local configuration, upstream URL validity, read-only history,
Unix storage mode bits, client availability on `PATH`, and the daemon's health
and storage APIs. It reports capture loss separately from storage failures,
and warns when the daemon runs a different build from the CLI (see
[builds](api.md#builds)).
It does not execute clients, read credential files, run credential helpers, test
provider connectivity, or consume subscription allowance. Missing clients are
warnings. Exit code is 0 with no failed checks (warnings can remain), 1 on a failed
check, and 2 for invalid CLI syntax. JSON output includes `ok` and `checks`.
These are readiness checks, not a full database-integrity or disk-space check.

`run` checks the gateway before launching, then applies only process-scoped
overrides. Claude receives its base URL and correlation-header flag. Codex gets
a complete `tgv_session` provider override with OpenAI authentication, and
`--no-daemon` keeps the session out of an already-running shared Codex server.
Existing model, approval, sandbox, sign-in, and credential-refresh behavior stays
owned by the client. TGV does not modify client configuration files or credentials.
On macOS/Linux the client replaces the launcher, preserving terminal interaction,
signals, and exit status. A normal client launch outside TGV restores normal routing.

Pass client arguments after `--`; TGV passes them literally without a shell.
Explicit client routing/auth options or managed client settings can supersede
the launcher's settings. API keys and token overrides already present in your
environment are retained, with a name-only warning: verify the client uses your
intended subscription login. `--dry-run` prints only TGV's generated arguments and
environment overrides, excluding inherited credentials and your client arguments;
it performs no network or client operations. Launch options target Claude Code
2.1.316, Codex CLI 0.164.0, and Pi 1.0.0.

For a remote daemon, save its trusted Tailscale HTTPS **root** URL once:

```sh
tgv init --gateway https://tgv.your-tailnet.ts.net
tgv run claude
tgv run codex -- --model <model-id>
```

`tgv init` checks the gateway's health and saves the URL only if the check
passes, to `$XDG_CONFIG_HOME/tgv/client.toml` (or `~/.config/tgv/client.toml`),
owner-only. Without `--gateway`, it asks on a terminal, offering the saved URL
as the default; with no terminal it fails instead of waiting. This file is
separate from the daemon's `tgv.toml`. `run` and `doctor` choose the gateway in
this order: `--gateway`, the `TGV_GATEWAY` environment variable, `client.toml`,
then the configured local listener. `doctor` reports which one it used. When
nothing names a gateway and the local daemon fails its preflight, `run` says so
and points to `tgv init`; no command prompts on its own.

A selected gateway skips local history/upstream checks; remote storage is
queried through the API. History commands (`sessions`, `history`, `inspect`)
still read local storage, so run them on the daemon's host.
Root URLs cannot contain credentials, queries, fragments, or provider suffixes;
HTTP is allowed only on loopback. Diagnostic requests send no provider credentials,
disable redirects/retries/environment proxies, and cap each response at 64 KiB
and five seconds. Client requests still carry client-owned credentials to the
selected gateway. No inference is sent by `doctor` or launch preflight.

The launch mechanism follows [Claude's gateway protocol](https://code.claude.com/docs/en/llm-gateway-protocol),
[Codex's per-run configuration overrides](https://learn.chatgpt.com/docs/config-file/config-advanced#one-off-overrides-from-the-cli),
and [OpenAI authentication for custom providers](https://learn.chatgpt.com/docs/auth#alternative-model-providers).

## Native endpoints

| Client endpoint                            | Upstream path                          |
| ------------------------------------------ | -------------------------------------- |
| `POST /anthropic/v1/messages`              | `/v1/messages`                         |
| `POST /anthropic/v1/messages/count_tokens` | `/v1/messages/count_tokens`            |
| `GET /anthropic/v1/models`                 | `/v1/models`                           |
| `POST /codex/responses`                    | `/backend-api/codex/responses`         |
| `POST /codex/responses/compact`            | `/backend-api/codex/responses/compact` |
| `GET /codex/models`                        | `/backend-api/codex/models`            |
| `POST /openai/v1/responses`                | `/v1/responses`                        |
| `POST /openai/v1/responses/compact`        | `/v1/responses/compact`                |
| `GET /openai/v1/models`                    | `/v1/models`                           |

The configured upstream bases determine the upstream path prefix. Query strings
and end-to-end headers are forwarded, including subscription authentication and
provider capability headers. Hop-by-hop headers are removed and Host is set for
the configured upstream. Redirects, automatic retries, response decompression,
and implicit environment proxies are disabled.

TGV does not sign in, refresh tokens, or substitute an API credential. The client
owns authentication. No requests are made to providers merely by starting TGV.

## Claude Code

With an existing subscription login, launch a session through TGV:

```sh
ANTHROPIC_BASE_URL=http://127.0.0.1:8787/anthropic CLAUDE_CODE_GATEWAY_HINT_HEADERS=1 claude
```

An active API key, bearer-token override, or credential helper can supersede that
login. Keep the subscription login active rather than supplying a replacement
credential for TGV. This is the subscription-preserving gateway configuration
[documented by Anthropic](https://code.claude.com/docs/en/llm-gateway-protocol#request-headers).

## Codex

Merge a provider entry into your Codex configuration:

```toml
model_provider = "tgv"

[model_providers.tgv]
name = "TGV"
base_url = "http://127.0.0.1:8787/codex"
wire_api = "responses"
requires_openai_auth = true
supports_websockets = true
```

Keep your existing ChatGPT sign-in and model selection. TGV relays Codex's
Responses WebSocket and HTTP/SSE alike; `tgv run codex` offers both. The
WebSocket is relayed message for message: each client and upstream message
passes unchanged and in order, each side answers its own pings, and no
compression extension is negotiated on either leg. To keep Codex on HTTP/SSE,
set `[websocket] mode = "refuse"`: handshakes then get `426 Upgrade Required`,
and Codex falls back to HTTP/SSE for the session. Handshakes beyond
`websocket.max_channels` (default 16) get the same `426`. Each call on a socket
counts against `max_concurrent_requests`; an idle socket does not. On shutdown,
idle sockets close with code `1012` (service restart); sockets with a call still
running close the same way once it ends, or when the drain deadline passes. See
OpenAI's [gateway authentication guide](https://learn.chatgpt.com/docs/enterprise/connect-to-a-gateway)
and [configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference).

## Pi

Start Pi with its existing sign-in and model catalog:

```sh
tgv run pi -- --model anthropic/<model-id>
tgv run pi -- --model openai/<model-id>
tgv run pi -- --model openai-codex/<model-id>
tgv run pi --gateway https://tgv.your-tailnet.ts.net -- --model anthropic/<model-id>
tgv run pi --dry-run
```

The embedded extension redirects **`anthropic`**, **`openai`**, and
**`openai-codex`** to TGV, forces SSE for these providers, and delegates request
conversion, authentication, token refresh, tool replay, and retries to Pi. Sign in
through Pi's own `/login` for the provider you select. TGV does not import Claude
Code/Codex credentials, create a login, inspect tokens, or supply an API key.
Existing Pi API-key overrides still apply; the launcher itself does not guarantee
subscription billing.

Pi 1.0.0 calls `openai-codex` its ChatGPT provider; its **`openai`** provider uses
Sign in with ChatGPT and the public Responses endpoint. TGV keeps these
destinations separate:

| Pi provider    | Gateway base URL | Default upstream                        |
| -------------- | ---------------- | --------------------------------------- |
| `anthropic`    | `/anthropic`     | `https://api.anthropic.com`             |
| `openai-codex` | `/codex`         | `https://chatgpt.com/backend-api/codex` |
| `openai`       | `/openai/v1`     | `https://api.openai.com/v1`             |

Set `openai_base_url` to the public OpenAI upstream **root** (default
`https://api.openai.com`). Route availability does not guarantee that a
subscription permits every endpoint. TGV records this destination as `openai`,
separately from `codex`.

For `openai`, the extension redirects Pi's HTTP fetch while retaining the native
model URL during serialization. Pi checks that URL when omitting fields unsupported
by ChatGPT sign-in; changing it directly would disable that handling. TGV delegates
serialization to Pi and does not rewrite the generated payload. Sign-in and refresh
requests still go directly from Pi to OpenAI. The endpoint and bearer-token flow
follow [OpenAI's ChatGPT plan inference documentation](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference).

Other providers keep their normal endpoints. Select a supported provider explicitly;
an existing default for another provider will bypass TGV. Later extensions or custom
provider registrations can supersede this adapter. Explicit `--extension` loading
still works with Pi's `--no-extensions`, which disables discovery.

The launcher extracts a hash-named extension into `$XDG_CACHE_HOME/tgv/pi` (or
`$HOME/.cache/tgv/pi`) and loads it for that Pi process. This leaves Pi's
configuration and credentials untouched and lets `/reload` reuse the same code.
Cache files are published atomically and checked against the bundled source on each
launch; symlinks and group/other-writable cache paths are rejected. `--dry-run`
shows an extension-path placeholder and performs no cache writes. Unused versions
can be removed once their Pi sessions stop. No Node/Bun installation is needed to
build TGV; the installed Pi runtime supplies the extension's provider APIs.

Pi sessions use the client's UUID. Each agent run gets a fresh extension-generated
turn UUID shared by tool follow-ups and retries. It is not a native Pi turn ID or
proof of tool execution. Session switches reset the run; requests outside a run
have no turn, and requests for another session do not inherit the active run.
TGV records these hints under `pi_extension_headers`, distinct from Claude Code
and Codex. The `x-tgv-pi-session-id` and `x-tgv-pi-turn-id` headers are consumed by
TGV and stripped before forwarding. Correlation remains scoped by upstream provider,
so switching providers creates separate TGV session groups for the same Pi UUID.

Verify with the installed Pi CLI without real credentials or inference charges:

```sh
cargo build --locked
node --test crates/tgv-harness/pi/tgv.test.mjs
python3 scripts/smoke-pi.py
```

The smoke test exercises all three provider paths, stored OAuth-shaped
authentication, tool invocation/results, resumed sessions, turn grouping, unchanged
auth files, and removal of internal headers. For public OpenAI it also checks that
Pi omits ChatGPT-incompatible request fields. Everything uses temporary state and
mock providers. It does not validate live entitlement or token-refresh network
calls. The adapter follows Pi's [provider extension contract](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/custom-provider.md).

## Sessions and turns

TGV assigns its own typed session and turn IDs while retaining native identifiers
and their source in each call's `attribution`. Claude Code uses
`x-claude-code-session-id` and `x-claude-code-prompt-id`; the environment flag above
enables prompt hints for custom gateways. These headers are
[documented by Anthropic](https://code.claude.com/docs/en/llm-gateway-protocol#request-headers).
Codex CLI 0.164.0 sends `session-id`, `thread-id`, and a `turn_id` inside
`x-codex-turn-metadata`; this integration is version-sensitive.

Sessions are scoped by provider, metadata source, and native session ID. Turns
are scoped to their session. Tool follow-up requests carrying the same IDs stay
in one turn. Native thread IDs are retained separately for Codex session trees.
Missing identifiers remain `null`; malformed, duplicate, or conflicting metadata
is flagged. TGV does not group by prompt text, cache keys, time, account, or request
IDs. These client-supplied hints are attribution evidence, never authentication,
proof of tool execution, or a signal that a session is idle or complete.

Correlation headers are parsed once, on arrival, into bounded typed hints (Codex
turn metadata is read only up to 8 KiB). Only allowlisted UUIDs are stored; the full
Codex metadata object and arbitrary headers are discarded. Correlation works with
metadata-only body capture.
