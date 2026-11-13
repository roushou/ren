<script lang="ts">
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { prefersReducedMotion } from 'svelte/motion';
  import { page } from '$app/state';
  import { RecentCalls } from '#lib/loaders.ts';
  import { goto } from '$app/navigation';
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { clock, providerName, short } from '#lib/api.ts';
  import { compactOrDash, span } from '#lib/display.ts';
  import { unrecognised, type Stored } from '#lib/decode.ts';
  import type { EffectKind, Excerpt } from '#lib/api.ts';
  import {
    Band,
    Board,
    LivePlace,
    type BandBy,
    type BandSeries,
    type BandStyle,
    type Density,
    Feed,
    type FeedItem,
    type LiveWindow,
    type Tile,
    type ResultMark,
    type ToolLine,
    type TurnCall,
    type TurnTally,
    type UnreadCall,
  } from '#lib/board.ts';
  import Icon, { type IconName } from '#lib/Icon.svelte';
  import StateGlyph from '#lib/StateGlyph.svelte';
  import Select, { type SelectOption } from '#lib/Select.svelte';
  import Markdown from '#lib/Markdown.svelte';
  import StreamingReply from '#lib/StreamingReply.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const workbench = getWorkbench();
  workbench.showScope(scope);
  const snapshot = createSnapshot(() => data.live);
  const current = $derived(snapshot.current.value);

  // Elapsed times tick between refreshes.
  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  const view = $derived(LivePlace.parse(page.url.searchParams));
  const band = $derived(
    current
      ? new Band(
          current.sessions,
          current.recent,
          Math.max(now, current.read),
          view.by,
          current.complete,
        )
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
          sessions: current.sessions,
          calls: current.calls,
          turns: current.turns,
          now: Math.max(now, current.read),
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
  const byOptions: SelectOption<BandBy>[] = [
    { value: 'client', label: 'By client' },
    { value: 'provider', label: 'By provider' },
    { value: 'model', label: 'By model' },
  ];
  const bandOptions: SelectOption<BandStyle>[] = [
    { value: 'ribbon', label: 'Ribbon' },
    { value: 'area', label: 'Area' },
  ];
  const densityOptions: SelectOption<Density>[] = [
    { value: 'comfortable', label: 'Cards' },
    { value: 'compact', label: 'Rows' },
  ];
  /** Whether a card belongs to the hovered band series. */
  function inHoveredSeries(tile: Tile): boolean {
    if (!hoveredSeries || !band) return false;
    const key = Band.sessionKey(view.by, tile.session, tile.model);
    return band.seriesOf(key)?.key === hoveredSeries;
  }
  /** The band's height in SVG units, drawn at the same height in pixels. */
  const BAND_HEIGHT = 120;
  const rate = (value: number) => (value < 10 ? value.toFixed(1) : String(Math.round(value)));
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
  const emptySlots = $derived(board ? Math.max(0, board.capacity - board.tiles.length) : 0);
  const pinHref = (tile: Tile) => LivePlace.href(LivePlace.togglePin(view, tile.session.id));
  const turnHref = (turn: string) => `/turns/${encodeURIComponent(turn)}`;
  /** The call a card is about: the open or last one its state names, else the newest. */
  function callHref(tile: Tile): string | null {
    const call = tile.phase.kind === 'idle' ? tile.latest : tile.phase.call;
    return call ? `${sessionHref(tile.session.id)}?call=${encodeURIComponent(call.id)}` : null;
  }
  const focusHref = (tile: Tile) =>
    LivePlace.href({ ...view, focus: board?.focus === tile.session.id ? null : tile.session.id });
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
  function effectIcon(effect: Stored<EffectKind> | null): IconName {
    switch (effect) {
      case null:
        return 'tool';
      case 'read_file':
        return 'file';
      case 'write_file':
        return 'pencil';
      case 'run_command':
        return 'terminal';
      case 'search':
        return 'search';
      case 'delegate':
        return 'branch';
      case 'fetch':
        return 'globe';
      default:
        unrecognised(effect);
        return 'tool';
    }
  }
  function tallyParts(tally: TurnTally): string[] {
    return [`read ${tally.read}`, `wrote ${tally.wrote}`, `ran ${tally.ran}`];
  }
  /** A call not read yet: open (waiting for its head, or responding), or just finished. */
  function unreadLabel(unread: UnreadCall, now: number, thinking: boolean): string {
    const { call } = unread;
    if (call.state !== 'open') return 'finished · reading';
    const state =
      call.status === null ? 'waiting for response' : thinking ? 'thinking' : 'responding';
    return `${state} · ${span(now - call.started_at)}`;
  }
  function callLabel(call: TurnCall): string {
    const parts = [`call ${call.index ?? '—'}`];
    if (call.durationMs !== null) parts.push(span(call.durationMs));
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

  const sessionHref = (id: string) => `/recordings/${encodeURIComponent(id)}`;
  const percent = (share: number | null) => (share === null ? '—' : `${Math.round(share * 100)}%`);
  const harnessClass = (tile: Tile) => `harness-${tile.harness ?? 'unknown'}`;
  /** The tile's facts line: model, cached share, output. The context is in the header. */
  function facts(tile: Tile): string {
    const parts = [tile.model ?? 'model —'];
    if (tile.cached !== null) parts.push(`${percent(tile.cached)} cached`);
    if (tile.output !== null) parts.push(`out ${compactOrDash(tile.output)}`);
    return parts.join(' · ');
  }
  /** The newest thing the session did, for a row. */
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
    <span class="live-tool-icon"><Icon name={effectIcon(tool.effect)} size={12} /></span>
    <b
      >{tool.name}{#if count > 1}<span class="live-count"> ×{count}</span>{/if}</b
    >
    {#if tool.target}<span class="live-target" title={tool.target}>{tool.target}</span>{/if}
    <span class="live-result {result.tone}">{result.label}</span>
    {#if tool.output}<span class="live-output" class:error={tool.result.tone === 'error'}
        >⎿ {tool.output}</span
      >{/if}
  </div>
{/snippet}

{#snippet scope()}
  {#if board}
    <div class="scope-group live-pulse" aria-label="Right now">
      <span class="live-active">
        <span class="chip">{board.tiles.length + board.overflow} active</span><Select
          label="How far back a recording counts as active"
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
          href="/recordings"
          title="Active recordings without a tile: the board keeps the busiest and most recent {Board.maxTiles}"
          >{board.overflow} not shown</a
        >{/if}
      {#if only}<a
          class="chip live-only"
          href={LivePlace.href({ ...view, series: null })}
          title="The wall shows only recordings in this series; select to show every recording"
          >only {only.label} <Icon name="close" size={11} /></a
        >{/if}
      {#if current && (current.truncated.calls || current.truncated.sessions)}<span
          class="chip warning"
          title="The window holds more than the page reads at once ({RecentCalls.pageLimit *
            RecentCalls.maxPages} calls, {LivePlace.windows[current.window]
            .sessions} sessions): the oldest are left out. Choose a shorter window to see all of it."
          >partial: newest only</span
        >{/if}
    </div>
  {/if}
  <div class="scope-group end">
    <Select
      label="What the band groups output by"
      size="compact"
      options={byOptions}
      value={view.by}
      onchange={(by) => goto(LivePlace.href({ ...view, by, series: null }))}
    />
    <Select
      label="Band style"
      size="compact"
      options={bandOptions}
      value={view.band}
      onchange={(band) => goto(LivePlace.href({ ...view, band }))}
    />
    <Select
      label="Board density"
      size="compact"
      options={densityOptions}
      value={view.density}
      onchange={(density) => goto(LivePlace.href({ ...view, density }))}
    />
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
    >{Board.clientName(tile.harness)} → {providerName(tile.session.provider)}</strong
  >
  <span class="mono muted" title="The session ID your client reported"
    >{short(tile.session.native_id)}</span
  >
{/snippet}

<svelte:window {onkeydown} />

<div class="live-page">
  {#if !current || !board}
    <div class="empty-state" role="alert">
      <div class="empty-icon"><Icon name="alert" size={26} /></div>
      <h2>Live activity could not be read</h2>
      <p>{snapshot.current.error?.message ?? 'Loading…'}</p>
    </div>
  {:else}
    {#if band && band.series.length}
      <section
        class="live-band"
        aria-label="Output tokens by {view.by}"
        title="Output tokens per {view.by} over the last {span(
          Band.spanMs,
        )}, counted when each call finishes. Marks along the top: calls that failed (red) or were cut off (yellow), when they ended."
      >
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
        <div class="live-legend" class:rows={view.band === 'area'}>
          {#each band.series as series (series.key)}
            <a
              class="live-legend-chip {series.swatch}"
              class:chosen={view.series === series.key}
              href={seriesHref(series.key)}
              title={series.members.length > 1
                ? `${series.members.map((key) => Band.label(view.by, key)).join(', ')}: select to show only their recordings`
                : view.series === series.key
                  ? 'Show every recording'
                  : `Show only ${series.label} recordings`}
              onpointerenter={() => (hoveredSeries = series.key)}
              onpointerleave={() => (hoveredSeries = null)}
              onfocus={() => (hoveredSeries = series.key)}
              onblur={() => (hoveredSeries = null)}
              ><span class="harness-mark"></span><b>{series.label}</b><span class="mono"
                >{rate(series.rate)} tok/s</span
              ></a
            >
          {/each}
          {#if band.series.some((series) => series.marks.length)}
            <span class="live-legend-key" aria-hidden="true"
              ><span class="live-key-mark error"></span>failed<span class="live-key-mark warning"
              ></span>cut off</span
            >
          {/if}
          {#if band.unread > 0}
            <span
              class="live-legend-key"
              title="More calls than the page reads at once: the shaded start of the band was not read, so it is unknown, not quiet."
              ><span class="live-key-mark unread"></span>not read</span
            >
          {/if}
        </div>
        <span class="live-band-scale mono" aria-hidden="true"
          >{view.band === 'ribbon' ? band.ribbonMax : band.max} tok/s</span
        >
        <div class="live-band-axis mono" aria-hidden="true">
          <span>−10m</span><span>−8m</span><span>−6m</span><span>−4m</span><span>−2m</span><span
            >now</span
          >
        </div>
      </section>
    {:else if band}
      <section class="live-band empty" aria-label="Output tokens">
        <span class="mono">No output in the last {span(Band.spanMs)}</span>
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
        <h2>No recording active in the {LivePlace.windows[view.window].label}</h2>
        <p>Recordings appear here as soon as a client sends a call through Ren.</p>
        {#if longer}<a class="button" href={LivePlace.href({ ...view, window: longer })}
            >Look back {LivePlace.windows[longer].label.replace('last ', '')}</a
          >{/if}
      </div>
    {:else if view.density === 'compact'}
      <div class="live-rows-wrap">
        <table class="live-rows">
          <thead>
            <tr>
              <th>Recording</th>
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
                    ><StateGlyph mark={rowChip.mark} />{rowChip.label}</span
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
          {@const focused = board.focus === tile.session.id}
          {@const toCall = callHref(tile)}
          <article
            class="live-tile state-{chip.mark} {harnessClass(tile)} {attention
              ? `attention-${attention}`
              : ''}"
            class:quiet={tile.quiet && !focused}
            class:focused
            class:highlighted={highlighted === tile.session.id || inHoveredSeries(tile)}
          >
            <header class="live-tile-head">
              <a class="live-open" href={sessionHref(tile.session.id)} title="Open the session"
                >{@render identity(tile)}<span class="live-open-hint"
                  ><Icon name="arrow" size={11} /></span
                ></a
              >
              {#if tile.turnCalls}<span
                  class="mono muted live-seq"
                  title="Model calls the current turn has made since its prompt"
                  >· call {tile.turnCalls}</span
                >{/if}
              {#if tile.open > 1}<span
                  class="mono live-open-count"
                  title="Model calls in flight at once: a subagent or a side request beside the main one"
                  ><span class="muted">·</span> {tile.open} in flight</span
                >{/if}
              {#if tile.pinned}<span
                  class="live-pinned"
                  title="Pinned: stays on the wall while the recording is active"
                  ><Icon name="pin" size={12} /></span
                >{/if}
              <span
                class="mono muted end"
                title="Context the newest call carried; provider-reported"
                >ctx {compactOrDash(tile.context)}</span
              >
              <div class="live-actions">
                {#if tile.turnId}<a
                    class="icon-button"
                    href={turnHref(tile.turnId)}
                    aria-label="Open the turn"
                    title="Open the turn: its prompt, tools, files, and findings"
                    ><Icon name="logs" size={13} /></a
                  >{/if}
                {#if toCall}<a
                    class="icon-button"
                    href={toCall}
                    aria-label="Open the call"
                    title="Open this call in its recording, with the inspector"
                    ><Icon name="pulse" size={13} /></a
                  >{/if}
                <a
                  class="icon-button live-pin"
                  class:on={tile.pinned}
                  href={pinHref(tile)}
                  aria-label={tile.pinned ? 'Unpin this card' : 'Pin this card'}
                  title={tile.pinned
                    ? 'Unpin: busier recordings may take its place'
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
                    call {unread.index} · {unreadLabel(unread, board.now, thinking)}
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
              {#if !tile.prompt && !tile.lines.length && !tile.unread.some((unread) => unread.call.state === 'open' || workbench.streamed.get(unread.call.id)?.reply)}
                <p class="live-wait">
                  {tile.turnId
                    ? 'Text appears once a call finishes and is read.'
                    : 'This client reports no turns, so only a reply as it streams shows here.'}
                </p>
              {/if}
            </div>
            {#if tile.tally}
              <div class="live-tally mono" title="What the turn did, as the client reported it">
                {#if tile.workdir}<span class="live-workdir" title={tile.workdir}
                    >{Board.folder(tile.workdir)}</span
                  >{/if}
                {#each tallyParts(tile.tally) as part (part)}<span>{part}</span>{/each}
                {#if tile.tally.failed}<span class="error">{tile.tally.failed} failed</span>{/if}
              </div>
            {/if}
            <footer class="live-tile-foot mono">
              <div class="live-foot-state">
                <span class="live-state {chip.tone} state-{chip.mark}" title={chip.title}
                  ><StateGlyph mark={chip.mark} />{chip.label}</span
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

  .live-page {
    --mono: 'SFMono-Regular', Consolas, 'Liberation Mono', monospace;
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

  /*
   * The band: output tokens, as calls finish, in one chart of fixed height
   * whatever the number of series; its legend sits inside it, top left.
   */
  .live-band {
    position: relative;
    flex: none;
    height: 120px;
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
  .live-legend {
    position: absolute;
    top: 6px;
    left: 8px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    max-height: calc(100% - 22px);
  }
  /* In Area, each chip sits at the top of its own lane. */
  .live-legend.rows {
    top: 0;
    bottom: 0;
    display: grid;
    grid-auto-rows: minmax(0, 1fr);
    gap: 0;
    max-height: none;
  }
  .live-legend.rows .live-legend-chip {
    align-self: start;
    justify-self: start;
    margin-top: 3px;
  }
  .live-legend-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 1px 6px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--surface) 82%, transparent);
    color: var(--ink);
    font-size: 11.5px;
    line-height: 18px;
    text-decoration: none;
    white-space: nowrap;
  }
  .live-legend-chip:hover,
  .live-legend-chip:focus-visible {
    background: var(--row-hover);
  }
  .live-legend-chip.chosen {
    box-shadow: inset 0 0 0 1px var(--harness);
  }
  .live-legend-chip .harness-mark {
    margin-right: 0;
  }
  .live-legend-chip b {
    font-weight: 600;
  }
  .live-legend-chip .mono {
    color: var(--muted);
    font-size: 10.5px;
  }
  /* Below the error ticks, which mark the top edge. */
  span.live-band-scale.mono {
    position: absolute;
    top: 13px;
    right: 6px;
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
  .live-state.state-yours {
    color: var(--success);
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
  .live-tally {
    display: flex;
    gap: 10px;
    padding: 4px 12px;
    overflow: hidden;
    border-top: 1px solid var(--line);
    color: var(--muted);
    font-size: 11px;
    white-space: nowrap;
  }
  .live-workdir {
    overflow: hidden;
    color: var(--ink);
    text-overflow: ellipsis;
  }
  .live-tally .error {
    color: var(--error);
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
  .live-open-hint {
    display: inline-flex;
    flex: none;
    color: var(--muted);
    opacity: 0;
  }
  .live-open:hover .live-open-hint,
  .live-open:focus-visible .live-open-hint {
    opacity: 1;
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
