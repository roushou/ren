# Policy, failover, and operator controls

All of this is optional; with no `[policy]` rules and no `[[upstreams]]` entries,
Ren forwards every call to the provider's own upstream with the client's own
credentials. Every decision is recorded with the call: the operator state revision,
the plan (candidates, transform rules, fallback rule), each attempt's upstream and
changes, and why a call was refused. Header values are never recorded.

## Request buffer

`[buffer]`: a request over its provider's limit in `[buffer.request_limits]` gets
`413`. The defaults are the providers' documented limits: `anthropic` 32 MiB
(Anthropic's 32 MB Messages limit), `openai` 512 MiB (OpenAI's 512 MB request
payload limit), `codex` 512 MiB (the Codex backend documents none, so OpenAI's
is used), and `deepseek` 32 MiB (DeepSeek documents none for its
Anthropic-compatible API, so Anthropic's Messages limit is used). All buffered requests together hold at most `budget_bytes` (default
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
`/betas`, `/previous_response_id`, `/client_metadata`, and Chat Completions'
`/user` and `/safety_identifier`) are protected: a rule that
touches them is refused when the configuration loads. A changed body is stored as
the attempt's own request beside the client's original. Body steps do not run on a
compressed request. A rule's `match` takes `provider`, `endpoint`, `model` (the
request's top-level `model`, exactly), `user` (a [named user](#named-users)),
and `upstream` (the entry the call goes to first: its route's entry, never a
fallback). Every field given must match, and at least one is required.
[`ren.example.toml`](../ren.example.toml) shows each rule type.

## Failover

`[[upstreams]]` and `[[policy.fallback]]`: subscription to API-key failover is
opt-in and bills per token. An extra upstream entry holds an operator API key read
from the daemon's environment at start, never from the file:

```toml
[[upstreams]]
id = "anthropic-key"
provider = "anthropic"
base_url = "https://api.anthropic.com"
operator_key_env = "REN_ANTHROPIC_API_KEY"

[[policy.fallback]]
id = "plan-limit"
provider = "anthropic"
to = "anthropic-key"
on_status = [429]
```

