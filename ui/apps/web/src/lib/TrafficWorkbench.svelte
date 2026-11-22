<script lang="ts">
  import { Clock } from './time.ts';
  import type { GatewayData } from '#lib/loaders.ts';
  import SlideOver from '#lib/layer/SlideOver.svelte';
  import Toolbar from './Toolbar.svelte';
  import { invalidateAll } from '$app/navigation';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { getWorkbench } from './workbench.svelte.ts';
  import { createArrival } from './snapshot.svelte.ts';
  import { Arrival, type Arriving } from './arrival.ts';
  import type { Column } from './columns.ts';
  import SkeletonRows from './SkeletonRows.svelte';
  import type { TrafficData } from './loaders.ts';
  import { num, short, type RecordingId } from './api.ts';
  import { duration, date } from './display.ts';
  import { type Filters, sinceChoices, turnGroups } from './traffic.ts';
  import { LiveFollow } from './live.ts';
  import Icon from './Icon.svelte';
  import SessionHead from './SessionHead.svelte';
  import TurnPanel from './TurnPanel.svelte';
  import { ManagementApi, TurnId, type TurnMetrics } from './api.ts';
  import Inspector from './Inspector.svelte';
  import CallOverview from './CallOverview.svelte';
  import ContextChart from './ContextChart.svelte';
  import Segmented, { type SegmentedOption } from './Segmented.svelte';
  import ConnectClient from './ConnectClient.svelte';
  import TrafficFilters from './traffic/TrafficFilters.svelte';
  import CallRow from './traffic/CallRow.svelte';
  import TurnRow from './traffic/TurnRow.svelte';
  import { CallSelection } from './traffic/selection.svelte.ts';
  let {
    snapshot,
    recordingId,
    desk = null,
  }: {
    snapshot: Arriving<TrafficData>;
    recordingId: RecordingId | null;
    /** Where the session opens on its desk, when Ren drives it. */
    desk?: string | null;
  } = $props();
  const workbench = getWorkbench();
  /** A session's turns: each one's prompt, and the report's figures for it. */
  let turnInfo = $state.raw<{
    recording: RecordingId;
    prompts: Map<TurnId, string>;
    metrics: Map<TurnId, TurnMetrics>;
  } | null>(null);
  $effect(() => {
    // Read again as the page refreshes; a read that lands after another
    // session opened is dropped. Not aborted: live refreshes come faster
    // than a slow read finishes.
    void workbench.refreshed;
    const target = recordingId;
    if (!target) return;
    const api = new ManagementApi();
    void Promise.all([
      api.turns(target, 100).catch(() => null),
      api.report(target).catch(() => null),
    ]).then(([listed, report]) => {
      if (recordingId !== target) return;
      turnInfo = {
        recording: target,
        prompts: new Map(
          (listed?.turns ?? []).flatMap((turn) =>
            turn.prompt ? [[turn.turn, turn.prompt.text.trim().split('\n')[0] ?? '']] : [],
          ),
        ),
        metrics: new Map((report?.turns ?? []).map((turn) => [turn.turn, turn])),
      };
    });
  });
  const turns = $derived(turnInfo && turnInfo.recording === recordingId ? turnInfo : null);
  /** The turn open beside the calls, as the address names it (`?turn=`). */
  const turnOpen = $derived(
    TurnId.parse((page.shallow?.url ?? page.url).searchParams.get('turn') ?? ''),
  );
  /** Open `turn` beside the calls, or close it: the address follows, so Back
   * and a reload show the same, and nothing is read again. */
  function openTurn(turn: TurnId | null) {
    const url = new URL((page.shallow?.url ?? page.url).href);
    if (turn) url.searchParams.set('turn', turn);
    else url.searchParams.delete('turn');
    void goto(url, { shallow: true, replaceState: true });
  }
  // The scope bar shows the narrowing controls, or the recording itself.
  const arrival = createArrival(() => snapshot);
  const shown = $derived(arrival.current);
  /** The listing is on its way: the table draws its own outline. */
  const outlined = $derived(shown.state === 'pending');
  const retained = $derived(shown.state === 'ready' ? shown.snapshot : null);
  /** The listing's columns: its header and its outline rows are drawn from them. */
  const callColumns: Column[] = [
    { label: 'Time', class: 'col-time' },
    { label: '', class: 'col-mark' },
    { label: 'Call', class: 'col-call' },
    { label: 'Model', class: 'col-model' },
    { label: 'Tools requested', class: 'col-tools' },
    { label: 'Context · cached', class: 'col-context', numeric: true },
    { label: 'Out', class: 'col-out', numeric: true },
    { label: 'Duration', class: 'col-duration', numeric: true },
    { label: 'HTTP', class: 'col-http', numeric: true },
  ];
  const calls = $derived(retained?.value?.calls ?? []);
  const nextAfter = $derived(retained?.value?.next ?? null);
  const recording = $derived(retained?.value?.recording);
  const details = $derived(retained?.value?.details ?? new Map());
  /** The listed recordings by ID, to name them in rows. */
  const recordingsById = $derived(
    new Map((retained?.value?.recordings ?? []).map((item) => [item.id, item])),
  );
  const updated = $derived(retained?.updated ?? null);
  const today = date(Date.now());
  const liveStatus = $derived(
    LiveFollow.summary(workbench.live, updated ? `Updated ${Clock.second(updated)}` : null),
  );
  const loading = $derived(workbench.loading);
  /** The daemon's per-body capture limit, to tell a trimmed body from lost bytes. */
  const bodyLimit = $derived(
    Arrival.value<GatewayData>(page.data.gateway)?.storage.capture?.body_limit_bytes ?? null,
  );
  const after = $derived(page.url.searchParams.get('after'));
  const appliedQuery = $derived(
    new URLSearchParams(
      [...page.url.searchParams.entries()].filter(
        ([key]) => !['call', 'after', 'inspect'].includes(key),
      ),
    ).toString(),
  );
  const applied = $derived(new URLSearchParams(appliedQuery));
  const filterCount = $derived([...applied.keys()].length);
  /** The filter form, drawn afresh from the address whenever it changes. */
  let form = $state<ReturnType<typeof TrafficFilters>>();
  /** Whether the form's advanced filters are open; kept across its drafts. */
  let moreFilters = $state(false);
  const selection = new CallSelection(() => calls, fetch);
  /** The page's longest finished call: each row's duration bar is its share. */
  const longest = $derived(Math.max(0, ...calls.map((call) => duration(call) ?? 0)));
  /** When the page was read: an open call runs to it on the timeline. */
  const readAt = $derived(updated ?? 0);
  const groups = $derived(
    recordingId
      ? turnGroups(calls)
      : [{ id: 'recent', label: 'Recent calls', turn: null, rows: calls }],
  );
  /** On the traffic page, rows name their session and narrow to it. */
  const sessions = $derived(
    recordingId ? null : { recordings: recordingsById, details, onfilter: pick },
  );
  function showCallRecording(id: RecordingId) {
    const url = new URL(`/monitor/sessions/${encodeURIComponent(id)}`, page.url.origin);
    if (selection.id) url.searchParams.set('call', selection.id);
    void goto(url);
  }
  function paginate(next: boolean) {
    const url = new URL(page.url.href);
    url.searchParams.delete('call');
    url.searchParams.delete('inspect');
    const cursors = [...(page.state.cursors ?? [])];
    let cursor: string | null;
    if (next && nextAfter) {
      cursors.push(after);
      cursor = nextAfter;
    } else cursor = cursors.pop() ?? null;
    if (cursor) url.searchParams.set('after', cursor);
    else url.searchParams.delete('after');
    void goto(url, { reset: false, state: { cursors }, persistState: true });
  }
  /** Narrow the form's draft and apply it at once. */
  function pick(change: (filters: Filters) => void) {
    form?.pick(change);
  }
  function clearFilters() {
    void goto(page.url.pathname, { reset: false });
  }
  // A custom from–until range (`range`) is none of these, so none is chosen.
  const sinceOptions: SegmentedOption<string>[] = [
    { value: '', label: 'Any', title: 'No time limit' },
    ...sinceChoices.map((choice) => ({
      value: choice.since,
      label: choice.label,
      title: choice.title,
    })),
  ];
  function keydown(event: KeyboardEvent) {
    const typing =
      event.target instanceof HTMLElement &&
      (event.target.matches('input,textarea,select') || event.target.isContentEditable);
    if (typing) return;
    if (
      !event.metaKey &&
      !event.ctrlKey &&
      !event.altKey &&
      (event.key === 'j' || event.key === 'k')
    ) {
      event.preventDefault();
      selection.step(event.key === 'j' ? 1 : -1);
      return;
    }
    if (event.key === '/' && !recordingId) {
      event.preventDefault();
      form?.focusSearch();
    }
  }
