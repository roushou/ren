# Policy, failover, and operator controls

All of this is optional; with no `[policy]` rules and no `[[upstreams]]` entries,
TGV forwards every call to the provider's own upstream with the client's own
credentials. Every decision is recorded with the call: the operator state revision,
the plan (candidates, transform rules, fallback rule), each attempt's upstream and
changes, and why a call was refused. Header values are never recorded.

## Request buffer

`[buffer]`: a request over its provider's limit in `[buffer.request_limits]` gets
`413`. The defaults are the providers' documented limits: `anthropic` 32 MiB
(Anthropic's 32 MB Messages limit), `openai` 512 MiB (OpenAI's 512 MB request
payload limit), and `codex` 512 MiB (the Codex backend documents none, so OpenAI's
is used). All buffered requests together hold at most `budget_bytes` (default
1 GiB); past that, new requests get `503` with `retry-after` at once. Memory is used
only by requests in progress, but one large OpenAI request can hold up to 512 MiB:
on a small host, lower `openai`, `codex`, and `budget_bytes`. Uploads that pause
longer than `idle_timeout_seconds` (30) or take longer than
`total_timeout_seconds` (300) get `408`. Refusals are recorded as calls with a
`rejection` and no attempt.

## Rules

`[policy]`: a `block` rule refuses matching calls with a non-retryable `403`. A
`transform` rule changes matching HTTP requests with `set_header`,
`remove_header`, `json_set`, or `json_remove` (JSON Pointer) steps. Credentials,
client identity (`user-agent`, `originator`, `x-app`), beta headers, framing
headers, and identity or position-sensitive body fields (`/system`, `/metadata`,
`/betas`, `/previous_response_id`, `/client_metadata`) are protected: a rule that
touches them is refused when the configuration loads. A changed body is stored as
the attempt's own request beside the client's original. Body steps do not run on a
compressed request. [`tgv.example.toml`](../tgv.example.toml) shows each rule type.

## Failover

`[[upstreams]]` and `[[policy.fallback]]`: subscription to API-key failover is
opt-in and bills per token. An extra upstream entry holds an operator API key read
from the daemon's environment at start, never from the file:

```toml
[[upstreams]]
id = "anthropic-key"
provider = "anthropic"
base_url = "https://api.anthropic.com"
operator_key_env = "TGV_ANTHROPIC_API_KEY"

[[policy.fallback]]
id = "plan-limit"
provider = "anthropic"
to = "anthropic-key"
on_status = [429]
```

When the provider's own upstream answers a listed status before anything reached
the client, TGV records that response and replays the request to the fallback
entry, replacing the client's credentials (and the OAuth beta) with the key. A
request the upstream provably did not accept (`not_sent` or `refused`) is replayed
the same way, whatever status the rule lists. A request that may have been accepted
(a timeout, a broken connection) or whose response already started is never
replayed. Codex accepts no operator keys, and WebSocket calls never fail over. A
background service must get the key variable from its own environment (a systemd
`EnvironmentFile`, or launchd `EnvironmentVariables`).

## Operator controls

With the daemon running:

```sh
tgv control state
tgv control pause --retry-after 30
tgv control resume
tgv control block <session-uuid> --for 600
tgv control unblock <session-uuid>
tgv control cancel <call-id> --block-session 60
tgv control backup
```

A pause answers every new call with `503` and `retry-after`, so clients wait. A
blocked session's calls get a non-retryable `403`. Cancelling a call before its
response starts answers it with a TGV error (`409`, `x-should-retry: false`);
once output is flowing, an HTTP stream is interrupted, and a Codex WebSocket call
is sent `response.interrupt`, with the answer kept from the client. Paused and
blocked Codex WebSocket calls get an `error` event of type `tgv_*`. See
[Recording](recording.md#backup) for `backup`.

Pauses and session blocks survive a restart: the daemon restores the state the
last recorded command made, logs a warning saying so, and keeps counting
revisions. Run `tgv control resume` or `unblock` to clear them. A command is
durable once the recorder commits it, shortly after the API answers; a crash in
between loses the command and its record together. If the stored state cannot be
read, the daemon refuses to start rather than starting unpaused.

The control API (`/control/v1`) accepts a request carrying the per-run token from
`control.token` in the data directory (the CLI reads it), or one from Tailscale
Serve whose `Tailscale-User-Login` is listed in `control.tailscale_logins`.
Loopback alone is not enough, because Serve forwards tagged devices and Funnel
traffic to loopback without an identity. Requests with `Origin` or
`Sec-Fetch-Site` (browsers) are refused. The UI has no control actions.

`tgv control` targets the daemon the history commands target. On the daemon's
host it sends the token. Through a gateway (`--gateway`, `TGV_GATEWAY`, or
`client.toml`) it sends none, so it works only through Tailscale Serve and only
for a login in the daemon's `tailscale_logins`:

```toml
[control]
tailscale_logins = ["you@example.com"]
```

`tailscale status --json` shows your login (`LoginName` under `User`). Restart
the daemon after changing it. A refusal for a missing identity or an unlisted
login says which one it was.
