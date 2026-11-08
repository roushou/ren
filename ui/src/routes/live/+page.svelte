<script lang="ts">
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { prefersReducedMotion } from 'svelte/motion';
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { clock, providerName, short } from '#lib/api.ts';
  import { compactOrDash, span } from '#lib/display.ts';
  import { harnessLabel } from '#lib/activity.ts';
  import {
    Band,
    Board,
    LivePlace,
    type BandStyle,
    type Density,
    Feed,
    type FeedItem,
    type LiveWindow,
    type Tile,
  } from '#lib/board.ts';
  import Icon from '#lib/Icon.svelte';
  import Select from '#lib/Select.svelte';
  import Markdown from '#lib/Markdown.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);
  const snapshot = createSnapshot(() => data.live);
  const current = $derived(snapshot.current.value);

  // Elapsed times tick between refreshes.
  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  const view = $derived(LivePlace.parse(page.url.searchParams));
  const board = $derived(
    current
      ? new Board({
          sessions: current.sessions,
          calls: current.calls,
          turns: current.turns,
          now: Math.max(now, current.read),
          windowMs: LivePlace.windows[current.window].ms,
        })
      : null,
  );
  const band = $derived(
    current && board ? new Band(current.sessions, current.calls, board.now) : null,
  );
  const windowOptions = (Object.keys(LivePlace.windows) as LiveWindow[]).map((value) => ({
    value,
    label: LivePlace.windows[value].label,
  }));
  const chooseWindow = (window: LiveWindow) => goto(LivePlace.href({ ...view, window }));
  const longer = $derived(LivePlace.longer(view.window));
  /** The session whose line in the Now strip is under the pointer. */
  let highlighted = $state<string | null>(null);
  /** How long ago a feed item happened, as short as it gets. */
  function ago(item: FeedItem, now: number): string {
    const seconds = Math.max(0, Math.round((now - item.at) / 1000));
    if (seconds < 3) return 'now';
    return seconds < 60 ? `${seconds}s` : `${Math.round(seconds / 60)}m`;
  }
  const emptySlots = $derived(Math.max(0, Board.maxTiles - (board?.tiles.length ?? 0)));
  const densityHref = (density: Density) => LivePlace.href({ ...view, density });
  const bandHref = (style: BandStyle) => LivePlace.href({ ...view, band: style });

  const sessionHref = (id: string) => `/sessions/${encodeURIComponent(id)}`;
  const harnessName = (tile: Tile) =>
    tile.harness ? harnessLabel(tile.harness) : 'Unknown client';
  const percent = (share: number | null) => (share === null ? '—' : `${Math.round(share * 100)}%`);
  const harnessClass = (tile: Tile) => `harness-${tile.harness ?? 'unknown'}`;
  /** The tile's facts line: model, input with its cached share, output. */
  function facts(tile: Tile): string {
    const parts = [tile.model ?? 'model —'];
    parts.push(
      tile.context === null
        ? 'in —'
        : `in ${compactOrDash(tile.context)} (${percent(tile.cached)} cached)`,
    );
    if (tile.output !== null) parts.push(`out ${compactOrDash(tile.output)}`);
    return parts.join(' · ');
  }
  /** The newest thing the session did, for a row. */
  function lastLine(tile: Tile): string {
    const step = tile.steps.at(-1);
    if (step?.kind === 'tool') return `${step.name}${step.target ? ` ${step.target}` : ''}`;
    if (step?.kind === 'reply') return step.text;
    return tile.prompt?.text ?? '';
  }
</script>