</script>

{#snippet callHead()}
  <thead
    ><tr
      >{#each callColumns as column (column.class)}<th
          class={column.class}
          class:numeric={column.numeric}>{column.label}</th
        >{/each}</tr
    ></thead
  >
{/snippet}

{#snippet scope()}
  <div class="scope-group">
    <Segmented
      label="Time"
      options={sinceOptions}
      value={applied.get('since') ?? (applied.get('from_ms') ? 'range' : '')}
      onchange={(since) =>
        pick((f) => {
          f.since = since;
          f.from = '';
          f.until = '';
        })}
    />
  </div>
{/snippet}

<svelte:window onkeydown={keydown} />
{#if selection.error}<div class="notice danger" role="alert">
    <Icon name="alert" size={14} /><span>Selected call: {selection.error}</span><button
      class="text-button"
      onclick={selection.close}>Dismiss</button
    >
  </div>{/if}
<Toolbar scope={recordingId ? undefined : scope} />
<div class="traffic-workspace">
  <section class="panel list-panel traffic-panel" aria-label="Recorded calls">
    {#if recordingId}<SessionHead
        {recordingId}
        {recording}
        detail={details.get(recordingId) ?? null}
        {desk}
        view="calls"
      />{/if}
    {#if !recordingId && !outlined && calls.length}<CallOverview
        {calls}
        now={readAt}
        name={(id) =>
          details.get(id)?.prompt?.text.trim().split('\n')[0] ||
          short(recordingsById.get(id)?.native_id ?? id)}
        selected={selection.id}
        onrecording={(recording) => pick((f) => (f.recording = recording))}
        href={selection.href}
        oncall={selection.select}
      />{/if}
    {#if !recordingId}
      {#key appliedQuery}<TrafficFilters
          bind:this={form}
          bind:more={moreFilters}
          {applied}
          live={liveStatus}
          recordings={retained?.value?.recordings ?? []}
          hidden={retained?.value?.hidden ?? null}
        />{/key}
    {/if}
    {#if recordingId && !outlined && calls.length}<ContextChart
        {calls}
        selected={selection.id}
        href={selection.href}
        oncall={selection.select}
      />{/if}
    <div class="list-content" class:busy={loading && calls.length > 0} aria-busy={loading}>
      {#if outlined}
        <div class="attempt-list">
          <table class="calls-table">
            {@render callHead()}
            <tbody><SkeletonRows columns={callColumns} rows={12} /></tbody>
          </table>
        </div>
      {:else if calls.length === 0 && retained?.error && !loading}
        <div class="empty-state" role="alert">
          <div class="empty-icon"><Icon name="alert" size={26} /></div>
          <h2>
            {retained.error.kind === 'incompatible'
              ? 'This UI and the daemon disagree'
              : "Can't reach Ren"}
          </h2>
          <p>{retained.error.message}</p>
          <button class="button" onclick={() => void invalidateAll()}
            ><Icon name="refresh" size={14} /> Retry</button
          >
        </div>
      {:else if calls.length === 0}
        <div class="empty-state" class:first-run={!filterCount && !recordingId && !after}>
          <div class="empty-icon">
            <Icon name={loading ? 'refresh' : filterCount ? 'search' : 'logs'} size={26} />
          </div>
          <h2>
            {loading
              ? 'Loading requests…'
              : filterCount || !!after
                ? 'No requests match these filters'
                : recordingId
                  ? 'No requests on this page'
                  : 'No recorded traffic'}
          </h2>
          {#if !loading && (filterCount || !!after)}<button class="button" onclick={clearFilters}
              >Clear filters</button
            >
          {:else if !loading && !recordingId}<ConnectClient
              layout="box"
              gatewayOrigin={page.data.gatewayUrl}
            />{/if}
        </div>
      {:else}
        <div class="attempt-list">
          <table class="calls-table">
            {@render callHead()}
            {#each groups as group (group.id)}
              <tbody>
                {#if recordingId}<TurnRow
                    {group}
                    prompt={group.turn ? turns?.prompts.get(group.turn) : null}
                    opened={turnOpen}
                    columns={callColumns.length}
                    ontoggle={openTurn}
                  />{/if}
                {#each group.rows as call, index (call.id)}<CallRow
                    {call}
                    previous={group.rows[index - 1]}
                    {selection}
                    {today}
                    {bodyLimit}
                    {longest}
                    {sessions}
                  />{/each}
              </tbody>
            {/each}
          </table>
        </div>
      {/if}
    </div>
    <div class="table-footer">
      {#if outlined}<span class="muted">Reading requests…</span>{:else}<span
          >{num(calls.length)} call{calls.length === 1 ? '' : 's'}
          <span class="muted">· {recordingId ? 'Oldest first' : 'Newest first'}</span></span
        >{/if}
      <div class="pagination">
        <button class="button small" disabled={loading || !after} onclick={() => paginate(false)}
          ><span class="rotate"><Icon name="chevron" size={13} /></span> Previous</button
        ><span>{after ? 'Older requests' : 'Latest requests'}</span><button
          class="button small"
          disabled={loading || !nextAfter}
          onclick={() => paginate(true)}>Next <Icon name="chevron" size={13} /></button
        >
      </div>
    </div>
  </section>
  {#if selection.call}<SlideOver
      label="Call details"
      pin="call"
      onclose={() => void selection.close()}
      size="wide"
      >{#key selection.call.id}<Inspector
          call={selection.call}
          onrecording={showCallRecording}
          onupdate={selection.observe}
        />{/key}</SlideOver
    >{/if}
</div>

{#if recordingId && turnOpen}
  <SlideOver label="Turn details" size="wide" pin="turn" onclose={() => openTurn(null)}>
    <TurnPanel
      turn={turnOpen}
      metrics={turns?.metrics.get(turnOpen) ?? null}
      report={`/monitor/sessions/${encodeURIComponent(recordingId)}/report`}
    />
  </SlideOver>
{/if}

<style>
  .calls-table {
    table-layout: fixed;
  }
  .calls-table :global(td) {
    vertical-align: middle;
  }
  /* The header's column widths; each row's cells carry the same. */
  .calls-table .col-time {
    width: 88px;
    white-space: nowrap;
  }
  .calls-table .col-mark {
    width: 26px;
    padding-right: 0;
  }
  .calls-table .col-call {
    width: 32%;
  }
  .calls-table .col-model {
    width: 16%;
  }
  .calls-table .col-tools {
    width: 16%;
  }
  .calls-table .col-context {
    width: 128px;
  }
  .calls-table .col-out {
    width: 64px;
  }
  .calls-table .col-duration {
    width: 112px;
  }
  .calls-table .col-http {
    width: 56px;
  }
  .numeric {
    text-align: right;
  }
  @media (max-width: 960px) {
    .calls-table .col-tools,
    .calls-table .col-out,
    .calls-table .col-http {
      display: none;
    }
  }
  @media (max-width: 760px) {
    .calls-table .col-context,
    .calls-table .col-model {
      display: none;
    }
  }
</style>
