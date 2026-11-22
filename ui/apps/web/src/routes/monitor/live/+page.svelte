<script lang="ts">
  import { Now } from '#lib/now.svelte.ts';
  import Toolbar from '#lib/Toolbar.svelte';
  import { page } from '$app/state';
  import { RecentCalls } from '#lib/loaders.ts';
  import { goto } from '$app/navigation';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import type { RecordingId } from '#lib/api.ts';
  import {
    Band,
    Board,
    LivePlace,
    type BandBy,
    type Density,
    type LiveWindow,
  } from '#lib/board.ts';
  import Icon from '#lib/Icon.svelte';
  import Glyph from '#lib/Glyph.svelte';
  import Segmented, { type SegmentedOption } from '#lib/Segmented.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import LiveBand from '#lib/live/LiveBand.svelte';
  import LiveOutline from '#lib/live/LiveOutline.svelte';
  import LiveRows from '#lib/live/LiveRows.svelte';
  import LiveTile from '#lib/live/LiveTile.svelte';
  import LiveWall from '#lib/live/LiveWall.svelte';
  import NowStrip from '#lib/live/NowStrip.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const arrival = createArrival(() => data.live);
  const shown = $derived(arrival.current);
  /** The window's reads are on their way: the page draws its outline. */
  const pending = $derived(shown.state === 'pending');
  const snapshot = $derived(shown.state === 'ready' ? shown.snapshot : null);
  const current = $derived(snapshot?.value ?? null);

  // Elapsed times tick between refreshes.
  const clock = new Now(1000);
  const now = $derived(clock.current);

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
  const only = $derived(band && view.series ? band.only(view.series) : null);
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
  /** The recording whose line in the Now strip is under the pointer. */
  let highlighted = $state<RecordingId | null>(null);
  const byOptions: SegmentedOption<BandBy>[] = [
    { value: 'client', label: 'By client' },
    { value: 'provider', label: 'By provider' },
    { value: 'model', label: 'By model' },
  ];
  const densityOptions: SegmentedOption<Density>[] = [
    { value: 'comfortable', label: 'Cards' },
    { value: 'compact', label: 'Rows' },
  ];
  const windowOptions = (Object.keys(LivePlace.windows) as LiveWindow[]).map((value) => ({
    value,
    label: LivePlace.shortLabel(value),
  }));
  const longer = $derived(LivePlace.longer(view.window));
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
</script>

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

<svelte:window {onkeydown} />

<Toolbar {scope} />
<div class="live-page">
  {#if pending}
    <LiveOutline by={view.by} density={view.density} />
  {:else if !current || !board}
    <div class="empty-state" role="alert">
      <div class="empty-icon"><Icon name="alert" size={26} /></div>
      <h2>Live activity could not be read</h2>
      <p>{snapshot?.error?.message ?? 'Loading…'}</p>
    </div>
  {:else}
    {#if band}
      <LiveBand
        kind="band"
        {band}
        {view}
        hovered={hoveredSeries}
        onhover={(key) => (hoveredSeries = key)}
      />
    {/if}
    <NowStrip
      kind="feed"
      feed={board.feed}
      now={board.now}
      {labelAt}
      onhighlight={(recording) => (highlighted = recording)}
    />
    {#if !board.tiles.length}
      <div class="empty-state">
        <div class="empty-icon"><Icon name="pulse" size={26} /></div>
        <h2>No session active in the {LivePlace.windows[view.window].label}</h2>
        {#if longer}<a class="button" href={LivePlace.href({ ...view, window: longer })}
            >Look back {LivePlace.shortLabel(longer)}</a
          >{/if}
      </div>
    {:else if view.density === 'compact'}
      <LiveRows kind="rows" {board} />
    {:else}
      <LiveWall empty={board.emptySlots}>
        {#each board.tiles as tile (tile.recording.id)}
          <LiveTile
            kind="tile"
            {tile}
            {board}
            {view}
            highlighted={highlighted === tile.recording.id ||
              (!!hoveredSeries && band?.seriesOfTile(tile)?.key === hoveredSeries)}
            {labelAt}
          />
        {/each}
      </LiveWall>
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
  .live-page > :global(.empty-state) {
    flex: 1;
    justify-content: center;
    background: var(--surface);
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
  .live-only {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
</style>
