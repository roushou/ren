<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { num, short, clock, providerName, providerMark } from '#lib/api.ts';
  import { compactOrDash, span } from '#lib/display.ts';
  import { localDayStart } from '#lib/usage.ts';
  import { ago } from '#lib/overview.ts';
  import { RecordingList, type RecordingRow } from '#lib/recordings.ts';
  import { Board } from '#lib/board.ts';
  import { Sessions } from '#lib/runs.ts';
  import Icon from '#lib/Icon.svelte';
  import Glyph from '#lib/Glyph.svelte';
  import SlideOver from '#lib/layer/SlideOver.svelte';
  import RecordingPreview from '#lib/RecordingPreview.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import SkeletonRows from '#lib/SkeletonRows.svelte';
  import type { Column } from '#lib/columns.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const arrival = createArrival(() => data.recordings);
  const desks = createArrival(() => data.desks);
  /** The sessions Ren drives, by recording; none until they are in. */
  const driven = $derived(desks.snapshot?.value ?? new Map());
  const arrived = $derived(arrival.current);
  /** The listing is on its way: the page draws its outline. */
  const pending = $derived(arrived.state === 'pending');
  const current = $derived(arrived.state === 'ready' ? arrived.snapshot.value : null);
  /** Widths come from the column group, so the folded table lines up with this one. */
  const tableColumns: Column[] = [
    { label: 'Session' },
    { label: 'Now' },
    { label: 'Day' },
    { label: 'Requests', numeric: true },
    { label: 'Turns', numeric: true },
    { label: 'Err', numeric: true },
    { label: 'Tokens in · out', numeric: true },
    { label: 'Active for', numeric: true },
  ];
  const recordings = $derived(current?.recordings ?? []);
  let query = $state('');
  // Relative times tick between refreshes.
  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => (now = Date.now()), 15_000);
    return () => clearInterval(timer);
  });
  const list = $derived(
    new RecordingList({
      recordings,
      details: current?.details ?? new Map(),
      open: current?.open ?? [],
      now,
      query,
      dayStart: localDayStart,
    }),
  );
  const shown = $derived(list.groups.reduce((sum, group) => sum + group.rows.length, 0));
  const recordingHref = (id: string) => `/monitor/sessions/${encodeURIComponent(id)}`;
  /** The recording whose report shows beside the list. */
  let previewing = $state<RecordingRow | null>(null);
  /** A plain click on a row previews it; a modified click, or its link
   * opened elsewhere, goes to its page. */
  function open(event: MouseEvent, row: RecordingRow) {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)
      return;
    if (event.target instanceof Element && event.target.closest('button,summary')) return;
    if (window.getSelection()?.toString()) return;
    event.preventDefault();
    previewing = previewing?.recording.id === row.recording.id ? null : row;
  }
</script>

