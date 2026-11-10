<script lang="ts">
  import BodyInspector from './BodyInspector.svelte';
  import Icon from './Icon.svelte';
  import CopyButton from './CopyButton.svelte';
  import {
    ManagementApi,
    type BodyView,
    type CallDetail,
    type CallView,
    type SessionId,
  } from './api.ts';
  import { num, short, title, bytes, clock, providerName, providerMark } from './api.ts';
  import {
    attemptEnding,
    captureChip,
    channelState,
    httpChip,
    messageCounts,
    modelChip,
    duration,
    elapsed,
    isActive,
    streamChip,
    tokenSummary,
    type Chip,
    callerShown,
  } from './display.ts';
  import { snapshotError } from './snapshots.ts';
  import { getWorkbench } from './workbench.svelte.ts';
  import { page } from '$app/state';
  let {
    call,
    close,
    onsession,
    onupdate,
  }: {
    call: CallView;
    close: () => void;
    onsession: (id: SessionId) => void;
    onupdate: (record: CallView) => void;
  } = $props();
  /** The daemon's per-body capture limit, to tell a trimmed body from lost bytes. */
  const bodyLimit = $derived(page.data.gateway?.value?.storage.capture?.body_limit_bytes ?? null);
  const api = new ManagementApi();
  let detail = $state<CallDetail | null>(null);
  let detailError = $state('');
  let section = $state<'overview' | 'request' | 'response' | 'metadata'>('response');
  // An open snapshot never replaces a finished one, whichever arrived last.
  const current = $derived(
    detail?.call.id === call.id && (detail.call.state !== 'open' || call.state === 'open')
      ? detail.call
      : call,
  );
  const attempts = $derived(detail?.call.id === call.id ? detail.attempts : []);
  const inspection = $derived(attempts.at(-1)?.inspection ?? null);
  const usage = $derived(current.usage ?? inspection?.usage ?? null);
  const tokens = $derived(tokenSummary(usage));
  // Reload detail after every page refresh (live or manual), which is when the
  // listed row can change. Depend on the id text and the refresh count only:
  // passing detail back up replaces the `call` prop and must not reload again.
  const workbench = getWorkbench();
  const id = $derived(call.id);
  function accept(record: CallDetail) {
    if (record.call.id !== id) return;
    detail = record;
    detailError = '';
    onupdate(record.call);
  }
  $effect(() => {
    const target = id;
    void workbench.refreshed;
    const controller = new AbortController();
    api
      .call(target, controller.signal)
      .then(accept)
      .catch((error: unknown) => {
        if (!controller.signal.aborted)
          detailError = snapshotError(error, 'Call detail could not be loaded.').message;
      });
    return () => controller.abort();
  });
  /** A body's capture as chips: its size, then anything short of whole. */
  function volume(body: BodyView | null): Chip[] {
    if (!body)
      return [
        isActive(current)
          ? { label: 'Recording', tone: 'live', title: 'Nothing stored yet' }
          : { label: 'Not recorded', tone: 'neutral', title: 'No body was recorded' },
      ];
    const chips: Chip[] = [
      {
        label: bytes(body.stored_bytes),
        tone: 'neutral',
        title: `${bytes(body.stored_bytes)} stored`,
      },
    ];
    if (body.observed_bytes !== null && body.observed_bytes !== body.stored_bytes)
      chips.push({
        label: `of ${bytes(body.observed_bytes)} seen`,
        tone: 'warning',
        title: 'Not every observed byte was stored',
      });
    if (body.ending === 'open')
      chips.push({ label: 'Recording', tone: 'live', title: 'The body is still arriving' });
    else if (body.ending === 'ended')
      chips.push({ label: 'Ended', tone: 'success', title: 'The body reached its end' });
    else
      chips.push({
        label: body.ending === 'unfinished' ? 'Never ended' : title(body.ending),
        tone: 'warning',
        title: 'The body did not reach its end',
      });
    return chips;
  }
</script>

