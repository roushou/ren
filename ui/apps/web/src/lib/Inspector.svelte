<script lang="ts">
  import { Clock, Elapsed } from './time.ts';
  import ChipTag from './ChipTag.svelte';
  import Meter from './Meter.svelte';
  import type { GatewayData } from '#lib/loaders.ts';
  import { Arrival } from '#lib/arrival.ts';
  import { Allowances } from './allowance.ts';
  import BodyInspector from './BodyInspector.svelte';
  import Glyph from './Glyph.svelte';
  import { CallTimeline } from './timeline.ts';
  import Icon from './Icon.svelte';
  import CopyButton from './CopyButton.svelte';
  import {
    ManagementApi,
    type BodyView,
    type CallDetail,
    type CallView,
    type RecordingId,
  } from './api.ts';
  import { num, short, title, bytes, providerName, providerMark } from './api.ts';
  import {
    attemptEnding,
    captureChip,
    channelState,
    httpChip,
    messageCounts,
    modelChip,
    duration,
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
    onrecording,
    onupdate,
  }: {
    call: CallView;
    onrecording: (id: RecordingId) => void;
    onupdate: (record: CallView) => void;
  } = $props();
  /** The daemon's per-body capture limit, to tell a trimmed body from lost bytes. */
  const bodyLimit = $derived(
    Arrival.value<GatewayData>(page.data.gateway)?.storage.capture?.body_limit_bytes ?? null,
  );
  const api = new ManagementApi();
  let detail = $state<CallDetail | null>(null);
  let detailError = $state('');
  let section = $state<'overview' | 'request' | 'response' | 'metadata'>('overview');
  // An open snapshot never replaces a finished one, whichever arrived last.
  const current = $derived(
    detail?.call.id === call.id && (detail.call.state !== 'open' || call.state === 'open')
      ? detail.call
      : call,
  );
  const attempts = $derived(detail?.call.id === call.id ? detail.attempts : []);
  const inspection = $derived(attempts.at(-1)?.inspection ?? null);
  /** The limit headers the final attempt's response head carried. */
  const limitHeaders = $derived(attempts.at(-1)?.limit_headers ?? []);
  /** What the limit headers say is left, each limit as a bar. */
  const allowances = $derived(Allowances.all(limitHeaders));
  const workbench = getWorkbench();
  const usage = $derived(current.usage ?? inspection?.usage ?? null);
  const split = $derived(CallTimeline.tokens({ ...current, usage }));
  const said = $derived(CallTimeline.mark(current));
  /** Attempts across the call's span; an open call's runs to the last read. */
  const attemptBars = $derived(
    CallTimeline.attempts(current, attempts, (void workbench.refreshed, Date.now())),
  );
  const tokens = $derived(tokenSummary(usage));
  // Reload detail after every page refresh (live or manual), which is when the
  // listed row can change. Depend on the id text and the refresh count only:
  // passing detail back up replaces the `call` prop and must not reload again.
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
      <h2 id="inspector-title" class="call-title">
        <Glyph mark={said.mark} label={said.label} />
        <span class="mono"
          >{current.model ?? (isActive(current) ? 'Request in progress' : 'Captured request')}</span
        >
      </h2>
      <div class="drawer-identity">
        <span class="provider-cell"
          ><span class="provider-mark tiny {current.provider}"
            >{providerMark(current.provider)}</span
          >{providerName(current.provider)}</span
        ><span class="divider"></span><span class="mono">{current.method} {current.endpoint}</span
        ><span class="divider"></span><span class="mono">{Clock.second(current.started_at)}</span
        ><span class="divider"></span><code>{short(current.id)}</code><CopyButton
          text={current.id}
          label="Copy call ID"
        />
      </div>
    </div>
  </div>
  {#if current.recording}{@const recording = current.recording}<button
      class="inspector-recording text-button"
      onclick={() => onrecording(recording)}
      >Session <span class="mono">{short(current.attribution.hint?.session ?? recording)}</span
      ><Icon name="arrow" size={12} /></button
    >{:else}<p class="inspector-recording muted">No session</p>{/if}
  <div class="chip-row drawer-chips" aria-label="Call figures">
    {#if current.status !== null}{@const http = httpChip(current.status)}<span
        class="chip mono {http.tone}"
        title={http.title}>HTTP {http.label}</span
      >{/if}{#if attempts.length > 1}<span class="chip mono" title="Upstream tries"
        >{attempts.length} attempts</span
      >{/if}{#if current.request}<span class="chip mono" title="Request body stored"
        >{bytes(current.request.stored_bytes)} sent</span
      >{/if}{#if current.response}<span class="chip mono" title="Response body stored"
        >{bytes(current.response.stored_bytes)} back</span
      >{/if}<span class="chip mono" title="From the request to the response's end"
      >{Elapsed.latency(duration(current))}</span
    >{#each [streamChip(current), captureChip(current, bodyLimit)].filter((chip) => chip.tone === 'warning' || chip.tone === 'error') as chip}<ChipTag
        {chip}
      />{/each}
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
              <ChipTag chip={modelChip(current)} />
            </dd>
          </div>
          <div>
            <dt>Response stream</dt>
            <dd>
              <ChipTag chip={streamChip(current)} />
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
        {#if inspection}
          {@const events = inspection.events}
          <dl class="event-figures" aria-label="Events">
            <div>
              <dt class="cap">Understood</dt>
              <dd class="mono">{num(events.understood)}</dd>
            </div>
            <div
              class:warning={events.unknown > 0}
              title={inspection.unknown_kinds.length
                ? inspection.unknown_kinds.map((k) => `${k.kind} ×${k.count}`).join('\n')
                : undefined}
            >
              <dt class="cap">Unknown</dt>
              <dd class="mono">
                {#if events.unknown}<Glyph mark="warning" />{/if}{num(events.unknown)}
              </dd>
            </div>
            <div class:error={events.malformed > 0}>
              <dt class="cap">Malformed</dt>
              <dd class="mono">
                {#if events.malformed}<Glyph mark="failed" />{/if}{num(events.malformed)}
              </dd>
            </div>
            <div>
              <dt class="cap">Terminal</dt>
              <dd class="mono ellipsis">{inspection.terminal_event ?? '—'}</dd>
            </div>
          </dl>
        {/if}
      </section>
      <section class="detail-section">
        <div class="section-heading">
          <h3>Attempts <span class="count-pill">{num(attempts.length)}</span></h3>
        </div>
        {#if attempts.length}<div class="attempts">
            {#each attemptBars as bar (bar.attempt.id)}
              <span class="mono muted">#{bar.attempt.ordinal}</span>
              <span class="attempt-said" title={attemptEnding(bar.attempt) ?? bar.label}
                ><Glyph mark={bar.mark} label={bar.label} /><span class="mono">{bar.label}</span
                ></span
              >
              <svg class="attempt-track" viewBox="0 0 100 6" preserveAspectRatio="none"
                ><rect class="track-whole" width="100" height="6" /><rect
                  class="track-part state-{bar.mark}"
                  x={bar.left}
                  width={bar.width}
                  height="6"
                /></svg
              >
              <span class="mono numeric">{Elapsed.latency(duration(bar.attempt))}</span>
            {/each}
          </div>
          <p class="fine-print">
            {attempts
              .map((attempt) => providerName(attempt.upstream))
              .filter((name, at, all) => all.indexOf(name) === at)
              .join(', ')}{attempts.some((attempt) => attempt.response_format)
              ? ` · ${attempts.flatMap((attempt) => (attempt.response_format ? [title(attempt.response_format)] : [])).at(-1)}`
              : ''}
          </p>{:else}<p class="quiet-empty">
            {detail ? 'No upstream attempt was recorded.' : 'Loading attempts…'}
          </p>{/if}
      </section>
      {#if limitHeaders.length}<section class="detail-section">
          <div class="section-heading">
            <h3>Limit headers <span class="count-pill">{num(limitHeaders.length)}</span></h3>
          </div>
          {#if allowances.length}
            <div class="allowances">
              {#each allowances as allowance (allowance.name)}
                <span class="allowance-name">{allowance.name}</span>
                <Meter
                  share={allowance.left}
                  size="bar"
                  tone={allowance.left < Allowances.LOW ? 'warning' : 'success'}
                />
                <span class="mono allowance-text">{allowance.text}</span>
              {/each}
            </div>
          {/if}
          <dl class="detail-list">
            {#each limitHeaders as header, index (index)}<div>
                <dt class="mono">{header.name}</dt>
                <dd class="mono" class:muted={header.value === null}>
                  {header.value ?? 'withheld'}
                </dd>
              </div>{/each}
          </dl>
        </section>{/if}
      <section class="detail-section">
        <div class="section-heading">
          <h3>Token usage</h3>
          <span class="subtle-label">Provider reported</span>
        </div>
        {#if split}
          <svg class="token-bar" viewBox="0 0 100 8" preserveAspectRatio="none" aria-hidden="true"
            ><rect
              class="tok-read"
              width={(split.read / Math.max(1, split.context)) * 100}
              height="8"
            /><rect
              class="tok-written"
              x={(split.read / Math.max(1, split.context)) * 100}
              width={(split.written / Math.max(1, split.context)) * 100}
              height="8"
            /><rect
              class="tok-fresh"
              x={((split.read + split.written) / Math.max(1, split.context)) * 100}
              width={(split.fresh / Math.max(1, split.context)) * 100}
              height="8"
            /></svg
          >
          <dl class="token-legend">
            <div>
              <dt><span class="swatch tok-read"></span>Read from cache</dt>
              <dd class="mono">{num(split.read)}</dd>
            </div>
            <div>
              <dt><span class="swatch tok-written"></span>Written to cache</dt>
              <dd class="mono">{num(split.written)}</dd>
            </div>
            <div>
              <dt><span class="swatch tok-fresh"></span>New input</dt>
              <dd class="mono">{num(split.fresh)}</dd>
            </div>
            <div>
              <dt><span class="swatch"></span>Output</dt>
              <dd class="mono">{num(split.output)}</dd>
            </div>
          </dl>
        {:else}<p class="quiet-empty">No usage reported or captured.</p>{/if}
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
        {:else if !inspection && current.tool_names.length}{#each current.tool_names as name}<div
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
                {#each volume(body) as chip}<ChipTag {chip} />{/each}
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
          <dt>Session</dt>
          <dd>{current.recording ?? 'Unassigned'}</dd>
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
                  #{observation.seq} · {Clock.second(observation.at)}
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
      >Duration <strong class="mono">{Elapsed.latency(duration(current))}</strong></span
    ><small>Provider reported · input includes cached context</small>
  </div>
</aside>

<style>
  .event-figures {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 1px;
    margin: 8px 0 0;
    border: 1px solid var(--line);
    background: var(--line);
  }
  .event-figures > div {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    padding: 6px 10px;
    background: var(--surface);
  }
  .event-figures > div.warning {
    background: color-mix(in srgb, var(--warning) 8%, var(--surface));
  }
  .event-figures > div.error {
    background: color-mix(in srgb, var(--error) 10%, var(--surface));
  }
  .event-figures dd {
    display: flex;
    gap: 6px;
    align-items: center;
    margin: 0;
  }
  .event-figures .ellipsis {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .allowances {
    display: grid;
    grid-template-columns: 110px minmax(0, 1fr) auto;
    column-gap: 10px;
    row-gap: 6px;
    align-items: center;
    margin-bottom: 10px;
    font-size: 12px;
  }
  .allowance-name::first-letter {
    text-transform: uppercase;
  }
  .allowance-text {
    color: var(--muted);
    font-size: 11px;
  }
  .call-title {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .attempts {
    display: grid;
    grid-template-columns: 24px minmax(0, 1fr) minmax(80px, 1.2fr) 64px;
    align-items: center;
    gap: 6px 10px;
    font-size: 11.5px;
  }
  .attempt-said {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
  .attempt-said .mono {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .attempt-track {
    width: 100%;
    height: 6px;
  }
  .track-whole {
    fill: var(--surface-inset);
  }
  .track-part {
    fill: var(--muted);
  }
  .track-part.state-failed {
    fill: var(--error);
  }
  .track-part.state-warning {
    fill: var(--warning);
  }
  .track-part.state-responding {
    fill: var(--live);
  }
  .numeric {
    text-align: right;
  }
  .token-bar {
    width: 100%;
    height: 8px;
    margin: 10px 0;
    background: var(--surface-inset);
  }
  .tok-read {
    fill: var(--success);
    background: var(--success);
  }
  .tok-written {
    fill: var(--warning);
    background: var(--warning);
  }
  .tok-fresh {
    fill: var(--accent);
    background: var(--accent);
  }
  .token-legend {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px 16px;
    margin: 0 0 10px;
    font-size: 11.5px;
  }
  .token-legend > div {
    display: flex;
    justify-content: space-between;
    gap: 8px;
  }
  .token-legend dt {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
  }
  .token-legend dd {
    margin: 0;
  }
  .swatch {
    width: 8px;
    height: 8px;
    background: var(--line);
  }
</style>