{#snippet scope()}
  <div class="scope-group">
    <strong
      >{#if pending}<Skeleton width="2em" />{:else}{recordings.length}{recordings.length === 100
          ? '+'
          : ''}{/if}</strong
    >
    <span class="muted">{data.shelf === 'archived' ? 'archived sessions' : 'sessions'}</span>
  </div>
  <div class="scope-group segmented" role="group" aria-label="Which sessions">
    <a class="button small" class:chosen={data.shelf === 'current'} href="/monitor/sessions"
      >Current</a
    >
    <a
      class="button small"
      class:chosen={data.shelf === 'archived'}
      href="/monitor/sessions?shelf=archived">Archived</a
    >
  </div>
  <label class="scope-group end search-input scope-search"
    ><Icon name="search" /><input
      aria-label="Filter sessions"
      placeholder="Search prompt, folder, client, or ID…"
      bind:value={query}
    /></label
  >
{/snippet}

{#snippet bar(share: number)}
  <svg class="size-bar" viewBox="0 0 100 4" preserveAspectRatio="none" aria-hidden="true"
    ><rect x="0" y="0" height="4" width={Math.max(share > 0 ? 1.5 : 0, share * 100)} /></svg
  >
{/snippet}

{#snippet columns()}
  <colgroup
    ><col class="col-recording" /><col class="col-now" /><col class="col-day" /><col
      class="col-requests"
    /><col class="col-turns" /><col class="col-errors" /><col class="col-tokens" /><col
      class="col-active"
    /></colgroup
  >
{/snippet}

{#snippet row(row: RecordingRow)}
  {@const recording = row.recording}
  {@const desk = driven.get(recording.id) ?? null}
  <tr class="call-row recording-row" onclick={(event) => open(event, row)}>
    <!-- The edge sits on the cell: on a row, its ::before would become a cell of its own. -->
    <td class="recording-cell state-edge {row.now.kind === 'open' ? `state-${row.now.mark}` : ''}">
      <a class="row-link recording-link" href={recordingHref(recording.id)}>
        <span class="recording-title">
          <span class="harness-mark harness-{row.harness ?? 'unknown'}"></span>
          <strong>{row.client}</strong>
          {#if row.detail?.prompt}<span class="recording-prompt">{row.detail.prompt.text}</span
            >{:else}<span class="recording-prompt muted">no prompt yet</span>{/if}
        </span>
        <span class="recording-meta mono">
          {#if row.detail?.workdir}<span class="recording-folder" title={row.detail.workdir}
              >{Board.folder(row.detail.workdir)}</span
            >{/if}
          <span title="The session ID your client reported">{short(recording.native_id)}</span>
          <span
            class="provider-mark tiny {recording.provider}"
            title={providerName(recording.provider)}>{providerMark(recording.provider)}</span
          >
        </span>
      </a>
      {#if desk}<a
          class="desk-link mono"
          href={Sessions.href(desk)}
          title="Ren drives this session: open it on its desk">desk ↗</a
        >{/if}
    </td>
    <td class="recording-now mono">
      {#if row.now.kind === 'open'}<span class="now-open state-{row.now.mark}"
          ><Glyph mark={row.now.mark} label={row.now.label} /></span
        >{:else if row.now.kind === 'recent'}<span class="now-recent"
          ><span class="recent-dot"></span>active {ago(row.now.at, now)}</span
        >{:else}<span class="muted" title={clock(row.now.at)}>{clock(row.now.at)}</span>{/if}
    </td>
    <td class="day-cell"
      ><svg class="day-bar" viewBox="0 0 100 6" preserveAspectRatio="none" aria-hidden="true"
        ><rect class="day-whole" width="100" height="6" />{#each [25, 50, 75] as tick (tick)}<rect
            class="day-tick"
            x={tick}
            width="0.4"
            height="6"
          />{/each}<rect
          class="day-span state-{row.now.kind === 'open' ? row.now.mark : row.now.kind}"
          x={row.day.from * 100}
          width={Math.max(0.8, (row.day.to - row.day.from) * 100)}
          height="6"
        /></svg
      ></td
    >
    <td class="numeric mono sized">{num(recording.call_count)}{@render bar(row.requests)}</td>
    <td class="numeric mono">{num(recording.turn_count)}</td>
    <td class="numeric mono"
      >{#if recording.error_count}<span class="errors"
          ><Glyph mark="failed" label="{recording.error_count} failed" />{num(
            recording.error_count,
          )}</span
        >{:else}<span class="muted">—</span>{/if}</td
    >
    <td
      class="numeric mono sized"
      title="Provider reported; input includes cached context. Not subscription allowance or charges."
      >{compactOrDash(recording.tokens.input)}
      <span class="muted">· {compactOrDash(recording.tokens.output)}</span>{@render bar(
        row.tokens,
      )}</td
    >
    <td class="numeric mono muted">{span(recording.last_seen - recording.first_seen)}</td>
  </tr>
{/snippet}

<Toolbar {scope} />
{#if !pending && list.summary.recordings}
  {@const summary = list.summary}
  <section class="summary" aria-label="The listed sessions at a glance">
    <div class="tile">
      <span class="cap">Sessions</span><span class="mono big">{num(summary.recordings)}</span>
    </div>
    <div class="tile">
      <span class="cap">Open now</span><span class="mono big"
        >{#if summary.open}<Glyph mark="responding" label={true} />{/if}{num(summary.open)}</span
      >
    </div>
    <div class="tile">
      <span class="cap">Requests</span><span class="mono big">{num(summary.requests)}</span>
    </div>
    <div class="tile">
      <span class="cap">Errors</span><span class="mono big"
        >{#if summary.errors}<Glyph mark="failed" label={true} />{/if}{num(summary.errors)}</span
      >
    </div>
    <div class="tile">
      <span class="cap">Tokens in</span><span class="mono big"
        >{compactOrDash(summary.tokensIn)}</span
      >
    </div>
    <div class="clients">
      <span class="cap">Clients · share of requests</span>
      <svg class="client-bar" viewBox="0 0 100 8" preserveAspectRatio="none" aria-hidden="true"
        >{#each summary.clients as client, index (client.client)}<rect
            class="client-{index % 4}"
            x={summary.clients.slice(0, index).reduce((sum, one) => sum + one.share, 0) * 100}
            width={client.share * 100}
            height="8"
          />{/each}</svg
      >
      <span class="client-legend"
        >{#each summary.clients.slice(0, 4) as client, index (client.client)}<span
            ><span class="swatch client-{index % 4}"></span>{client.client}
            {Math.round(client.share * 100)}%</span
          >{/each}</span
      >
    </div>
  </section>
{/if}
<section class="panel list-panel recordings-panel">
  <div class="list-content">
    {#if pending || shown || list.folded.length}
      <div class="table-scroll">
        <table class="recordings-table" aria-busy={pending}>
          {@render columns()}
          <thead
            ><tr
              >{#each tableColumns as column (column.label)}<th class:numeric={column.numeric}
                  >{column.label}</th
                >{/each}</tr
            ></thead
          >
          {#if pending}<tbody><SkeletonRows columns={tableColumns} rows={10} /></tbody>{/if}
          {#each list.groups as group (group.key)}
            <tbody>
              <tr class="day-row"
                ><th colspan="8">{group.label}<span class="muted"> · {group.rows.length}</span></th
                ></tr
              >
              {#each group.rows as each (each.recording.id)}{@render row(each)}{/each}
            </tbody>
          {/each}
        </table>
      </div>
      {#if list.folded.length}
        <details class="recordings-folded">
          <summary
            >{list.folded.length} single-call session{list.folded.length === 1 ? '' : 's'} with no turns</summary
          >
          <table class="recordings-table">
            {@render columns()}
            <tbody>
              {#each list.folded as each (each.recording.id)}{@render row(each)}{/each}
            </tbody>
          </table>
        </details>
      {/if}
    {:else if data.shelf === 'archived' && !query}<div class="empty-state">
        <div class="empty-icon"><Icon name="archive" size={25} /></div>
        <h2>Nothing archived</h2>
        <p>
          Archive a recording from its page, or a run from Runs. A recording with new traffic is
          listed with the current ones again.
        </p>
        <a class="button" href="/monitor/sessions"
          >Current sessions <Icon name="arrow" size={14} /></a
        >
      </div>
    {:else}<div class="empty-state">
        <div class="empty-icon"><Icon name="sessions" size={25} /></div>
        <h2>{query ? 'No matching sessions' : 'No sessions yet'}</h2>
        <p>
          {query
            ? 'Try a different prompt, folder, client, or session ID.'
            : 'Sessions appear when clients send session identifiers through Ren.'}
        </p>
        {#if query}<button class="button" onclick={() => (query = '')}>Clear search</button
          >{:else}<button class="button" onclick={() => goto('/monitor/traffic')}
            >Go to request logs <Icon name="arrow" size={14} /></button
          >{/if}
      </div>{/if}
  </div>
  {#if previewing}
    <SlideOver label="Session report" pin="report" onclose={() => (previewing = null)}>
      <RecordingPreview row={previewing} desk={driven.get(previewing.recording.id) ?? null} />
    </SlideOver>
  {/if}
</section>

<style>
  .muted {
    color: var(--muted);
  }
  /* Fixed columns, so the folded table lines up with the one above it. */
  .recordings-table {
    width: 100%;
    table-layout: fixed;
  }
  .col-recording {
    width: auto;
  }
  .col-now {
    width: 190px;
  }
  .col-requests {
    width: 110px;
  }
  .col-turns {
    width: 70px;
  }
  .col-tokens {
    width: 150px;
  }
  .col-active {
    width: 110px;
  }
  .recordings-table td {
    vertical-align: middle;
  }
  /* A row: who and what, then the state now, then how big it got. */
  .recording-cell {
    position: relative;
    overflow: hidden;
  }
  .desk-link {
    position: absolute;
    right: 8px;
    bottom: 7px;
    padding: 0 5px;
    border: 1px solid var(--control-border);
    color: var(--muted);
    font-size: 10.5px;
  }
  .desk-link:hover {
    color: var(--ink);
  }
  .recording-link {
    display: flex;
    flex-direction: column;
    /* The global row link centres its items; in a column that would centre them across. */
    align-items: stretch;
    gap: 3px;
    min-width: 0;
    padding-left: 4px;
  }
  .recording-title {
    display: flex;
    align-items: center;
    gap: 0;
    min-width: 0;
  }
  .recording-title strong {
    flex: none;
    margin-right: 10px;
    font-weight: 600;
  }
  .recording-prompt {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .recording-meta {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-left: 15px;
    color: var(--muted);
    font-size: 11px;
  }
  .recording-folder {
    color: var(--ink);
  }
  .errors {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .col-day {
    width: 120px;
  }
  .col-errors {
    width: 56px;
  }
  .day-cell {
    padding-inline: 8px;
  }
  .day-bar {
    display: block;
    width: 100%;
    height: 6px;
  }
  .day-whole {
    fill: var(--surface-inset);
  }
  .day-tick {
    fill: var(--line);
  }
  .day-span {
    fill: color-mix(in srgb, var(--chart) 70%, transparent);
  }
  .day-span.state-responding,
  .day-span.state-recent {
    fill: var(--live);
  }
  .recordings-panel {
    position: relative;
  }
  .summary {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr)) minmax(240px, 2fr);
    border-bottom: 1px solid var(--line);
    background: var(--surface);
    font-size: 11.5px;
  }
  .tile,
  .clients {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 14px;
    border-right: 1px solid var(--line);
  }
  .clients {
    border-right: 0;
  }
  .cap {
    color: var(--muted);
    font-size: 10.5px;
    font-weight: 500;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .big {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 17px;
  }
  .client-bar {
    width: 100%;
    height: 8px;
    background: var(--surface-inset);
  }
  .client-legend {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 12px;
    color: var(--muted);
  }
  .client-legend > span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .swatch {
    width: 8px;
    height: 8px;
  }
  .client-0 {
    fill: var(--accent);
    background: var(--accent);
  }
  .client-1 {
    fill: var(--role-tool);
    background: var(--role-tool);
  }
  .client-2 {
    fill: var(--success);
    background: var(--success);
  }
  .client-3 {
    fill: var(--muted);
    background: var(--muted);
  }
  /* Narrower, the day bar and the time active go first, so the recording
     keeps its width; the summary takes two rows, then three. */
  @media (max-width: 1200px) {
    .recordings-table .col-day,
    .recordings-table .day-cell,
    .recordings-table .col-active,
    .recordings-table td:last-child,
    .recordings-table th:last-child {
      display: none;
    }
  }
  @media (max-width: 960px) {
    .summary {
      grid-template-columns: repeat(5, minmax(0, 1fr));
    }
    .clients {
      grid-column: 1 / -1;
      border-top: 1px solid var(--line);
    }
  }
  .recording-now {
    font-size: 11.5px;
    white-space: nowrap;
  }
  .now-open {
    color: var(--ink);
  }
  .now-open.state-yours {
    color: var(--success);
  }
  .now-recent {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--ink);
  }
  .recent-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--success);
  }
  /* Sizes as thin bars under the numbers, against the largest in the list. */
  .sized {
    position: relative;
  }
  .size-bar {
    display: block;
    width: 100%;
    max-width: 120px;
    height: 4px;
    margin: 4px 0 0 auto;
  }
  .size-bar rect {
    fill: color-mix(in srgb, var(--chart) 70%, transparent);
  }
  .day-row th {
    padding: 6px 12px;
    border-bottom: 1px solid var(--line);
    background: var(--surface-subtle);
    color: var(--ink);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-align: left;
    text-transform: uppercase;
  }
  .day-row .muted {
    font-weight: 400;
  }
  .recordings-folded {
    border-top: 1px solid var(--line);
  }
  .recordings-folded summary {
    padding: 10px 12px;
    color: var(--ink);
    font-size: 12px;
    cursor: pointer;
  }
  .recordings-folded summary:hover {
    background: var(--row-hover);
  }
  .recordings-folded[open] summary {
    border-bottom: 1px solid var(--line);
  }
  @media (max-width: 760px) {
    .recordings-table th:nth-child(n + 3),
    .recordings-table td:nth-child(n + 3),
    .recordings-table col:nth-child(n + 3) {
      display: none;
    }
    /* Narrow: two columns that share the width, the recording taking what is left. */
    .recordings-table {
      table-layout: auto;
    }
    .recording-cell {
      width: 100%;
      max-width: 0;
    }
    .recording-now {
      white-space: normal;
    }
    .day-row th {
      display: table-cell !important;
    }
  }
  /* Last, so it wins over the 960px rule above on a phone. */
  @media (max-width: 560px) {
    .summary {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
</style>