<aside class="inspector-pane" aria-labelledby="inspector-title">
  <div class="drawer-heading">
    <div>
      <span class="drawer-eyebrow"><Icon name="logs" size={14} /> CALL</span>
      <h2 id="inspector-title">
        {current.model ?? (isActive(current) ? 'Request in progress' : 'Captured request')}
      </h2>
      <div class="drawer-identity">
        <span class="provider-cell"
          ><span class="provider-mark tiny {current.provider}"
            >{providerMark(current.provider)}</span
          >{providerName(current.provider)}</span
        ><span class="divider"></span><code>{short(current.id)}</code><CopyButton
          text={current.id}
          label="Copy call ID"
        />
      </div>
    </div>
    <button
      class="icon-button close-button"
      onclick={close}
      aria-label="Close call details"
      title="Close · Esc"><Icon name="close" size={15} /></button
    >
  </div>
  {#if current.session}{@const session = current.session}<button
      class="inspector-session text-button"
      onclick={() => onsession(session)}
      >Session <span class="mono">{short(current.attribution.hint?.session ?? session)}</span><Icon
        name="arrow"
        size={12}
      /></button
    >{:else}<p class="inspector-session muted">No reported session</p>{/if}
  <div class="chip-row drawer-chips" aria-label="Call state">
    {#each [modelChip(current), streamChip(current), captureChip(current, bodyLimit)] as chip}<span
        class="chip {chip.tone}"
        title={chip.title}>{chip.label}</span
      >{/each}{#if current.status !== null}{@const http = httpChip(current.status)}<span
        class="chip mono {http.tone}"
        title={http.title}>HTTP {http.label}</span
      >{/if}
  </div>
  {#if current.outcome === 'completed' && current.terminal === 'interrupted'}<p
      class="inspector-notice"
    >
      Model completion was observed; the HTTP stream was interrupted.
    </p>{/if}
  <nav class="detail-tabs" aria-label="Request detail views">
    {#each ['request', 'response', 'overview', 'metadata'] as tab}<button
        class:active={section === tab}
        aria-pressed={section === tab}
        onclick={() => (section = tab as typeof section)}
        >{tab === 'overview'
          ? 'Evidence'
          : tab === 'request'
            ? 'Request'
            : tab === 'response'
              ? 'Response'
              : 'Metadata'}</button
      >{/each}
  </nav>
  <div class="drawer-content">
    {#if detailError}<p class="callout" role="alert">
        {detailError}{detail ? ' Showing the last loaded detail.' : ''}
      </p>{/if}
    {#if section === 'overview'}
      <section class="detail-section">
        <div class="section-heading">
          <h3>Request summary</h3>
          <span class="muted">{new Date(current.started_at).toLocaleString()}</span>
        </div>
        <dl class="detail-list">
          <div>
            <dt>Endpoint</dt>
            <dd class="mono">
              <span class="method">{current.method}</span>{current.endpoint}{detail
                ? ` · ${detail.transport}`
                : ''}
            </dd>
          </div>
          {#if detail && channelState(detail.channel)}<div>
              <dt>WebSocket channel</dt>
              <dd class="mono">
                {channelState(detail.channel)} · {messageCounts(detail.messages).request} client / {messageCounts(
                  detail.messages,
                ).response} upstream messages
              </dd>
            </div>{/if}
          <div>
            <dt>Model outcome</dt>
            <dd>
              {#each [modelChip(current)] as chip}<span class="chip {chip.tone}" title={chip.title}
                  >{chip.label}</span
                >{/each}
            </dd>
          </div>
          <div>
            <dt>Response stream</dt>
            <dd>
              {#each [streamChip(current)] as chip}<span class="chip {chip.tone}" title={chip.title}
                  >{chip.label}</span
                >{/each}
            </dd>
          </div>
          <div>
            <dt>Inspection coverage</dt>
            <dd>
              <span
                class="chip"
                class:success={inspection?.coverage === 'complete'}
                class:warning={!!inspection && inspection.coverage !== 'complete'}
                title="How much of the response the inspector understood"
                >{inspection ? title(inspection.coverage) : 'Not inspected'}</span
              >
            </dd>
          </div>
          <div>
            <dt>Stop reason</dt>
            <dd>{title(current.stop_reason)}</dd>
          </div>
        </dl>
        {#if current.outcome === 'completed' && current.terminal === 'interrupted'}<p
            class="callout"
          >
            The model completed before the HTTP stream ended. The client may have stopped reading
            after completion.
          </p>{/if}
        {#if current.state === 'recovered'}<p class="callout">
            Ren restarted before this call finished. Its outcome is unknown.
          </p>{/if}
        {#if inspection?.limitations.length}<div class="limitation-list">
            {#each inspection.limitations as reason}<span>{title(reason)}</span>{/each}
          </div>{/if}
        {#if inspection}<p class="fine-print">
            Events: {num(inspection.events.understood)} understood · {num(
              inspection.events.unknown,
            )}
            unknown · {num(inspection.events.malformed)} malformed · terminal event {inspection.terminal_event}{inspection
              .unknown_kinds.length
              ? ` · unknown kinds: ${inspection.unknown_kinds.map((k) => `${k.kind} (${k.count})`).join(', ')}`
              : ''}
          </p>{/if}
      </section>
      <section class="detail-section">
        <div class="section-heading">
          <h3>Attempts <span class="count-pill">{num(attempts.length)}</span></h3>
          <span class="subtle-label">Upstream tries</span>
        </div>
        {#if attempts.length}<dl class="detail-list">
            {#each attempts as attempt (attempt.id)}{@const stream = streamChip(attempt)}
              <div>
                <dt>
                  #{attempt.ordinal} <span class="mono">{short(attempt.id)}</span> · {providerName(
                    attempt.upstream,
                  )}
                </dt>
                <dd class="chip-row attempt-chips">
                  {#if attempt.status !== null}{@const http = httpChip(attempt.status)}<span
                      class="chip mono {http.tone}"
                      title={http.title}>{http.label}</span
                    >{/if}<span
                    class="chip {stream.tone}"
                    title={attemptEnding(attempt) ?? stream.title}>{stream.label}</span
                  ><span class="chip mono">{elapsed(duration(attempt))}</span
                  >{#if attempt.response_format}<span class="chip"
                      >{title(attempt.response_format)}</span
                    >{/if}{#if attempt.response_encoding && attempt.response_encoding !== 'identity'}<span
                      class="chip">{attempt.response_encoding}</span
                    >{/if}
                </dd>
              </div>{/each}
          </dl>{:else}<p class="quiet-empty">
            {detail ? 'No upstream attempt was recorded.' : 'Loading attempts…'}
          </p>{/if}
      </section>
      <section class="detail-section">
        <div class="section-heading">
          <h3>Token usage</h3>
          <span class="subtle-label">Provider reported</span>
        </div>
        <div class="usage-grid">
          <div><span>Input</span><strong>{num(usage?.input_tokens)}</strong></div>
          <div><span>Output</span><strong>{num(usage?.output_tokens)}</strong></div>
          <div><span>Cache read</span><strong>{num(usage?.cache_read_tokens)}</strong></div>
          <div><span>Cache write</span><strong>{num(usage?.cache_creation_tokens)}</strong></div>
        </div>
        <dl class="detail-list">
          <div>
            <dt>Reasoning tokens</dt>
            <dd class="mono">{num(usage?.reasoning_tokens)}</dd>
          </div>
        </dl>
        <p class="fine-print">
          {usage
            ? usage.accounting === 'includes_cache'
              ? 'Input includes cached tokens.'
              : usage.accounting === 'excludes_cache'
                ? 'Input excludes cached tokens.'
                : `Cached-token accounting: ${usage.accounting}.`
            : 'No usage reported or captured.'} Token counts do not represent subscription allowance or
          charges.
        </p>
      </section>
      <section class="detail-section">
        <div class="section-heading">
          <h3>
            Tool calls <span class="count-pill"
              >{num(inspection ? inspection.tool_calls.length : current.tool_names.length)}</span
            >
          </h3>
          <Icon name="tool" size={14} />
        </div>
        {#if inspection?.tool_calls.length}{#each inspection.tool_calls as tool}<div
              class="tool-item"
            >
              <span class="tool-icon"><Icon name="tool" size={14} /></span>
              <div>
                <strong>{tool.name ?? title(tool.kind)}</strong><code
                  >{tool.call_id ?? tool.item_id ?? '—'}</code
                >
              </div>
            </div>{/each}
          <p class="fine-print">
            Requested by the model. Execution is tracked by your client.
          </p>{:else if !inspection && current.tool_names.length}{#each current.tool_names as name}<div
              class="tool-item"
            >
              <span class="tool-icon"><Icon name="tool" size={14} /></span>
              <div><strong>{name}</strong></div>
            </div>{/each}{:else}<p class="quiet-empty">
            No tool requests observed in the captured response.
          </p>{/if}
      </section>
      <section class="detail-section">
        <div class="section-heading">
          <h3>Capture volume</h3>
          <span class="subtle-label">Stored / observed</span>
        </div>
        <dl class="detail-list">
          {#each [['Request', current.request] as const, ['Response', current.response] as const] as [name, body]}<div
            >
              <dt>{name}</dt>
              <dd class="chip-row volume-chips">
                {#each volume(body) as chip}<span class="chip {chip.tone}" title={chip.title}
                    >{chip.label}</span
                  >{/each}
              </dd>
            </div>{/each}
        </dl>
      </section>
    {:else if section === 'request' || section === 'response'}
      {#key `${current.id}-${section}`}<BodyInspector
          id={current.id}
          initialDirection={section}
          bodies={{ request: current.request, response: current.response }}
          onupdate={accept}
        />{/key}
    {:else}
      <section class="detail-section">
        <div class="section-heading"><h3>Identifiers & provenance</h3></div>
        <dl class="id-list">
          <dt>Call ID</dt>
          <dd>{current.id}</dd>
          <dt>Channel</dt>
          <dd>{current.channel}</dd>
          <dt>Caller</dt>
          <dd>{callerShown(current.caller).title}</dd>
          <dt>Ren session</dt>
          <dd>{current.session ?? 'Unassigned'}</dd>
          <dt>Ren turn</dt>
          <dd>{current.turn ?? 'Unassigned'}</dd>
          <dt>Attribution source</dt>
          <dd>{title(current.attribution.hint?.source)}</dd>
          <dt>Native session</dt>
          <dd>{current.attribution.hint?.session ?? 'Unreported'}</dd>
          <dt>Native turn</dt>
          <dd>{current.attribution.hint?.turn ?? 'Unreported'}</dd>
          <dt>Native thread</dt>
          <dd>{current.attribution.hint?.thread ?? 'Unreported'}</dd>
          <dt>Config revision</dt>
          <dd>{current.config_revision || 'Unknown'}</dd>
        </dl>
        {#each current.attribution.issues as issue}<p class="callout">
            {title(issue)}
          </p>{/each}
      </section>
      <section class="detail-section">
        <div class="section-heading">
          <h3>
            Observation log <span class="count-pill">{num(detail?.observations.length)}</span>
          </h3>
          <span class="subtle-label">Committed order</span>
        </div>
        {#if detail?.observations.length}<dl class="detail-list">
            {#each detail.observations as observation (observation.seq)}<div>
                <dt>
                  {title(observation.kind)}{observation.attempt
                    ? ` · ${short(observation.attempt)}`
                    : ''}
                </dt>
                <dd class="mono" title={new Date(observation.at).toLocaleString()}>
                  #{observation.seq} · {clock(observation.at)}
                </dd>
              </div>{/each}
          </dl>{:else}<p class="quiet-empty">
            {detail ? 'No observations retained.' : 'Loading observations…'}
          </p>{/if}
      </section>
      <div class="code-heading">
        <h3>Recorded metadata</h3>
        <CopyButton
          text={JSON.stringify(detail ?? current, null, 2)}
          label="Copy recorded metadata"
        />
      </div>
      <pre class="metadata-code">{JSON.stringify(detail ?? current, null, 2)}</pre>
    {/if}
  </div>
  <div class="inspector-footer">
    {#if tokens}<span title={tokens.title}
        >Tokens <strong class="mono">{tokens.input}</strong> in
        <strong class="mono">{tokens.output}</strong> out</span
      >{:else}<span class="muted">No token usage reported</span>{/if}<span
      >Duration <strong class="mono">{elapsed(duration(current))}</strong></span
    ><small>Provider reported · input includes cached context</small>
  </div>
</aside>
