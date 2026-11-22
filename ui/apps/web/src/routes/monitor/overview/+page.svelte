<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import { onMount } from 'svelte';
  import { page } from '$app/state';
  import { createArrival, createSnapshot } from '#lib/snapshot.svelte.ts';
  import { providerName, short } from '#lib/api.ts';
  import { compactOrDash, percentOrDash, SEPARATOR } from '#lib/display.ts';
  import { Board, LivePlace } from '#lib/board.ts';
  import { ago, Attention, Day, healthLine, Verdict } from '#lib/overview.ts';
  import ConnectClient from '#lib/ConnectClient.svelte';
  import CopyButton from '#lib/CopyButton.svelte';
  import { LaunchCommand } from '#lib/launch.ts';
  import Glyph from '#lib/Glyph.svelte';
  import { Marks } from '#lib/marks.ts';
  import Icon from '#lib/Icon.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const arrival = createArrival(() => data.overview);
  const gateway = createSnapshot(() => data.gateway);
  const shown = $derived(arrival.current);
  /** The day's reads are on their way: the page draws its outline. */
  const pending = $derived(shown.state === 'pending');
  const overview = $derived(shown.state === 'ready' ? shown.snapshot : null);
  const current = $derived(overview?.value ?? null);
  const health = $derived(gateway.current.value?.health ?? null);
  const items = $derived(
    healthLine(
      health,
      gateway.current.value?.storage ?? null,
      gateway.current.error?.message ?? null,
    ),
  );
  // Elapsed times tick between refreshes.
  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  const attention = $derived(
    current
      ? new Attention({
          now: current.now,
          health,
          failed: current.failed,
          stopped: current.stopped,
          bookkeeping: current.bookkeeping,
          exposures: current.exposures,
        })
      : null,
  );
  const day = $derived(
    current
      ? new Day(current.usage, current.previous, current.now, {
          bookkeeping: current.bookkeeping.length,
          stopped: current.stopped.length,
        })
      : null,
  );
  const live = $derived(current?.live ?? null);
  const board = $derived(
    live
      ? new Board({
          recordings: live.recordings,
          calls: live.calls,
          turns: live.turns,
          now: Math.max(now, live.read),
          windowMs: LivePlace.windows['15m'].ms,
          focus: null,
          pins: [],
          only: null,
        })
      : null,
  );
  /** The recordings at work, most recently seen first. */
  const working = $derived(
    (board?.tiles ?? [])
      .toSorted((a, b) => b.recording.last_seen - a.recording.last_seen)
      .slice(0, 6),
  );
  const lastCall = $derived(Math.max(0, ...(current?.recordings ?? []).map((s) => s.last_seen)));

  /** The verdict: how the day stands, in one line, coloured by the worst thing in it. */
  const verdict = $derived(
    new Verdict({
      health,
      items,
      attention,
      active: board ? board.tiles.length + board.overflow : 0,
      inFlight: board?.inFlight ?? 0,
      lastCall,
      now,
    }),
  );

  const recordingHref = (id: string) => `/monitor/recordings/${encodeURIComponent(id)}`;
  const spark = (values: number[]) => Day.spark(values, 22);
  const CHART = 120;
  /** Nothing recorded in the last day: the page leads with how to connect a client. */
  const firstRun = $derived(!!current && current.usage.totals.calls === 0 && !board?.tiles.length);
  const startCommand = $derived(LaunchCommand.for('claude', page.data.gatewayUrl).text);
  /** The day's tiles while they are on their way, labelled as `Day` labels them. */
  /** Rows a list panel's outline shows. */
  const OUTLINE_ROWS = 3;
</script>

