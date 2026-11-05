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
    channelState,
    messageCounts,
    captureState,
    duration,
    elapsed,
    isActive,
    modelState,
    transportState,
  } from './display.ts';
  import { snapshotError } from './snapshots.ts';
  import { getWorkbench } from './workbench.svelte.ts';
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
  function volume(body: BodyView | null) {
    if (!body) return isActive(current) ? 'Recording…' : 'Not recorded';
    const observed =
      body.observed_bytes === null
        ? body.ending === 'open'
          ? 'unknown while open'
          : 'unknown'
        : bytes(body.observed_bytes);
    return `${bytes(body.stored_bytes)} / ${observed} · ${title(body.ending)}`;
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
  <div class="drawer-status">
    <span><span class="muted">Model</span><strong>{modelState(current)}</strong></span>
    <span><span class="muted">Transport</span><strong>{transportState(current)}</strong></span>
    <span class:warning={!isActive(current) && !current.capture_complete}
      ><span class="muted">Capture</span><strong>{captureState(current)}</strong></span
    >
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
            <dd>{modelState(current)}</dd>
          </div>
          <div>
            <dt>HTTP outcome</dt>
            <dd>{title(current.terminal)}</dd>
          </div>
          <div>
            <dt>Inspection coverage</dt>
            <dd>{title(inspection?.coverage)}</dd>
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
            TGV restarted before this call finished. Its outcome is unknown.
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
            {#each attempts as attempt (attempt.id)}<div>
                <dt>
                  #{attempt.ordinal} <span class="mono">{short(attempt.id)}</span> · {providerName(
                    attempt.upstream,
                  )}
                </dt>
                <dd class="mono">
                  {transportState(attempt)} · {elapsed(duration(attempt))}{attempt.response_format
                    ? ` · ${title(attempt.response_format)}`
                    : ''}{attempt.response_encoding && attempt.response_encoding !== 'identity'
                    ? ` · ${attempt.response_encoding}`
                    : ''}{attemptEnding(attempt) ? ` · ${attemptEnding(attempt)}` : ''}
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
          <div>
            <dt>Request</dt>
            <dd class="mono">{volume(current.request)}</dd>
          </div>
          <div>
            <dt>Response</dt>
            <dd class="mono">{volume(current.response)}</dd>
          </div>
        </dl>
      </section>
    {:else if section === 'request' || section === 'response'}
      {#if section === 'response' && (inspection?.tool_calls.length || current.tool_names.length)}<p
          class="fine-print"
        >
          Tool requests were emitted by the model. Execution is not established by this response.
        </p>{/if}
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
          <dt>TGV session</dt>
          <dd>{current.session ?? 'Unassigned'}</dd>
          <dt>TGV turn</dt>
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
    <span
      >Tokens in / out <strong class="mono"
        >{num(usage?.input_tokens)} / {num(usage?.output_tokens)}</strong
      ></span
    ><span>Duration <strong class="mono">{elapsed(duration(current))}</strong></span><small
      >Provider reported · not subscription allowance or charges</small
    >
  </div>
</aside>
