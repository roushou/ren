<script lang="ts">
  import { Clock, Elapsed } from '#lib/time.ts';
  import Toolbar from '#lib/Toolbar.svelte';
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { prefersReducedMotion } from 'svelte/motion';
  import { page } from '$app/state';
  import { RecentCalls } from '#lib/loaders.ts';
  import { goto } from '$app/navigation';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { num, providerName, short } from '#lib/api.ts';
  import { turnHref } from '#lib/activity.ts';
  import { compactOrDash } from '#lib/display.ts';
  import type { Excerpt } from '#lib/api.ts';
  import { Steps } from '#lib/steps.ts';
  import {
    Band,
    Board,
    LivePlace,
    type BandBy,
    type BandSeries,
    type Density,
    Feed,
    type LiveWindow,
    type Tile,
    type ResultMark,
    type ToolLine,
    type TurnCall,
    type UnreadCall,
  } from '#lib/board.ts';
  import Icon from '#lib/Icon.svelte';
  import Glyph from '#lib/Glyph.svelte';
  import { Marks } from '#lib/marks.ts';
  import Segmented, { type SegmentedOption } from '#lib/Segmented.svelte';
  import Markdown from 'markdown/Markdown.svelte';
  import StreamingReply from '#lib/StreamingReply.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import SkeletonRows from '#lib/SkeletonRows.svelte';
  import type { Column } from '#lib/columns.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const workbench = getWorkbench();
  const arrival = createArrival(() => data.live);
  const shown = $derived(arrival.current);
  /** The window's reads are on their way: the page draws its outline. */
  const pending = $derived(shown.state === 'pending');
  const snapshot = $derived(shown.state === 'ready' ? shown.snapshot : null);
  const current = $derived(snapshot?.value ?? null);

  // Elapsed times tick between refreshes.
  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  /** The time the board and band are built at: on `Board.tickMs`, not every
   * second, so the wall is not rebuilt from every call each tick. */
  const builtAt = $derived(
    current ? Math.max(Math.floor(now / Board.tickMs) * Board.tickMs, current.read) : now,
  );
  /** The time elapsed labels read: every second, never before the read. */
  const labelAt = $derived(Math.max(now, current?.read ?? now));
  const view = $derived(LivePlace.parse(page.url.searchParams));
  const band = $derived(
    current
      ? new Band(current.recordings, current.recent, builtAt, view.by, current.complete)
      : null,
  );
  /** The series the wall is narrowed to: the band's, or the bare key while it draws nothing. */
  const only = $derived(
    view.series
      ? {
          by: view.by,
          label: band?.series.find((series) => series.key === view.series)?.label ?? view.series,
          keys: band?.series.find((series) => series.key === view.series)?.members ?? [view.series],
        }
      : null,
  );
  const board = $derived(
    current
      ? new Board({
          recordings: current.recordings,
          calls: current.calls,
          turns: current.turns,
          now: builtAt,
          windowMs: LivePlace.windows[current.window].ms,
          focus: view.focus,
          pins: view.pins,
          only,
        })
      : null,
  );
  /** The band series under the pointer: its layer stands out and its cards are outlined. */
  let hoveredSeries = $state<string | null>(null);
  const seriesHref = (key: string) =>
    LivePlace.href({ ...view, series: view.series === key ? null : key });
  const byOptions: SegmentedOption<BandBy>[] = [
    { value: 'client', label: 'By client' },
    { value: 'provider', label: 'By provider' },
    { value: 'model', label: 'By model' },
  ];
  const densityOptions: SegmentedOption<Density>[] = [
    { value: 'comfortable', label: 'Cards' },
    { value: 'compact', label: 'Rows' },
  ];
  /** Whether a card belongs to the hovered band series. */
  function inHoveredSeries(tile: Tile): boolean {
    if (!hoveredSeries || !band) return false;
    const key = Band.recordingKey(view.by, tile.recording, tile.model);
    return band.seriesOf(key)?.key === hoveredSeries;
  }
  /** The band's height in SVG units, drawn at the same height in pixels. */
  const BAND_HEIGHT = 120;
  const rate = (value: number) => (value < 10 ? value.toFixed(1) : String(Math.round(value)));
  const windowOptions = (Object.keys(LivePlace.windows) as LiveWindow[]).map((value) => ({
    value,
    label: LivePlace.windows[value].label.replace('last ', ''),
  }));
  const longer = $derived(LivePlace.longer(view.window));
  /** The recording whose line in the Now strip is under the pointer. */
  let highlighted = $state<string | null>(null);
  /** The rows view's columns: its header and its outline rows are drawn from them. */
  const rowColumns: Column[] = [
    { label: 'Session' },
    { label: 'Now' },
    { label: `Last ${Elapsed.span(Board.laneMs)}`, class: 'lane-column' },
    { label: 'Latest', class: 'live-latest' },
    { label: 'Context', numeric: true },
  ];
  /** Text widths for the outline cards' lines, varied so they read as text. */
  const outlineLines = ['82%', '64%', '91%', '57%', '76%', '69%', '88%', '61%'];
  const emptySlots = $derived(board ? Math.max(0, board.capacity - board.tiles.length) : 0);
  const pinHref = (tile: Tile) => LivePlace.href(LivePlace.togglePin(view, tile.recording.id));
  /** The call a card is about: the open or last one its state names, else the newest. */
  function callHref(tile: Tile): string | null {
    const call = tile.phase.kind === 'idle' ? tile.latest : tile.phase.call;
    return call ? `${recordingHref(tile.recording.id)}?call=${encodeURIComponent(call.id)}` : null;
  }
  const focusHref = (tile: Tile) =>
    LivePlace.href({
      ...view,
      focus: board?.focus === tile.recording.id ? null : tile.recording.id,
    });
  /** Escape closes the expanded card, unless a control has the key. */
  function onkeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape' || !board?.focus || event.defaultPrevented) return;
    const target = event.target;
    if (
      target instanceof HTMLElement &&
      target.closest('input, select, textarea, [role="listbox"]')
    )
      return;
    goto(LivePlace.href({ ...view, focus: null }));
  }
  /** A call not read yet: open (waiting for its head, or responding), or just finished. */
  function unreadLabel(unread: UnreadCall, now: number, thinking: boolean): string {
    const { call } = unread;
    if (call.state !== 'open') return 'finished · reading';
    const state =
      call.status === null ? 'waiting for response' : thinking ? 'thinking' : 'responding';
    return `${state} · ${Elapsed.span(now - call.started_at)}`;
  }
  function callLabel(call: TurnCall): string {
    const parts = [`call ${call.index ?? '—'}`];
    if (call.durationMs !== null) parts.push(Elapsed.span(call.durationMs));
    if (call.output !== null) parts.push(`out ${compactOrDash(call.output)}`);
    return parts.join(' · ');
  }
  /**
   * The expanded card opens on its newest call and follows new ones while
   * it is scrolled to the end; scrolling up stops the following.
   */
  function stickToEnd(node: HTMLElement, active: boolean) {
    let following = active;
    let atEnd = true;
    const toEnd = () => (node.scrollTop = node.scrollHeight);
    const onscroll = () => (atEnd = node.scrollHeight - node.scrollTop - node.clientHeight < 24);
    const observer = new MutationObserver(() => {
      if (following && atEnd) toEnd();
    });
    node.addEventListener('scroll', onscroll);
    observer.observe(node, { childList: true, subtree: true, characterData: true });
    if (active) toEnd();
    return {
      update(next: boolean) {
        if (next && !following) {
          atEnd = true;
          toEnd();
        }
        following = next;
      },
      destroy() {
        observer.disconnect();
        node.removeEventListener('scroll', onscroll);
      },
    };
  }
  const more = (excerpt: Excerpt) =>
    excerpt.omitted_chars
      ? `\n… ${excerpt.omitted_chars.toLocaleString('en')} more characters`
      : '';

  const recordingHref = (id: string) => `/monitor/sessions/${encodeURIComponent(id)}`;
  const harnessClass = (tile: Tile) => `harness-${tile.harness ?? 'unknown'}`;
  /** What a card is called: its client, and the folder it works in when known. */
  function cardTitle(tile: Tile): string {
    const client = Board.clientName(tile.harness);
    return tile.workdir ? `${client} · ${Board.folder(tile.workdir)}` : client;
  }
  /** Output tokens a series made over the band's span. */
  const seriesTotal = (series: BandSeries) =>
    Math.round(series.bins.reduce((sum, value) => sum + value, 0) * (Band.binMs / 1000));
  const bandTotal = $derived(
    band ? band.series.reduce((sum, series) => sum + seriesTotal(series), 0) : 0,
  );
  const shareOf = (series: BandSeries) =>
    bandTotal ? `${Math.round((seriesTotal(series) / bandTotal) * 100)}%` : '—';
  const bandRate = $derived(band ? band.series.reduce((sum, series) => sum + series.rate, 0) : 0);
  /** The newest thing the recording did, for a row. */
  function lastLine(tile: Tile): string {
    const line = tile.lines.at(-1);
    if (line?.kind === 'tools')
      return `${line.tool.name}${line.tool.target ? ` ${line.tool.target}` : ''}`;
    if (line?.kind === 'reply') return line.text;
    return tile.prompt?.text ?? '';
  }