<Toolbar />
<div class="overview">
  <div class="console ov-console">
    {#if !pending && (!current || !attention || !day)}
      <div class="empty-state" role="alert">
        <div class="empty-icon"><Icon name="alert" size={26} /></div>
        <h2>The overview could not be read</h2>
        <p>{overview?.error?.message ?? 'Loading…'}</p>
      </div>
    {:else}
      <section
        class="ov-verdict state-edge {pending ? '' : verdict.tone}"
        aria-label="How it stands"
        aria-busy={pending}
      >
        <p>
          {#if pending}<Skeleton width="22em" />{:else}<strong>{verdict.headline}</strong>
            {#each verdict.facts as fact (fact)}<span class="muted">{SEPARATOR}{fact}</span
              >{/each}{#each verdict.problems as problem (problem.key)}<span class="muted"
                >{SEPARATOR}</span
              ><a class="ov-problem {problem.severity}" href={problem.href}>{problem.title}</a
              >{/each}{/if}
        </p>
        <div class="ov-health mono" aria-label="Health">
          {#each items as item (item.label)}{#if item.href}<a
                class={item.tone}
                href={item.href}
                title={item.title}><Glyph mark={Marks.ofTone(item.tone)} />{item.label}</a
              >{:else}<span class={item.tone} title={item.title}
                ><Glyph mark={Marks.ofTone(item.tone)} />{item.label}</span
              >{/if}{/each}
        </div>
      </section>

      {#if firstRun}
        <section class="ov-panel ov-first" aria-labelledby="first-title">
          <h2 id="first-title">Connect a client to start recording</h2>
          <p class="muted">
            Nothing passed through Ren in the last 24 hours. Start a session through this gateway;
            its calls show up here, on Live, and in Traffic as they happen.
          </p>
          <ConnectClient layout="box" gatewayOrigin={page.data.gatewayUrl} />
          <span class="empty-footnote"
            ><Icon name="shield" size={12} /> Uses your client’s existing sign-in</span
          >
        </section>
      {:else}
        <div class="ov-top">
          <section class="ov-panel" aria-labelledby="attention-title" aria-busy={pending}>
            <header class="ov-heading">
              <h2 id="attention-title">Needs attention</h2>
              <span class="muted mono">last 24 h</span>
            </header>
            {#if pending}
              {#each { length: OUTLINE_ROWS }, row (row)}
                <div class="ov-attention outline">
                  <i></i>
                  <span class="ov-attention-text">
                    <strong><Skeleton width={['11em', '8em', '13em'][row % 3]} /></strong>
                    <small><Skeleton width={['18em', '22em', '15em'][row % 3]} /></small>
                  </span>
                  <span></span>
                  <span class="ov-when"><Skeleton width="4em" /></span>
                </div>
              {/each}
            {:else}{#each attention?.groups ?? [] as group (group.key)}
                <a class="ov-attention {group.severity}" href={group.href}>
                  <Glyph
                    mark={group.severity === 'error'
                      ? 'failed'
                      : group.severity === 'warning'
                        ? 'warning'
                        : 'info'}
                    label={true}
                  />
                  <span class="ov-attention-text">
                    <strong>{group.title}</strong>
                    <small>{group.detail}</small>
                  </span>
                  {#if group.marks.length > 1}
                    <svg
                      class="ov-marks"
                      viewBox="0 0 100 10"
                      preserveAspectRatio="none"
                      aria-label="When they happened over the day"
                    >
                      <line x1="0" y1="9.5" x2="100" y2="9.5" />
                      {#each group.marks as mark, index (index)}<rect
                          x={mark * 100 - 0.6}
                          y="2"
                          width="1.2"
                          height="7"
                        />{/each}
                    </svg>
                  {/if}
                  <span class="mono muted ov-when"
                    >{group.at === null ? 'since start' : ago(group.at, now)}</span
                  >
                </a>
              {:else}
                <p class="ov-clear">
                  <Glyph mark="passed" /> Nothing needs a look: no failed or cut-off work, no exposed
                  secrets, no capture loss in the last 24 hours.
                </p>
              {/each}{/if}
          </section>

          <section class="ov-panel" aria-labelledby="live-title" aria-busy={pending}>
            <header class="ov-heading">
              <h2 id="live-title">Live now</h2>
              <a class="text-button" href="/monitor/live">Live <Icon name="arrow" size={12} /></a>
            </header>
            {#if pending}
              {#each { length: OUTLINE_ROWS }, row (row)}
                <div class="ov-recording outline">
                  <span class="ov-recording-who">
                    <strong><Skeleton width="6em" /></strong>
                    <span class="live-state"><Skeleton width="5em" /></span>
                  </span>
                  <span class="ov-recording-prompt"
                    ><Skeleton width={['85%', '64%', '76%'][row % 3]} /></span
                  >
                  <span class="ov-lane"><Skeleton width="100%" /></span>
                </div>
              {/each}
            {:else}{#each working as tile (tile.recording.id)}
                {@const chip = Board.phaseChip(tile.phase, board?.now ?? now)}
                <a
                  class="ov-recording state-edge state-{chip.mark}"
                  class:quiet={tile.quiet}
                  href={recordingHref(tile.recording.id)}
                >
                  <span class="ov-recording-who">
                    <strong>{Board.clientName(tile.harness)}</strong>
                    <span class="mono muted">{short(tile.recording.native_id)}</span>
                    <span class="mono live-state {chip.tone} state-{chip.mark}" title={chip.title}
                      ><Glyph mark={chip.mark} label={chip.label} /></span
                    >
                  </span>
                  <span class="ov-recording-prompt"
                    >{tile.prompt?.text ??
                      `${providerName(tile.recording.provider)} · ${tile.model ?? 'no model yet'}`}</span
                  >
                  <svg
                    class="ov-lane"
                    viewBox="0 0 100 8"
                    preserveAspectRatio="none"
                    aria-label="Calls in the last 2 minutes"
                  >
                    <line x1="0" y1="4" x2="100" y2="4" />
                    {#each tile.lane as mark (mark.call.id)}<rect
                        class={mark.open ? 'open' : mark.tone}
                        x={mark.start * 100}
                        y="1"
                        width={Math.max(0.8, (mark.end - mark.start) * 100)}
                        height="6"
                      />{/each}
                  </svg>
                </a>
              {:else}
                <div class="ov-clear ov-start">
                  <span class="muted"
                    >No recording active in the last 15 minutes{lastCall
                      ? `; the last call was ${ago(lastCall, now)}`
                      : ''}.</span
                  >
                  <span class="ov-start-command"
                    ><span class="muted">Start one:</span> <code>{startCommand}</code><CopyButton
                      text={startCommand}
                      label="Copy the launch command"
                    /></span
                  >
                </div>
              {/each}{/if}
            {#if board && board.tiles.length > working.length}<a
                class="ov-more"
                href="/monitor/live"
                >{board.tiles.length - working.length + board.overflow} more on Live</a
              >{/if}
          </section>
        </div>

        <section class="ov-tiles" aria-label="Last 24 hours" aria-busy={pending}>
          {#if pending}
            {#each Day.kinds as kind (kind)}
              <div class="ov-tile">
                <span class="muted">{Day.label(kind)}</span>
                <strong class="mono"><Skeleton size="figure" width="4em" /></strong>
                <small><Skeleton width="10em" /></small>
              </div>
            {/each}
          {:else if day}{#each day.tiles as tile (tile.label)}
              <div class="ov-tile">
                <span class="muted">{tile.label}</span>
                <strong class="mono" class:error={tile.tone === 'error'}>{tile.value}</strong>
                <small class="muted">{tile.detail}</small>
                {#if tile.kind === 'input' && day.input && day.input.cached + day.input.fresh > 0}
                  <span
                    class="ov-cache"
                    title="{percentOrDash(
                      Math.round((day.input.cached / (day.input.cached + day.input.fresh)) * 100),
                    )} read from cache"
                    ><svg viewBox="0 0 100 6" preserveAspectRatio="none" aria-hidden="true"
                      ><rect
                        class="cached"
                        x="0"
                        y="0"
                        height="6"
                        width={(day.input.cached / (day.input.cached + day.input.fresh)) * 100}
                      /><rect
                        class="fresh"
                        y="0"
                        height="6"
                        x={(day.input.cached / (day.input.cached + day.input.fresh)) * 100}
                        width={(day.input.fresh / (day.input.cached + day.input.fresh)) * 100}
                      /></svg
                    ></span
                  >
                {:else if tile.spark.length}
                  <svg
                    class="ov-spark"
                    viewBox="0 0 100 22"
                    preserveAspectRatio="none"
                    aria-hidden="true"><polyline points={spark(tile.spark)} /></svg
                  >
                {/if}
              </div>
            {/each}{/if}
        </section>

        <section class="ov-panel ov-chart" aria-labelledby="day-title">
          <header class="ov-heading">
            <h2 id="day-title">Last 24 hours</h2>
            <span class="ov-legend mono">
              <span><i class="cached"></i>input from cache</span>
              <span><i class="fresh"></i>fresh input</span>
              <span><i class="failed"></i>failed work</span>
              <span><i class="stopped"></i>cut off</span>
              <span><i class="bookkeeping"></i>failed bookkeeping</span>
            </span>
            <a class="text-button" href="/monitor/usage?range=24h"
              >Usage <Icon name="arrow" size={12} /></a
            >
          </header>
          {#if pending}
            <div class="ov-day-outline" aria-busy="true">
              <div><Skeleton size="block" width="100%" /></div>
            </div>
          {:else if day && current}<svg
              class="ov-day"
              viewBox="0 0 1000 {CHART}"
              preserveAspectRatio="none"
              aria-label="Input tokens per hour over the last 24 hours"
            >
              {#each [0.25, 0.5, 0.75] as at (at)}<line
                  class="grid"
                  x1="0"
                  y1={CHART * at}
                  x2="1000"
                  y2={CHART * at}
                />{/each}
              {#each day.bars(CHART - 4) as bar (bar.start)}
                <g
                  ><title
                    >{Day.clock(bar.start)}: {compactOrDash(
                      day.points.find((point) => point.start === bar.start)?.tokens.input ?? null,
                    )} in</title
                  ><rect
                    class="cached"
                    x={bar.x}
                    width={bar.width}
                    y={CHART - bar.cached}
                    height={bar.cached}
                  /><rect
                    class="fresh"
                    x={bar.x}
                    width={bar.width}
                    y={CHART - bar.cached - bar.fresh}
                    height={bar.fresh}
                  /></g
                >
              {/each}
              <line class="base" x1="0" y1={CHART} x2="1000" y2={CHART} />
            </svg>
            <!-- Ticks keep their own fixed height while the bars above stretch. -->
            <svg
              class="ov-ticks"
              viewBox="0 0 1000 12"
              preserveAspectRatio="none"
              aria-label="Failed or cut-off calls over the same hours"
            >
              {#each [['bookkeeping', current.bookkeeping], ['stopped', current.stopped], ['failed', current.failed]] as const as [kind, calls] (kind)}
                {#each calls as call (call.id)}<rect
                    class="tick {kind}"
                    x={day.x(call.started_at) - 1}
                    y="2"
                    width="2"
                    height="9"
                  />{/each}
              {/each}
            </svg>
            <div class="ov-axis mono" aria-hidden="true">
              {#each day.axis() as tick (tick.at)}<span>{tick.label}</span>{/each}
            </div>{/if}
        </section>
      {/if}
    {/if}
  </div>
</div>

<style>
  .muted {
    color: var(--muted);
  }
  /* The verdict: one line that says how it stands, its edge the worst thing in it. */
  .ov-verdict {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 16px;
    padding: 10px 16px 10px 18px;
  }
  .ov-verdict.error {
    --state: var(--error);
  }
  .ov-verdict.warning {
    --state: var(--warning);
  }
  .ov-verdict.success {
    --state: var(--success);
  }
  .ov-verdict p {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0 8px;
    margin: 0;
    font-size: 13.5px;
  }
  .ov-verdict strong {
    font-weight: 600;
  }
  .ov-problem {
    font-weight: 500;
    text-decoration: none;
  }
  .ov-problem:hover {
    text-decoration: underline;
  }
  .ov-problem.error {
    color: var(--error);
  }
  .ov-problem.warning {
    color: var(--warning);
  }
  .ov-health {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    margin-left: auto;
    color: var(--muted);
    font-size: 11px;
  }
  .ov-health a,
  .ov-health span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: inherit;
    text-decoration: none;
  }
  .ov-health a:hover {
    color: var(--ink);
  }
  .ov-health .warning {
    color: var(--warning);
  }
  .ov-health .error {
    color: var(--error);
  }

  .ov-top {
    display: grid;
    grid-template-columns: minmax(0, 1.1fr) minmax(0, 1fr);
    gap: 1px;
    background: var(--control-border) !important;
  }
  .ov-panel {
    display: flex;
    flex-direction: column;
    min-width: 0;
    background: var(--surface);
  }
  .ov-heading {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 38px;
    padding: 0 16px;
    border-bottom: 1px solid var(--line);
  }
  .ov-heading h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
  }
  .ov-heading > :last-child {
    margin-left: auto;
    font-size: 11px;
  }
  .ov-clear {
    display: flex;
    align-items: center;
    margin: 0;
    padding: 14px 16px;
    font-size: 12.5px;
  }

  /* Attention: one line per kind of trouble, its repeats folded into a count. */
  .ov-attention {
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) auto auto;
    align-items: center;
    gap: 0 12px;
    min-height: 46px;
    padding: 0 16px 0 14px;
    border-bottom: 1px solid var(--line);
    color: var(--ink);
    text-decoration: none;
  }
  .ov-attention:hover {
    background: var(--row-hover);
  }
  .ov-attention > i {
    align-self: stretch;
    background: var(--line);
  }
  .ov-attention-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 7px 0;
  }
  .ov-attention-text strong {
    font-size: 12.5px;
    font-weight: 600;
  }
  .ov-attention.error strong {
    color: var(--error);
  }
  .ov-attention.warning strong {
    color: var(--warning);
  }
  .ov-attention-text small {
    overflow: hidden;
    color: var(--muted);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ov-marks {
    width: 120px;
    height: 12px;
  }
  .ov-marks line {
    stroke: var(--line);
    vector-effect: non-scaling-stroke;
  }
  .ov-marks rect {
    fill: var(--muted);
  }
  .ov-attention.error .ov-marks rect {
    fill: var(--error);
  }
  .ov-attention.warning .ov-marks rect {
    fill: var(--warning);
  }
  /* An outline row is not a link: it does not answer the pointer. */
  .ov-attention.outline:hover,
  .ov-recording.outline:hover {
    background: transparent;
  }
  .ov-when {
    font-size: 11px;
    white-space: nowrap;
  }

  /* Live now: the recordings at work, in the Live page's state colours. */
  .ov-recording {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr) 90px;
    align-items: center;
    gap: 0 12px;
    min-height: 46px;
    padding: 6px 16px 6px 18px;
    border-bottom: 1px solid var(--line);
    color: var(--ink);
    text-decoration: none;
  }
  .ov-recording:hover {
    background: var(--row-hover);
  }
  .ov-recording.quiet {
    opacity: 0.55;
  }
  .ov-recording-who {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 2px 8px;
    min-width: 0;
  }
  .ov-recording-who strong {
    font-size: 12.5px;
    font-weight: 600;
  }
  .ov-recording-who .mono {
    font-size: 11px;
  }
  .live-state {
    flex-basis: 100%;
    font-size: 11px;
    white-space: nowrap;
  }
  .live-state.live {
    color: var(--ink);
  }
  .live-state.warning {
    color: var(--warning);
  }
  .live-state.error {
    color: var(--error);
  }
  .live-state.neutral {
    color: var(--muted);
  }
  .live-state.state-yours {
    color: var(--success);
  }
  .ov-recording-prompt {
    overflow: hidden;
    color: var(--muted);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ov-lane {
    width: 100%;
    height: 8px;
  }
  .ov-lane line {
    stroke: var(--line);
    vector-effect: non-scaling-stroke;
  }
  .ov-lane rect {
    fill: var(--muted);
    opacity: 0.55;
  }
  .ov-lane rect.success {
    fill: var(--success);
  }
  .ov-lane rect.warning {
    fill: var(--warning);
    opacity: 0.9;
  }
  .ov-lane rect.error {
    fill: var(--error);
    opacity: 0.9;
  }
  .ov-lane rect.open {
    fill: var(--live);
    opacity: 1;
  }
  .ov-more {
    padding: 8px 16px;
    color: var(--muted);
    font-size: 11.5px;
    text-decoration: none;
  }
  .ov-more:hover {
    color: var(--ink);
  }

  /* Tiles: the day in four numbers, each with what is behind it. */
  .ov-tiles {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 1px;
    background: var(--control-border) !important;
  }
  .ov-tile {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 10px 16px 12px;
    background: var(--surface);
  }
  .ov-tile > span {
    font-size: 11.5px;
  }
  .ov-tile strong {
    font-size: 24px;
    font-weight: 500;
    line-height: 1.2;
  }
  .ov-tile strong.error {
    color: var(--error);
  }
  .ov-tile small {
    overflow: hidden;
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ov-spark {
    width: 100%;
    height: 22px;
    margin-top: 6px;
  }
  .ov-spark polyline {
    fill: none;
    stroke: var(--muted);
    stroke-width: 1.3;
    vector-effect: non-scaling-stroke;
  }
  .ov-cache svg {
    display: block;
    width: 100%;
    height: 6px;
    margin-top: 14px;
  }
  .cached {
    fill: color-mix(in srgb, var(--chart) 35%, transparent);
    background: color-mix(in srgb, var(--chart) 35%, transparent);
  }
  .fresh {
    fill: var(--chart);
    background: var(--chart);
  }

  /* The day: input per hour, cached under fresh, with trouble ticked below. */
  .ov-legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    color: var(--muted);
    font-size: 11px;
  }
  .ov-legend span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .ov-legend i {
    width: 8px;
    height: 8px;
  }
  .ov-legend i.failed {
    background: var(--error);
  }
  .ov-legend i.stopped {
    background: var(--warning);
  }
  .ov-legend i.bookkeeping {
    background: var(--muted);
  }
  /* The page fills the window: the day's chart takes whatever height is left. */
  .ov-console {
    min-height: 100%;
  }
  .ov-console > .ov-chart,
  .ov-console > .ov-first {
    flex: 1 0 auto;
  }
  .ov-day {
    display: block;
    flex: 1;
    width: calc(100% - 32px);
    min-height: 160px;
    margin: 10px 16px 0;
  }
  /* The chart's outline takes the chart's place: its bars, ticks, and axis. */
  .ov-day-outline {
    position: relative;
    flex: 1;
    min-height: 160px;
    margin: 10px 16px 34px;
  }
  .ov-day-outline > div {
    position: absolute;
    inset: 0;
  }
  .ov-start {
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
  }
  .ov-start-command {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .ov-start-command code {
    font-size: 11.5px;
  }
  .ov-first {
    align-items: flex-start;
    gap: 10px;
    padding: 20px 16px;
  }
  .ov-first h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .ov-first p {
    max-width: 640px;
    margin: 0;
    font-size: 12.5px;
  }
  .ov-day .grid {
    stroke: var(--line);
    stroke-dasharray: 2 3;
    vector-effect: non-scaling-stroke;
  }
  .ov-day .base {
    stroke: var(--control-border);
    vector-effect: non-scaling-stroke;
  }
  .ov-ticks {
    display: block;
    width: calc(100% - 32px);
    height: 12px;
    margin: 0 16px;
  }
  .ov-ticks .tick.failed {
    fill: var(--error);
  }
  .ov-ticks .tick.stopped {
    fill: var(--warning);
  }
  .ov-ticks .tick.bookkeeping {
    fill: var(--muted);
  }
  .ov-axis {
    display: flex;
    justify-content: space-between;
    padding: 2px 16px 10px;
    color: var(--muted);
    font-size: 10.5px;
  }

  @media (max-width: 900px) {
    .ov-top {
      grid-template-columns: minmax(0, 1fr);
    }
    .ov-tiles {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .ov-health {
      margin-left: 0;
    }
  }
  @media (max-width: 560px) {
    .ov-recording {
      grid-template-columns: minmax(0, 1fr) 70px;
    }
    .ov-recording-prompt {
      display: none;
    }
    .ov-marks {
      display: none;
    }
  }
</style>