{#snippet scope()}
  {#if board}
    <div class="scope-group live-pulse" aria-label="Right now">
      <span class="live-active">
        <span class="chip">{board.tiles.length + board.overflow} active</span><Select
          label="How far back a session counts as active"
          size="compact"
          options={windowOptions}
          value={view.window}
          onchange={chooseWindow}
        />
      </span>
      <span class="chip" class:live={board.inFlight > 0}
        ><span class="status-dot" class:live={board.inFlight > 0}></span>{board.inFlight} in flight</span
      >
      {#if board.overflow}<a
          class="chip"
          href="/sessions"
          title="Active sessions without a tile: the board keeps the busiest and most recent {Board.maxTiles}"
          >{board.overflow} not shown</a
        >{/if}
      {#if current && (current.truncated.calls || current.truncated.sessions)}<span
          class="chip warning"
          title="The window holds more than the page reads at once ({LivePlace.windows[
            current.window
          ].calls} calls, {LivePlace.windows[current.window]
            .sessions} sessions): the oldest are left out. Choose a shorter window to see all of it."
          >partial: newest only</span
        >{/if}
    </div>
  {/if}
  <div class="scope-group end">
    <nav class="segmented live-modes" aria-label="Band style">
      <a
        href={bandHref('area')}
        class:chosen={view.band === 'area'}
        aria-current={view.band === 'area' ? 'page' : undefined}
        title="One chart per client">Area</a
      >
      <a
        href={bandHref('ribbon')}
        class:chosen={view.band === 'ribbon'}
        aria-current={view.band === 'ribbon' ? 'page' : undefined}
        title="Every client stacked into one chart: its top edge is the total">Ribbon</a
      >
    </nav>
    <nav class="segmented live-modes" aria-label="Board density">
      <a
        href={densityHref('comfortable')}
        class:chosen={view.density === 'comfortable'}
        aria-current={view.density === 'comfortable' ? 'page' : undefined}
        title="Cards with each session's latest steps"><Icon name="grid" size={13} />Cards</a
      >
      <a
        href={densityHref('compact')}
        class:chosen={view.density === 'compact'}
        aria-current={view.density === 'compact' ? 'page' : undefined}
        title="One row per session"><Icon name="rows" size={13} />Rows</a
      >
    </nav>
  </div>
{/snippet}

{#snippet lane(tile: Tile)}
  <svg
    class="live-lane"
    viewBox="0 0 1000 10"
    preserveAspectRatio="none"
    role="img"
    aria-label="Calls in the last {span(Board.laneMs)}"
  >
    <line class="live-lane-base" x1="0" y1="5" x2="1000" y2="5" />
    {#each tile.lane as mark (mark.call.id)}
      <rect
        class="live-mark {mark.tone}"
        class:open={mark.open}
        x={mark.start * 1000}
        y="1"
        width={Math.max(3, (mark.end - mark.start) * 1000)}
        height="8"
        ><title
          >{mark.call.model ?? mark.call.endpoint} · {clock(mark.call.started_at)}{mark.open
            ? ' · open'
            : ''}</title
        ></rect
      >
    {/each}
  </svg>
{/snippet}

{#snippet ticks(height: number)}
  {#each [200, 400, 600, 800] as x (x)}
    <line class="live-band-tick" x1={x} y1="0" x2={x} y2={height} />
  {/each}
{/snippet}

{#snippet marks(row: Band['rows'][number])}
  {#each row.marks as mark (mark.key)}
    <rect class="live-band-mark {mark.tone}" x={mark.at * 1000 - 1.5} y="2" width="3" height="9" />
  {/each}
{/snippet}

{#snippet identity(tile: Tile)}
  <span class="harness-mark {harnessClass(tile)}"></span><strong
    >{harnessName(tile)} → {providerName(tile.session.provider)}</strong
  >
  <span class="mono muted" title="The session ID your client reported"
    >{short(tile.session.native_id)}</span
  >
{/snippet}

<div class="live-page">
  {#if !current || !board}
    <div class="empty-state" role="alert">
      <div class="empty-icon"><Icon name="alert" size={26} /></div>
      <h2>Live activity could not be read</h2>
      <p>{snapshot.current.error?.message ?? 'Loading…'}</p>
    </div>
  {:else}
    {#if band}
      <section
        class="live-band"
        aria-label="Output tokens by client"
        title="Output tokens per client over the last {span(Band.spanMs)}"
      >
        <div class="live-band-labels">
          {#each band.rows as row (row.harness)}
            <div
              class="live-band-label"
              title="Output tokens per second over the last {span(
                Band.rateMs,
              )}, counted when each call finishes"
            >
              <strong
                ><span class="harness-mark harness-{row.harness}"></span>{harnessLabel(
                  row.harness,
                )}</strong
              >
              <span class="mono"
                >{row.rate < 10 ? row.rate.toFixed(1) : Math.round(row.rate)} tok/s</span
              >
            </div>
          {/each}
        </div>
        <div class="live-band-charts">
          {#if view.band === 'ribbon'}
            <svg
              class="live-band-row ribbon"
              viewBox="0 0 1000 137"
              preserveAspectRatio="none"
              aria-hidden="true"
            >
              {@render ticks(137)}
              <line class="live-band-tick" x1="0" y1="68.5" x2="1000" y2="68.5" />
              {#each band.rows as row, index (row.harness)}
                {#if row.bins.some((value) => value > 0)}
                  {@const layer = band.layer(index, 137)}
                  <path class="live-band-area harness-{row.harness}" d={layer.area} />
                  <path class="live-band-line harness-{row.harness}" d={layer.line} />
                {/if}
              {/each}
              {#each band.rows as row (row.harness)}
                {@render marks(row)}
              {/each}
            </svg>
            <span class="live-band-scale mono" aria-hidden="true">{band.ribbonMax} tok/s</span>
          {:else}
            {#each band.rows as row (row.harness)}
              <svg
                class="live-band-row harness-{row.harness}"
                viewBox="0 0 1000 46"
                preserveAspectRatio="none"
                aria-hidden="true"
              >
                {@render ticks(46)}
                <path class="live-band-area" d={band.area(row, 46)} />
                <path class="live-band-line" d={band.line(row, 46)} />
                {@render marks(row)}
              </svg>
            {/each}
          {/if}
          <div class="live-band-axis mono" aria-hidden="true">
            <span>−10m</span><span>−8m</span><span>−6m</span><span>−4m</span><span>−2m</span><span
              >now</span
            >
          </div>
        </div>
      </section>
    {/if}
    <section class="live-now" aria-label="Now" aria-live="off">
      <span class="live-now-label"><span class="status-dot live"></span>Now</span>
      <div class="live-now-feed">
        {#each board.feed as item (item.key)}
          <a
            class="live-now-item {item.tone}"
            class:pinned={item.pinned}
            class:fresh={!item.pinned && board.now - item.at < 3000}
            href={item.href}
            title={item.detail ?? undefined}
            in:slide={{ axis: 'x', duration: prefersReducedMotion.current ? 0 : 180 }}
            onpointerenter={() => (highlighted = item.session?.id ?? null)}
            onpointerleave={() => (highlighted = null)}
            ><i></i>{#if item.session}<b>{short(item.session.native_id)}</b>{/if}<span
              >{item.text}</span
            >{#if item.detail}<span class="muted">{item.detail}</span>{/if}{#if !item.pinned}<span
                class="live-now-age">{ago(item, board.now)}</span
              >{/if}</a
          >
        {:else}
          <span class="live-now-item calm">Quiet: nothing in the last {span(Feed.calmMs)}</span>
        {/each}
      </div>
    </section>
    {#if !board.tiles.length}
      <div class="empty-state">
        <div class="empty-icon"><Icon name="pulse" size={26} /></div>
        <h2>No session active in the {LivePlace.windows[view.window].label}</h2>
        <p>Sessions appear here as soon as a client sends a call through TGV.</p>
        {#if longer}<a class="button" href={LivePlace.href({ ...view, window: longer })}
            >Look back {LivePlace.windows[longer].label.replace('last ', '')}</a
          >{/if}
      </div>
    {:else if view.density === 'compact'}
      <div class="live-rows-wrap">
        <table class="live-rows">
          <thead>
            <tr>
              <th>Session</th>
              <th>Now</th>
              <th class="lane-column">Last {span(Board.laneMs)}</th>
              <th>Latest</th>
              <th class="numeric">Context</th>
            </tr>
          </thead>
          <tbody>
            {#each board.tiles as tile (tile.session.id)}
              {@const rowChip = Board.phaseChip(tile.phase, board.now)}
              <tr class:quiet={tile.quiet}>
                <td
                  ><a class="live-open" href={sessionHref(tile.session.id)} title="Open the session"
                    >{@render identity(tile)}</a
                  ></td
                >
                <td
                  ><span class="mono live-state {rowChip.tone}" title={rowChip.title}
                    >{rowChip.label}</span
                  ></td
                >
                <td class="lane-column">{@render lane(tile)}</td>
                <td class="live-latest">{lastLine(tile)}</td>
                <td class="numeric mono">{compactOrDash(tile.context)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <div class="live-wall">
        {#each board.tiles as tile (tile.session.id)}
          {@const attention = board.attention(tile.session.id)}
          {@const chip = Board.phaseChip(tile.phase, board.now)}
          <article
            class="live-tile {harnessClass(tile)} {attention ? `attention-${attention}` : ''}"
            class:quiet={tile.quiet}
            class:highlighted={highlighted === tile.session.id}
          >
            <header class="live-tile-head">
              <a class="live-open" href={sessionHref(tile.session.id)} title="Open the session"
                >{@render identity(tile)}</a
              >
              {#if tile.turnCalls}<span class="mono muted live-seq">· call {tile.turnCalls}</span
                >{/if}
              {#if tile.open > 1}<span class="mono live-open-count" title="Model calls open at once"
                  >{tile.open} open</span
                >{/if}
              <span
                class="mono muted end"
                title="Context the newest call carried; provider-reported"
                >ctx {compactOrDash(tile.context)}</span
              >
            </header>
            <div class="live-tile-body">
              {#if tile.prompt}
                <p class="live-sent"><span class="live-who">You</span>{tile.prompt.text}</p>
              {/if}
              {#each tile.steps as step (step.key)}
                {#if step.kind === 'tool'}
                  <p class="live-step-tool">
                    <span class="live-tool-tag">tool</span><b>{step.name}</b>
                    {#if step.target}<span class="live-target">{step.target}</span>{/if}
                    <span class="live-result {step.result.tone}">{step.result.label}</span>
                  </p>
                {:else}
                  <div class="live-step-reply"><Markdown text={step.text} /></div>
                {/if}
              {/each}
              {#if tile.pending}<p class="live-wait">
                  {tile.pending} call{tile.pending > 1 ? 's' : ''} not read yet
                </p>{/if}
              {#if !tile.prompt && !tile.steps.length && !tile.pending}
                <p class="live-wait">
                  {tile.turnId
                    ? 'Text appears once a call finishes and is read.'
                    : 'This client reports no turns, so there is no transcript.'}
                </p>
              {/if}
            </div>
            <footer class="live-tile-foot mono">
              <div class="live-foot-state">
                <span class="live-state {chip.tone}" title={chip.title}>{chip.label}</span>
                <a
                  class="text-button end"
                  href={sessionHref(tile.session.id)}
                  title="Open the session">Session <Icon name="arrow" size={11} /></a
                >
              </div>
              <div class="live-foot-facts" title="Provider-reported tokens of the newest call">
                {facts(tile)}
              </div>
            </footer>
          </article>
        {/each}
        {#each Array.from({ length: emptySlots }, (_, slot) => slot) as slot (slot)}
          <div class="live-slot" aria-hidden="true"></div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .live-page {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
    background: var(--control-border);
    font-size: 13px;
    line-height: 1.45;
  }
  .live-page > :global(*) {
    background: var(--surface);
  }
  .live-page.reading {
    overflow: hidden;
  }
  .live-page > :global(.empty-state) {
    flex: 1;
    justify-content: center;
  }
  :global(.live-modes) a {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 8px;
  }
  :global(.live-modes) .count-pill {
    margin-left: 2px;
  }
  .live-pulse {
    gap: 6px;
  }
  .live-pulse a.chip {
    cursor: pointer;
  }
  /* The count and the window it counts over read as one control. */
  .live-active {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .live-pulse .status-dot {
    margin-right: 2px;
  }

  /* Clients: one colour each, shared by the band and the tiles. */
  .live-page {
    --harness-claude_code: var(--accent);
    --harness-codex: var(--provider-codex-ink);
    --harness-pi: var(--role-tool);
    --harness-unknown: var(--muted);
    --mono: 'SFMono-Regular', Consolas, 'Liberation Mono', monospace;
  }
  :global(:root[data-theme='dark']) .live-page {
    --harness-codex: #81a1c1;
  }
  .harness-claude_code {
    --harness: var(--harness-claude_code);
  }
  .harness-codex {
    --harness: var(--harness-codex);
  }
  .harness-pi {
    --harness: var(--harness-pi);
  }
  .harness-unknown {
    --harness: var(--harness-unknown);
  }
  .harness-mark {
    display: inline-block;
    flex: none;
    width: 8px;
    height: 8px;
    margin-right: 7px;
    background: var(--harness);
  }

  /* The band: output tokens per client, as calls finish. */
  .live-band {
    display: grid;
    flex: none;
    grid-template-columns: 150px minmax(0, 1fr);
  }
  .live-band-label {
    display: flex;
    flex-direction: column;
    justify-content: center;
    height: 46px;
    padding: 0 12px 0 16px;
    border-right: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  .live-band-label strong {
    display: flex;
    overflow: hidden;
    white-space: nowrap;
    align-items: center;
    font-size: 12.5px;
    font-weight: 600;
  }
  .live-band-label .mono {
    color: var(--muted);
    font-size: 11px;
  }
  .live-band-row {
    display: block;
    width: 100%;
    height: 46px;
    border-bottom: 1px solid var(--line);
  }
  .live-band-tick {
    stroke: var(--line);
    stroke-dasharray: 2 3;
    vector-effect: non-scaling-stroke;
  }
  .live-band-area {
    fill: var(--harness);
    opacity: 0.16;
  }
  .live-band-line {
    fill: none;
    stroke: var(--harness);
    stroke-width: 1.4;
    vector-effect: non-scaling-stroke;
  }
  .live-band-mark.warning {
    fill: var(--warning);
  }
  .live-band-mark.error {
    fill: var(--error);
  }
  /* Time labels sit inside the chart, along its bottom edge. */
  .live-band-charts {
    position: relative;
    min-width: 0;
  }
  .live-band-row.ribbon {
    height: 138px;
  }
  .live-band-row.ribbon .live-band-area {
    opacity: 0.28;
  }
  span.live-band-scale.mono {
    position: absolute;
    top: 2px;
    left: 4px;
    color: var(--muted);
    font-size: 9.5px;
    opacity: 0.7;
    pointer-events: none;
  }
  div.live-band-axis.mono {
    position: absolute;
    right: 0;
    bottom: 2px;
    left: 0;
    display: flex;
    justify-content: space-between;
    padding: 0 3px;
    color: var(--muted);
    font-size: 9.5px;
    opacity: 0.7;
    pointer-events: none;
  }

  /* The wall: fixed slots that fill the workspace; tiles touch, split by 1px rules. */
  /* Now: what the gateway saw lately, newest first, sliding in at the left. */
  .live-now {
    display: flex;
    flex: none;
    min-height: 30px;
    overflow: hidden;
  }
  .live-now-label {
    display: flex;
    flex: none;
    align-items: center;
    gap: 7px;
    width: 150px;
    padding: 0 12px 0 16px;
    border-right: 1px solid var(--line);
    color: var(--muted);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .live-now-feed {
    display: flex;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    mask-image: linear-gradient(to right, #000 85%, transparent);
  }
  .live-now-item {
    display: flex;
    flex: none;
    align-items: center;
    gap: 6px;
    padding: 0 12px;
    border-right: 1px solid var(--line);
    color: var(--muted);
    font: 11.5px var(--mono);
    text-decoration: none;
    white-space: nowrap;
    transition: background-color 1.2s;
  }
  .live-now-item i {
    flex: none;
    width: 7px;
    height: 7px;
    background: var(--muted);
  }
  .live-now-item b {
    color: var(--ink);
    font-weight: 500;
  }
  .live-now-item .muted {
    opacity: 0.75;
  }
  .live-now-age {
    color: var(--muted);
    font-size: 10.5px;
  }
  .live-now-item.success i {
    background: var(--success);
  }
  .live-now-item.warning {
    color: var(--warning);
  }
  .live-now-item.warning i {
    background: var(--warning);
  }
  .live-now-item.error {
    color: var(--error);
  }
  .live-now-item.error i {
    background: var(--error);
  }
  .live-now-item.pinned {
    background: var(--surface-inset);
  }
  .live-now-item.fresh {
    background: var(--row-hover);
    transition: none;
  }
  a.live-now-item:hover {
    background: var(--row-hover);
    transition: none;
  }
  .live-now-item.calm {
    color: var(--muted);
  }

  /* A fixed 4 × 2 wall that fills the rest of the page; tiles clip their own text. */
  .live-wall {
    display: grid;
    flex: 1;
    gap: 1px;
    min-height: 0;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    grid-template-rows: repeat(2, minmax(0, 1fr));
    background: var(--line) !important;
  }
  .live-slot {
    background: var(--canvas);
  }
  .live-tile {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--surface);
  }
  /*
   * Quiet sessions fade to dim once, as they go quiet; hovering brings one
   * back at once, with no transition either way.
   */
  .live-tile.quiet,
  .live-rows tr.quiet {
    opacity: 0.4;
    animation: live-dim 600ms ease-out;
  }
  @keyframes live-dim {
    from {
      opacity: 1;
    }
  }
  .live-tile.quiet:hover,
  .live-tile.quiet:focus-within,
  .live-tile.quiet.highlighted,
  .live-rows tr.quiet:hover {
    opacity: 1;
  }
  .live-tile.attention-warning {
    box-shadow: inset 0 2px 0 var(--warning);
  }
  .live-tile.attention-error {
    box-shadow: inset 0 2px 0 var(--error);
  }
  /* Hovering a tile, or its line in the Now strip, outlines it in its client's colour. */
  .live-tile:hover::after,
  .live-tile.highlighted::after {
    position: absolute;
    inset: 0;
    box-shadow: inset 0 0 0 2px var(--harness);
    content: '';
    pointer-events: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .live-tile.quiet,
    .live-rows tr.quiet {
      animation: none;
    }
  }
  .live-tile-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    min-width: 0;
    font-size: 12.5px;
  }
  .live-tile-head {
    border-bottom: 1px solid var(--line);
  }
  .live-tile-head > .mono {
    flex: none;
    font-size: 11px;
    white-space: nowrap;
  }
  .live-open-count {
    color: var(--live);
  }
  .end {
    margin-left: auto;
  }
  .live-open {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    text-align: left;
  }
  .live-open strong,
  .live-open .mono {
    white-space: nowrap;
  }
  .live-open strong {
    font-weight: 600;
  }
  .live-open .mono {
    overflow: hidden;
    font-size: 11px;
    text-overflow: ellipsis;
  }
  .live-open:hover strong {
    text-decoration: underline;
  }
  .live-lane {
    display: block;
    width: 100%;
    height: 10px;
  }
  .live-lane-base {
    stroke: var(--line);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  .live-mark {
    fill: var(--muted);
    opacity: 0.55;
  }
  .live-mark.success {
    fill: var(--success);
  }
  .live-mark.warning {
    fill: var(--warning);
    opacity: 0.9;
  }
  .live-mark.error {
    fill: var(--error);
    opacity: 0.9;
  }
  .live-mark.live,
  .live-mark.open {
    fill: var(--live);
    opacity: 1;
    animation: live-breathe 1.6s ease-in-out infinite;
  }
  @keyframes live-breathe {
    50% {
      opacity: 0.45;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .live-mark.open,
    .live-mark.live {
      animation: none;
    }
  }
  .live-tile .live-lane {
    padding: 4px 12px 2px;
    box-sizing: content-box;
    width: calc(100% - 24px);
  }
  /* The body reads bottom up, newest last; older lines fade out at the top. */
  .live-tile-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    min-height: 0;
    padding: 6px 12px;
    overflow: hidden;
    color: var(--ink);
    font-size: 12.5px;
    line-height: 1.5;
    text-align: left;
    -webkit-mask-image: linear-gradient(to bottom, transparent 0, #000 38px);
    mask-image: linear-gradient(to bottom, transparent 0, #000 38px);
  }
  .live-tile-body > p {
    flex: none;
    margin: 0 0 6px;
    overflow-wrap: anywhere;
  }
  .live-tile-body > p:last-child {
    margin-bottom: 0;
  }
  .live-who {
    margin-right: 7px;
    color: var(--ink);
    font-family: inherit;
    font-size: 9.5px;
    font-weight: 600;
    letter-spacing: 0.07em;
    text-transform: uppercase;
  }
  .live-sent {
    padding: 4px 7px;
    background: var(--surface-inset);
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11.5px;
  }
  .live-step-tool {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    padding-left: 7px;
    border-left: 2px solid var(--line);
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11.5px;
    white-space: nowrap;
  }
  .live-tool-tag {
    color: var(--muted);
  }
  .live-step-tool b {
    color: var(--ink);
    font-weight: 500;
  }
  .live-result {
    flex: none;
    margin-left: auto;
    color: var(--muted);
  }
  .live-result.success {
    color: var(--success);
  }
  .live-result.warning {
    color: var(--warning);
  }
  .live-result.error {
    color: var(--error);
  }
  .live-tile-body > .live-step-reply {
    flex: none;
    margin: 0 0 6px;
  }
  .live-step-reply :global(.markdown) {
    font-size: 12.5px;
    line-height: 1.5;
  }
  .live-step-reply :global(.markdown > *) {
    margin-bottom: 6px;
  }
  .live-step-reply :global(.md-heading) {
    margin: 6px 0 3px;
    font-size: 12.5px;
  }
  .live-wait {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11.5px;
    opacity: 0.7;
  }
  .live-target,
  .live-latest {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .live-target {
    color: var(--muted);
  }
  .live-tile-foot {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 12px;
    border-top: 1px solid var(--line);
    color: var(--muted);
    font-size: 11px;
  }
  .live-foot-state {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .live-foot-facts {
    overflow: hidden;
    color: var(--muted);
    font-size: 10.5px;
    opacity: 0.7;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .live-state {
    overflow: hidden;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .live-state.live {
    color: var(--ink);
  }
  .live-state.success {
    color: var(--success);
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
  .live-tile-foot .text-button {
    flex: none;
    font-size: 11px;
  }
  .live-quiet {
    margin: 0;
    padding: 8px 12px;
    color: var(--muted);
  }

  /* Rows: the same board, one line per session. */
  .live-rows-wrap {
    flex: 1;
    min-height: 0;
  }
  .live-rows {
    width: 100%;
    font-size: 12.5px;
    border-collapse: collapse;
  }
  .live-rows th {
    padding: 5px 12px;
    font-size: 11px;
    border-bottom: 1px solid var(--line);
    color: var(--muted);
    font-weight: 500;
    text-align: left;
    white-space: nowrap;
  }
  .live-rows td {
    padding: 4px 12px;
    border-bottom: 1px solid var(--line-subtle);
    vertical-align: middle;
  }
  .live-rows .numeric {
    text-align: right;
  }
  .live-rows .pick-column {
    width: 1%;
    padding-right: 0;
  }
  .live-rows .lane-column {
    width: 28%;
    min-width: 160px;
  }
  .live-rows .live-latest {
    max-width: 0;
    width: 40%;
  }

  @media (max-width: 1100px) {
    .live-wall {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      grid-template-rows: repeat(4, minmax(0, 1fr));
    }
  }
  @media (max-width: 760px) {
    .live-wall {
      flex: none;
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: none;
      grid-auto-rows: 260px;
    }
    .live-slot {
      display: none;
    }
    .live-band {
      grid-template-columns: 96px minmax(0, 1fr);
    }
    .live-now-label {
      width: 96px;
      padding-left: 12px;
    }
    .live-rows .lane-column,
    .live-rows .live-latest {
      display: none;
    }
  }
</style>