</script>

{#snippet toolLine(tool: ToolLine, count: number, result: ResultMark)}
  <div class="live-tool effect-{tool.effect ?? 'none'}">
    <span class="live-tool-icon"><Icon name={Steps.ICONS[Steps.act(tool.effect)]} size={12} /></span
    >
    <b
      >{tool.name}{#if count > 1}<span class="live-count"> ×{count}</span>{/if}</b
    >
    {#if tool.target}<span class="live-target" title={tool.target}>{tool.target}</span>{/if}
    <span class="live-result"><Glyph mark={Marks.ofTone(result.tone)} label={result.label} /></span>
    {#if tool.output}<span class="live-output" class:error={tool.result.tone === 'error'}
        >⎿ {tool.output}</span
      >{/if}
  </div>
{/snippet}

{#snippet scope()}
  <div class="scope-group live-pulse" aria-label="Right now" aria-busy={!board}>
    {#if board}
      <span class="chip" title="Sessions active in the window"
        ><Glyph mark="responding" label="Active" />{board.tiles.length + board.overflow}</span
      >
      <span class="chip" title="Model calls in flight now"
        ><Glyph
          mark={board.inFlight > 0 ? 'waiting' : 'idle'}
          label="In flight"
        />{board.inFlight}</span
      >
      {#if board.overflow}<a
          class="chip"
          href="/monitor/sessions"
          title="Active sessions without a card: the board keeps the busiest and most recent {Board.maxTiles}"
          >+{board.overflow}</a
        >{/if}
      {#if only}<a class="chip live-only" href={LivePlace.href({ ...view, series: null })}
          >only {only.label} <Icon name="close" size={11} /></a
        >{/if}
      {#if current && (current.truncated.calls || current.truncated.recordings)}<span
          class="chip warning"
          title="The window holds more than the page reads at once ({RecentCalls.pageLimit *
            RecentCalls.maxPages} calls, {LivePlace.windows[current.window]
            .recordings} recordings): the oldest are left out.">partial: newest only</span
        >{/if}
    {:else if pending}
      <span class="chip"><Glyph mark="responding" /><Skeleton width="1.5em" /></span>
      <span class="chip"><Glyph mark="idle" /><Skeleton width="1.5em" /></span>
    {/if}
  </div>
  <div class="scope-group end">
    <Segmented
      label="How far back a session counts as active"
      options={windowOptions}
      value={view.window}
      href={(window) => LivePlace.href({ ...view, window })}
    />
    <Segmented
      label="What the band groups output by"
      options={byOptions}
      value={view.by}
      href={(by) => LivePlace.href({ ...view, by, series: null })}
    />
    <Segmented
      label="Board density"
      options={densityOptions}
      value={view.density}
      href={(density) => LivePlace.href({ ...view, density })}
    />
  </div>
{/snippet}

{#snippet lane(tile: Tile)}
  <svg
    class="live-lane"
    viewBox="0 0 1000 10"
    preserveAspectRatio="none"
    role="img"
    aria-label="Calls in the last {Elapsed.span(Board.laneMs)}"
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
          >{mark.call.model ?? mark.call.endpoint} · {Clock.second(mark.call.started_at)}{mark.open
            ? ' · open'
            : ''}</title
        ></rect
      >
    {/each}
  </svg>
{/snippet}

{#snippet rowsHead()}
  <thead>
    <tr>
      {#each rowColumns as column (column.label)}<th
          class={column.class}
          class:numeric={column.numeric}>{column.label}</th
        >{/each}
    </tr>
  </thead>
{/snippet}

{#snippet ticks(height: number)}
  {#each [200, 400, 600, 800] as x (x)}
    <line class="live-band-tick" x1={x} y1="0" x2={x} y2={height} />
  {/each}
{/snippet}

{#snippet marks(series: BandSeries, top: number)}
  {#each series.marks as mark (mark.key)}
    <rect
      class="live-band-mark {mark.tone}"
      x={mark.at * 1000 - 1.5}
      y={top + 2}
      width="3"
      height="9"><title>{series.label}: {mark.label}</title></rect
    >
  {/each}
{/snippet}

{#snippet identity(tile: Tile)}
  <span class="harness-mark {harnessClass(tile)}"></span><strong
    >{Board.clientName(tile.harness)} → {providerName(tile.recording.provider)}</strong
  >
  <span class="mono muted" title="The session ID your client reported"
    >{short(tile.recording.native_id)}</span
  >
{/snippet}

<svelte:window {onkeydown} />

<Toolbar {scope} />
<div class="live-page">
  {#if pending}
    <section class="live-band" aria-label="Output tokens by {view.by}" aria-busy="true">
      <svg
        class="live-band-chart"
        viewBox="0 0 1000 {BAND_HEIGHT}"
        preserveAspectRatio="none"
        aria-hidden="true"
      >
        {@render ticks(BAND_HEIGHT)}
      </svg>
      <div class="live-legend" aria-hidden="true">
        {#each ['6em', '4.5em'] as width (width)}
          <span class="live-legend-chip"
            ><Skeleton {width} /><span class="mono"><Skeleton width="3.5em" /></span></span
          >
        {/each}
      </div>
      <div class="live-band-axis mono" aria-hidden="true">
        <span>−10m</span><span>−8m</span><span>−6m</span><span>−4m</span><span>−2m</span><span
          >now</span
        >
      </div>
    </section>
    <section class="live-now" aria-label="Now" aria-busy="true">
      <span class="live-now-label"><Glyph mark="idle" />Now</span>
      <div class="live-now-feed" aria-hidden="true">
        {#each ['9em', '12em', '7em', '10em'] as width (width)}
          <span class="live-now-item"><i></i><Skeleton {width} /></span>
        {/each}
      </div>
    </section>
    {#if view.density === 'compact'}
      <div class="live-rows-wrap" aria-busy="true">
        <table class="live-rows">
          {@render rowsHead()}
          <tbody><SkeletonRows columns={rowColumns} rows={Board.maxTiles} /></tbody>
        </table>
      </div>
    {:else}
      <div class="live-wall" aria-busy="true">
        {#each Array.from({ length: Board.maxTiles }, (_, slot) => slot) as slot (slot)}
          <div class="live-tile state-idle" aria-hidden="true">
            <header class="live-tile-head">
              <Skeleton width="{9 + (slot % 3) * 2}em" /><span class="mono muted end"
                >ctx <Skeleton width="2.5em" /></span
              >
            </header>
            <div class="live-tile-body">
              {#each { length: 3 }, line (line)}
                <p><Skeleton width={outlineLines[(slot + line * 3) % outlineLines.length]} /></p>
              {/each}
            </div>
            <footer class="live-tile-foot mono">
              <div class="live-foot-state"><Skeleton width="5em" /></div>
              <div class="live-foot-facts"><Skeleton width="8em" /></div>
            </footer>
          </div>
        {/each}
      </div>
    {/if}
  {:else if !current || !board}
    <div class="empty-state" role="alert">
      <div class="empty-icon"><Icon name="alert" size={26} /></div>
      <h2>Live activity could not be read</h2>
      <p>{snapshot?.error?.message ?? 'Loading…'}</p>
    </div>
  {:else}
    {#if band && band.series.length}
      <section class="live-band" aria-label="Output tokens by {view.by}">
        <div class="live-band-main">
          <div class="live-band-head">
            <span class="cap">Output tokens per second · last {Elapsed.span(Band.spanMs)}</span>
            <span class="live-band-rate mono">{rate(bandRate)}</span>
            <span class="meta"
              >tok/s now · peak {view.band === 'ribbon' ? band.ribbonMax : band.max}</span
            >
          </div>
          <div class="live-band-plot">
            <svg
              class="live-band-chart"
              class:hovering={hoveredSeries !== null}
              viewBox="0 0 1000 {BAND_HEIGHT}"
              preserveAspectRatio="none"
              aria-hidden="true"
            >
              {@render ticks(BAND_HEIGHT)}
              {#if band.unread > 0}
                <rect
                  class="live-band-unread"
                  x="0"
                  y="0"
                  width={band.unread * 1000}
                  height={BAND_HEIGHT}
                />
              {/if}
              {#if view.band === 'ribbon'}
                <line
                  class="live-band-tick"
                  x1="0"
                  y1={BAND_HEIGHT / 2}
                  x2="1000"
                  y2={BAND_HEIGHT / 2}
                />
                {#each band.series as series, index (series.key)}
                  {#if series.bins.some((value) => value > 0)}
                    {@const layer = band.layer(index, BAND_HEIGHT)}
                    <g class={series.swatch} class:lit={hoveredSeries === series.key}>
                      <path class="live-band-area" d={layer.area} />
                      <path class="live-band-line" d={layer.line} />
                    </g>
                  {/if}
                {/each}
                {#each band.series as series (series.key)}
                  {@render marks(series, 0)}
                {/each}
              {:else}
                {#each band.series as series, index (series.key)}
                  {@const row = band.row(index, BAND_HEIGHT)}
                  <g class={series.swatch} class:lit={hoveredSeries === series.key}>
                    {#if index > 0}<line
                        class="live-band-rule"
                        x1="0"
                        y1={row.top}
                        x2="1000"
                        y2={row.top}
                      />{/if}
                    <path class="live-band-area" d={row.area} />
                    <path class="live-band-line" d={row.line} />
                  </g>
                  {@render marks(series, row.top)}
                {/each}
              {/if}
            </svg>
          </div>
          <div class="live-band-axis mono" aria-hidden="true">
            <span>−10m</span><span>−8m</span><span>−6m</span><span>−4m</span><span>−2m</span><span
              >now</span
            >
          </div>
        </div>
        <div class="live-band-side">
          <span class="cap">By {view.by} · last {Elapsed.span(Band.spanMs)}</span>
          {#each band.series as series (series.key)}
            <a
              class="live-legend-row {series.swatch}"
              class:chosen={view.series === series.key}
              href={seriesHref(series.key)}
              title={view.series === series.key
                ? 'Show every session'
                : `Show only ${series.label} sessions`}
              onpointerenter={() => (hoveredSeries = series.key)}
              onpointerleave={() => (hoveredSeries = null)}
              onfocus={() => (hoveredSeries = series.key)}
              onblur={() => (hoveredSeries = null)}
              ><span class="harness-mark"></span><span class="ellipsis">{series.label}</span><span
                class="mono">{compactOrDash(seriesTotal(series))}</span
              ><span class="mono muted share">{shareOf(series)}</span></a
            >
          {/each}
          {#if band.series.some((series) => series.marks.length) || band.unread > 0}
            <span class="live-legend-key" aria-hidden="true"
              >{#if band.series.some((series) => series.marks.length)}<span
                  class="live-key-mark error"
                ></span>failed<span class="live-key-mark warning"></span>cut off{/if}{#if band.unread > 0}<span
                  class="live-key-mark unread"
                ></span>not read{/if}</span
            >
          {/if}
        </div>
      </section>
    {:else if band}
      <section class="live-band empty" aria-label="Output tokens">
        <span class="mono">No output in the last {Elapsed.span(Band.spanMs)}</span>
      </section>
    {/if}
    <section class="live-now" aria-label="Now" aria-live="off">
      <span class="live-now-label"><Glyph mark="responding" />Now</span>
      <div class="live-now-feed">
        {#each board.feed as item (item.key)}
          <a
            class="live-now-item {item.tone}"
            class:pinned={item.pinned}
            class:fresh={!item.pinned && board.now - item.at < 3000}
            href={item.href}
            title={item.detail ?? undefined}
            in:slide={{ axis: 'x', duration: prefersReducedMotion.current ? 0 : 180 }}
            onpointerenter={() => (highlighted = item.recording?.id ?? null)}
            onpointerleave={() => (highlighted = null)}
            ><i></i>{#if item.recording}<b>{short(item.recording.native_id)}</b>{/if}<span
              >{item.text}</span
            >{#if item.detail}<span class="muted">{item.detail}</span>{/if}{#if !item.pinned}<span
                class="live-now-age">{Elapsed.ago(item.at, labelAt)}</span
              >{/if}</a
          >
        {:else}
          <span class="live-now-item calm"
            ><Glyph mark="idle" label="Nothing in the last {Elapsed.span(Feed.calmMs)}" /><span
              class="mono">0 · {Elapsed.span(Feed.calmMs)}</span
            ></span
          >
        {/each}
      </div>
    </section>
    {#if !board.tiles.length}
      <div class="empty-state">
        <div class="empty-icon"><Icon name="pulse" size={26} /></div>
        <h2>No session active in the {LivePlace.windows[view.window].label}</h2>
        {#if longer}<a class="button" href={LivePlace.href({ ...view, window: longer })}
            >Look back {LivePlace.windows[longer].label.replace('last ', '')}</a
          >{/if}
      </div>
    {:else if view.density === 'compact'}
      <div class="live-rows-wrap">
        <table class="live-rows">
          {@render rowsHead()}
          <tbody>
            {#each board.tiles as tile (tile.recording.id)}
              {@const rowChip = Board.phaseChip(tile.phase, board.now)}
              <tr class:quiet={tile.quiet}>
                <td
                  ><a
                    class="live-open"
                    href={recordingHref(tile.recording.id)}
                    title="Open the session">{@render identity(tile)}</a
                  ></td
                >
                <td
                  ><span class="mono live-state {rowChip.tone}" title={rowChip.title}
                    ><Glyph mark={rowChip.mark} label={rowChip.label} /></span
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
        {#each board.tiles as tile (tile.recording.id)}
          {@const attention = board.attention(tile.recording.id)}
          {@const chip = Board.phaseChip(tile.phase, board.now)}
          {@const focused = board.focus === tile.recording.id}
          {@const toCall = callHref(tile)}
          <article
            class="live-tile state-{chip.mark} {harnessClass(tile)} {attention
              ? `attention-${attention}`
              : ''}"
            class:quiet={tile.quiet && !focused}
            class:focused
            class:highlighted={highlighted === tile.recording.id || inHoveredSeries(tile)}
          >
            <header class="live-tile-head">
              <Glyph mark={chip.mark} label={chip.label} />
              <a
                class="live-open"
                href={recordingHref(tile.recording.id)}
                title="{Board.clientName(tile.harness)} → {providerName(
                  tile.recording.provider,
                )} · {short(tile.recording.native_id)}"
                ><span class="live-title">{cardTitle(tile)}</span></a
              >
              {#if tile.open > 1}<span
                  class="mono live-open-count"
                  title="Model calls in flight at once: a subagent or a side request beside the main one"
                  ><span class="muted">·</span> {tile.open} in flight</span
                >{/if}
              {#if tile.pinned}<span
                  class="live-pinned"
                  title="Pinned: stays on the wall while the session is active"
                  ><Icon name="pin" size={12} /></span
                >{/if}
              <span class="mono muted end live-state-word" title={chip.title}>{chip.label}</span>
              <div class="live-actions">
                {#if tile.turnId}<a
                    class="icon-button"
                    href={turnHref(tile.recording.id, tile.turnId)}
                    aria-label="Open the turn"
                    title="Open the turn: its prompt, tools, files, and findings"
                    ><Icon name="logs" size={13} /></a
                  >{/if}
                {#if toCall}<a
                    class="icon-button"
                    href={toCall}
                    aria-label="Open the call"
                    title="Open this call in its session, with the inspector"
                    ><Icon name="pulse" size={13} /></a
                  >{/if}
                <a
                  class="icon-button live-pin"
                  class:on={tile.pinned}
                  href={pinHref(tile)}
                  aria-label={tile.pinned ? 'Unpin this card' : 'Pin this card'}
                  title={tile.pinned
                    ? 'Unpin: busier sessions may take its place'
                    : 'Pin: keep it on the wall while the recording is active'}
                  ><Icon name="pin" size={13} /></a
                >
                <a
                  class="icon-button live-expand"
                  href={focusHref(tile)}
                  aria-label={focused ? 'Collapse this card' : 'Expand this card'}
                  title={focused
                    ? 'Collapse (Esc)'
                    : 'Expand: the whole turn, with tool input and output'}
                  ><Icon name={focused ? 'shrink' : 'expand'} size={13} /></a
                >
              </div>
            </header>
            <div class="live-tile-body" class:full={focused} use:stickToEnd={focused}>
              {#if tile.prompt}
                <p class="live-sent"><span class="live-who">You</span>{tile.prompt.text}</p>
              {/if}
              {#if focused}
                {#each tile.calls as call (call.key)}
                  <p class="live-sep">{callLabel(call)}</p>
                  {#if call.reply}<div class="live-step-reply">
                      <Markdown text={call.reply} />
                    </div>{/if}
                  {#each call.tools as tool (tool.key)}
                    {@render toolLine(tool, 1, tool.result)}
                    <pre class="live-io" title="Input the model sent">{tool.input.text}{more(
                        tool.input,
                      )}</pre>
                    {#if tool.reported && tool.reported.text.trim()}<pre
                        class="live-io reported {tool.result.tone}"
                        title="Output the client reported">{tool.reported.text}{more(
                          tool.reported,
                        )}</pre>{/if}
                  {/each}
                {/each}
              {:else}
                {#each tile.lines as line (line.key)}
                  {#if line.kind === 'tools'}
                    {@render toolLine(line.tool, line.count, line.result)}
                  {:else}
                    <div class="live-step-reply"><Markdown text={line.text} /></div>
                  {/if}
                {/each}
              {/if}
              {#each tile.unread as unread (unread.call.id)}
                {@const streamed = workbench.streamed.get(unread.call.id)}
                <!-- Once text streams, it holds the call's place: no status line
                     comes and goes around it, and the read reply replaces it in place. -->
                {#if !streamed?.reply && (tile.turnId || unread.call.state === 'open')}
                  {@const thinking = unread.call.state === 'open' && !!streamed?.thinking}
                  <p class="live-sep live" class:open={unread.call.state === 'open'} class:thinking>
                    call {unread.index} · {unreadLabel(unread, labelAt, thinking)}
                  </p>
                {/if}
                {#if streamed}
                  {#each streamed.tools as tool, index (index)}
                    <div class="live-tool live-streaming-tool">
                      <span class="live-tool-icon"><Icon name="tool" size={12} /></span><b>{tool}</b
                      >
                    </div>
                  {/each}
                  {#if streamed.reply}
                    <div
                      class="live-step-reply live-streaming"
                      class:writing={unread.call.state === 'open'}
                    >
                      <StreamingReply
                        text={streamed.reply}
                        writing={unread.call.state === 'open'}
                        cut={streamed.cut}
                      />
                    </div>
                  {/if}
                {/if}
              {/each}
            </div>
            <footer class="live-tile-foot mono">
              {@render lane(tile)}
              <span class="live-foot-facts" title="Provider-reported tokens of the newest call"
                >{tile.context === null ? '' : `${compactOrDash(tile.context)} ctx · `}{num(
                  tile.recording.call_count,
                )} calls</span
              >
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
  .live-page > :global(.empty-state) {
    flex: 1;
    justify-content: center;
  }
  .live-pulse {
    gap: 6px;
  }
  .live-pulse a.chip {
    cursor: pointer;
  }
  .live-pulse :global(.glyph) {
    margin-right: 2px;
  }

  /* Providers and models take a slot's colour; clients keep their own. */
  .series-0 {
    --harness: var(--accent);
  }
  .series-1 {
    --harness: var(--harness-codex);
  }
  .series-2 {
    --harness: var(--role-tool);
  }
  .series-3 {
    --harness: var(--role-thinking);
  }
  .series-other {
    --harness: var(--muted);
  }

  /* The band: output tokens as calls finish, with each series' share beside it. */
  .live-band {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    gap: 1px;
    border-bottom: 1px solid var(--line);
    background: var(--line);
  }
  .live-band-main {
    flex: 999 1 640px;
    min-width: 0;
    padding: 10px 16px 6px;
    background: var(--surface);
  }
  .live-band-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .live-band-head .cap,
  .live-band-side .cap {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .live-band-rate {
    margin-left: 8px;
    font-size: 18px;
    font-weight: 500;
  }
  .live-band-head .meta {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11px;
  }
  .live-band-plot {
    position: relative;
    height: 92px;
    margin-top: 8px;
    background: var(--surface-inset);
  }
  .live-band-side {
    display: flex;
    flex: 1 1 260px;
    flex-direction: column;
    gap: 2px;
    padding: 10px 16px;
    background: var(--surface);
  }
  .live-band-side .cap {
    margin-bottom: 4px;
  }
  .live-legend-row {
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) auto 36px;
    column-gap: 8px;
    align-items: center;
    height: 24px;
    color: var(--ink);
    font-size: 12px;
  }
  .live-legend-row:hover,
  .live-legend-row.chosen {
    background: var(--row-hover);
  }
  .live-legend-row .harness-mark {
    width: 8px;
    height: 8px;
    margin: 0;
    border-radius: 0;
    background: var(--harness);
  }
  .live-legend-row .mono {
    font-size: 11.5px;
  }
  .live-legend-row .share {
    color: var(--muted);
    text-align: right;
  }
  .live-band.empty {
    display: flex;
    align-items: center;
    height: 30px;
    padding: 0 16px;
    color: var(--muted);
    font-size: 11px;
  }
  .live-band-chart {
    display: block;
    width: 100%;
    height: 100%;
  }
  .live-band-unread {
    fill: var(--surface-inset);
    opacity: 0.7;
  }
  .live-band-tick {
    stroke: var(--line);
    stroke-dasharray: 2 3;
    vector-effect: non-scaling-stroke;
  }
  .live-band-rule {
    stroke: var(--line);
    vector-effect: non-scaling-stroke;
  }
  .live-band-area {
    fill: var(--harness);
    opacity: 0.28;
  }
  .live-band-line {
    fill: none;
    stroke: var(--harness);
    stroke-width: 1.4;
    vector-effect: non-scaling-stroke;
  }
  .live-band-chart g {
    transition: opacity 120ms;
  }
  .live-band-chart.hovering g:not(.lit) {
    opacity: 0.25;
  }
  .live-band-mark.warning {
    fill: var(--warning);
  }
  .live-legend-key {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--muted);
    font-size: 11px;
  }
  .live-key-mark {
    display: inline-block;
    width: 3px;
    height: 9px;
    margin-left: 6px;
  }
  .live-key-mark.error {
    background: var(--error);
  }
  .live-key-mark.warning {
    background: var(--warning);
  }
  .live-key-mark.unread {
    width: 9px;
    border: 1px solid var(--line);
    background: var(--surface-inset);
  }
  .live-band-mark.error {
    fill: var(--error);
  }
  div.live-band-axis.mono {
    display: flex;
    justify-content: space-between;
    padding-top: 3px;
    color: var(--muted);
    font-size: 10px;
  }
  .live-only {
    display: inline-flex;
    align-items: center;
    gap: 4px;
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
    grid-template-columns: repeat(3, minmax(0, 1fr));
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
   * Quiet recordings fade to dim once, as they go quiet; hovering brings one
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
  /*
   * The card's state is a tab beside its title and a faint wash behind it,
   * not an edge down the whole card: stacked cards in one state would join
   * into a single bar and read as a column.
   */
  .live-tile-head {
    border-bottom: 1px solid var(--line);
    background: color-mix(in srgb, var(--state, transparent) 9%, var(--surface));
  }
  .live-tile-head::before {
    position: absolute;
    top: 0;
    bottom: -1px;
    left: 0;
    width: 3px;
    background: var(--state, transparent);
    content: '';
    pointer-events: none;
  }
  .live-tile.state-idle .live-tile-head {
    background: var(--surface);
  }
  .live-tile.state-idle .live-tile-head::before {
    background: transparent;
  }
  .live-tile.state-responding .live-tile-head::before {
    animation: state-breathe 1.6s ease-in-out infinite;
  }
  /* Unknown is dashed grey, as everywhere: never an ending, good or bad. */
  .live-tile.state-unknown .live-tile-head::before {
    background: repeating-linear-gradient(to bottom, var(--muted) 0 6px, transparent 6px 10px);
  }
  @media (prefers-reduced-motion: reduce) {
    .live-tile.state-responding .live-tile-head::before {
      animation: none;
    }
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
  /* A tool call: effect glyph, name, what it touched, result; what came back below. */
  .live-tool {
    flex: none;
    display: grid;
    grid-template-columns: 14px auto minmax(0, 1fr) auto;
    column-gap: 6px;
    align-items: baseline;
    margin: 0 0 4px;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11.5px;
    white-space: nowrap;
  }
  .live-tool-icon {
    display: inline-flex;
    align-self: center;
    color: var(--muted);
  }
  .live-tool.effect-write_file .live-tool-icon {
    color: var(--accent);
  }
  .live-tool b {
    color: var(--ink);
    font-weight: 500;
  }
  .live-count {
    color: var(--muted);
    font-weight: 400;
  }
  .live-output {
    grid-column: 3 / 5;
    overflow: hidden;
    color: var(--muted);
    font-size: 11px;
    opacity: 0.8;
    text-overflow: ellipsis;
  }
  .live-output.error {
    color: var(--error);
    opacity: 1;
  }
  /* A call boundary in the expanded card, and calls not read yet. */
  .live-tile-body > .live-sep {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 6px 0 4px;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    white-space: nowrap;
  }
  .live-sep::after {
    flex: 1;
    height: 1px;
    background: var(--line);
    content: '';
  }
  .live-sep.open {
    color: var(--ink);
  }
  .live-io {
    flex: none;
    max-height: 13.5em;
    margin: 0 0 6px 20px;
    padding: 4px 7px;
    overflow: auto;
    background: var(--surface-inset);
    color: var(--muted);
    font: 11px/1.5 var(--mono);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .live-io.reported {
    border-left: 2px solid var(--line);
  }
  .live-io.reported.error {
    border-left-color: var(--error);
  }
  /*
   * A card's actions sit over the right of its header, shown while the card
   * is hovered or holds focus; always shown when expanded, or without hover.
   */
  .live-tile-head {
    position: relative;
  }
  .live-actions {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 6px 0 10px;
    background: inherit;
    opacity: 0;
    pointer-events: none;
  }
  .live-tile:hover .live-actions,
  .live-tile:focus-within .live-actions {
    opacity: 1;
    pointer-events: auto;
  }
  .live-tile.focused .live-actions {
    position: static;
    margin: -3px -6px -3px 0;
    padding: 0;
    opacity: 1;
    pointer-events: auto;
  }
  @media (hover: none) and (pointer: coarse) {
    .live-actions {
      position: static;
      margin: -3px -6px -3px 0;
      padding: 0;
      opacity: 1;
      pointer-events: auto;
    }
  }
  .live-pin.on,
  .live-pinned {
    color: var(--accent);
  }
  .live-pinned {
    display: inline-flex;
    flex: none;
  }
  /* The expanded card covers four slots and reads top down, scrolling. */
  .live-tile.focused {
    grid-column: span 2;
    grid-row: span 2;
  }
  .live-tile-body.full {
    justify-content: flex-start;
    overflow-y: auto;
    -webkit-mask-image: none;
    mask-image: none;
  }
  .live-result {
    display: inline-flex;
    flex: none;
    margin-left: auto;
  }
  .live-tile-body > .live-step-reply {
    flex: none;
    margin: 0 0 6px;
  }
  /* A reply while its call streams, from the live feed: a caret while it is
     still being written; the read reply replaces it once the call is done. */
  .live-streaming {
    color: var(--muted);
  }
  .live-streaming.writing :global(.markdown > :last-child)::after {
    content: '▍';
    margin-left: 1px;
    color: var(--accent);
    animation: live-caret 1s steps(1) infinite;
  }
  /* The model is thinking: its text is not shown, so the status line pulses. */
  .live-sep.thinking {
    color: var(--accent);
    animation: live-thinking 1.6s ease-in-out infinite;
  }
  @keyframes live-thinking {
    50% {
      opacity: 0.45;
    }
  }
  @keyframes live-caret {
    50% {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .live-streaming.writing :global(.markdown > :last-child)::after,
    .live-sep.thinking {
      animation: none;
    }
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
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    column-gap: 10px;
    align-items: center;
    padding: 6px 12px 8px;
    border-top: 1px solid var(--line);
    color: var(--muted);
    font-size: 11px;
  }
  .live-foot-facts {
    white-space: nowrap;
  }
  .live-title {
    overflow: hidden;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .live-state-word {
    flex: none;
    font-size: 11px;
    white-space: nowrap;
  }
  /* A card reads top down: what was asked, then the newest lines, each reply cut short. */
  .live-tile-body:not(.full) {
    justify-content: flex-start;
  }
  .live-tile-body:not(.full) .live-sent {
    flex: none;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .live-tile-body:not(.full) > .live-step-reply {
    display: -webkit-box;
    overflow: hidden;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 4;
    line-clamp: 4;
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

  /* Rows: the same board, one line per recording. */
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
    .live-tile.focused {
      grid-column: auto;
      grid-row: span 2;
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