An entry speaks its provider's own wire format. `wire` names it explicitly
(`anthropic_messages`, `openai_responses`, or `openai_chat_completions`); a format
the provider does not offer is refused when the configuration loads. Chat
Completions is offered by `openai`, `deepseek` (its OpenAI-compatible API), and
`anthropic` (its OpenAI SDK compatibility layer, which Anthropic documents for
testing rather than production); `codex` speaks only Responses. A Chat
Completions entry is reached through its [direct routes](#direct-routes): no
provider route speaks it.

When the provider's own upstream answers a listed status before anything reached
the client, Ren records that response and replays the request to the fallback
entry, replacing the client's credentials (and the OAuth beta) with the key. A
request the upstream provably did not accept (`not_sent` or `refused`) is replayed
the same way, whatever status the rule lists. A request that may have been accepted
(a timeout, a broken connection) or whose response already started is never
replayed. Codex accepts no operator keys, and WebSocket calls never fail over. A
background service must get the key variable from its own environment (a systemd
`EnvironmentFile`, or launchd `EnvironmentVariables`).

## Direct routes

An operator-key upstream is reached only through a fallback rule unless it opts
in to routes of its own:

```toml
[[upstreams]]
id = "anthropic-key"
provider = "anthropic"
base_url = "https://api.anthropic.com"
operator_key_env = "REN_ANTHROPIC_API_KEY"
ingress = "direct"
```

The entry is then mounted at `/upstreams/anthropic-key`, with its wire format's
native paths below it (`/v1/messages`, `/v1/messages/count_tokens`, and
`/v1/models` for `anthropic_messages`; `/v1/responses`, `/v1/responses/compact`,
and `/v1/models` for `openai_responses`, whose WebSocket mode is refused;
`/v1/chat/completions` and `/v1/models` for `openai_chat_completions`). Every
call there uses the operator key in place of the client's credentials, so any
caller that can reach the gateway bills the key: leave `ingress` out unless that
is intended, or name the [users](#named-users) who may use the entry. Block and transform rules apply as on the provider's own routes;
fallback rules do not, since the call already uses an operator key. Chat
Completions takes the key as `Authorization: Bearer`, for every provider. Calls are
recorded with the entry's provider and wire format, and the attempt with the
entry's id.

## Named users

`[[users]]` names the people and machines that use the gateway (see
[`ren.example.toml`](../ren.example.toml)); a user is not a harness, which
Ren attributes separately. Each forwarded call, and each WebSocket channel
with its calls, records its caller:

- `user_tailscale`: a user, by a Tailscale login it lists. Tailscale Serve
  adds `Tailscale-User-Login` and removes any copy the caller sent.
- `user_token`: a user, by its token in `X-Ren-User-Token`.
  `ren users token <id>` prints a new token once, with the `token_blake3`
  line that holds its digest; the configuration never holds the token.
  `ren run` sends it from `REN_USER_TOKEN` (see
  [Clients](clients.md#named-users)).
- `unlisted`: a Tailscale login no user lists, kept as it came.
- `unidentified`: no identity. Tailscale Serve sends tagged devices and Funnel
  traffic without one, and a process on this host can reach the loopback
  listener directly, so this is not evidence that the call came from this
  host.

Both headers are removed before forwarding, so neither reaches a provider. A
token no user has, more than one token, a malformed login, or a login and
token that name different users is refused with a non-retryable `401`
(`ren_identity_refused`), recorded as rejection `identity_refused`; it is
never ignored, so a caller cannot believe it is identified when it is not.
Identity protects against other machines, not this one: a local process can
set any header, including `Tailscale-User-Login`.

A user's `max_concurrent` (1–1024) caps the calls it may have running at once,
and the top-level `max_concurrent_unnamed` caps those of every caller that is
not a named user, together. A call over its caller's limit gets `429` with
`retry-after: 1` (`ren_user_busy`), recorded as rejection `user_busy`; a
WebSocket channel counts as one call for its whole life, and a handshake over
the limit gets `426`, so Codex falls back to HTTP. A caller's limit
is checked before the gateway's own `max_concurrent_requests`, which still applies
to everyone: a caller over its limit gets `429` even when the gateway is full.

An operator-key entry can be limited to some users:

```toml
[[upstreams]]
id = "team-key"
provider = "anthropic"
base_url = "https://api.anthropic.com"
operator_key_env = "REN_ANTHROPIC_API_KEY"
ingress = "direct"
users = ["alice", "ci"]
```

Any other caller, named or not, is refused on the entry's direct routes with a
non-retryable `403` (`ren_user_not_allowed`), recorded as rejection
`user_not_allowed`, and a fallback rule to the entry is not offered to them:
their calls stay on the provider's own entry. Every name must be a configured
user, and the list cannot be empty; leave it out to admit anyone.

## Operator controls

With the daemon running:

```sh
ren control state
ren control pause --retry-after 30
ren control resume
ren control block <session-uuid> --for 600
ren control unblock <session-uuid>
ren control block-user <user-id> --for 3600
ren control unblock-user <user-id>
ren control cancel <call-id> --block-session 60
ren control backup
```

A pause answers every new call with `503` and `retry-after`, so clients wait. A
blocked session's calls get a non-retryable `403`, as do a blocked
[named user](#named-users)'s, recorded as `user_blocked`; only configured users
can be blocked, and their running calls continue. Cancelling a call before its
response starts answers it with a Ren error (`409`, `x-should-retry: false`);
once output is flowing, an HTTP stream is interrupted, and a Codex WebSocket call
is sent `response.interrupt`, with the answer kept from the client. Paused and
blocked Codex WebSocket calls get an `error` event of type `ren_*`. See
[Recording](recording.md#backup) for `backup`.

Pauses and session and user blocks survive a restart: the daemon restores the state the
last recorded command made, logs a warning saying so, and keeps counting
revisions. Run `ren control resume`, `unblock`, or `unblock-user` to clear them. A command is
durable once the recorder commits it, shortly after the API answers; a crash in
between loses the command and its record together. If the stored state cannot be
read, the daemon refuses to start rather than starting unpaused.

The control API (`/control/v1`) accepts a request carrying the per-run token from
`control.token` in the data directory (the CLI reads it), or one from Tailscale
Serve whose `Tailscale-User-Login` is listed in `control.tailscale_logins`.
Loopback alone is not enough, because Serve forwards tagged devices and Funnel
traffic to loopback without an identity. Requests with `Origin` or
`Sec-Fetch-Site` (browsers) are refused. The UI drives sessions and edits
history only through its own server, which holds the token
([UI](../ui/README.md#sessions)). The
endpoints are listed in the [API reference](api.md#control-api).

`ren control` targets the daemon the history commands target. On the daemon's
host it sends the token. Through a gateway (`--gateway`, `REN_GATEWAY`, or
`client.toml`) it sends none, so it works only through Tailscale Serve and only
for a login in the daemon's `tailscale_logins`:

```toml
[control]
tailscale_logins = ["you@example.com"]
```

`tailscale status --json` shows your login (`LoginName` under `User`). Restart
the daemon after changing it. A refusal for a missing identity or an unlisted
login says which one it was.
