<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import SlideOver from '#lib/layer/SlideOver.svelte';
  import Toolbar from './Toolbar.svelte';
  import { untrack } from 'svelte';
  import { invalidateAll } from '$app/navigation';
  import { goto } from '$app/navigation';
  import { page, navigating } from '$app/state';
  import { getWorkbench } from './workbench.svelte.ts';
  import { createArrival } from './snapshot.svelte.ts';
  import type { Arriving } from './arrival.ts';
  import type { Column } from './columns.ts';
  import SkeletonRows from './SkeletonRows.svelte';
  import { offPageSelection, type OffPageSelection, type TrafficData } from './loaders.ts';
  import { CallId, type CallView, type RecordingId } from './api.ts';
  import { num, short, title, clock, providerName, providerMark } from './api.ts';
  import {
    isActive,
    captureChip,
    duration,
    date,
    httpChip,
    streamChip,
    recordingTone,
    compactOrDash,
    callerShown,
    SEPARATOR,
  } from './display.ts';
  import {
    type Filters,
    type TextFilter,
    TEXT_FILTERS,
    clientChoices,
    sinceChoices,
    emptyFilters,
    trafficParams,
    filtersFromParams,
    selectedCall,
    turnGroups,
    neighbour,
  } from './traffic.ts';
  import { LiveFollow } from './live.ts';
  import Icon from './Icon.svelte';
  import SessionHead from './SessionHead.svelte';
  import TurnPanel from './TurnPanel.svelte';
  import { ManagementApi, type TurnId, type TurnMetrics } from './api.ts';
  import Inspector from './Inspector.svelte';
  import CallOverview from './CallOverview.svelte';
  import ContextChart from './ContextChart.svelte';
  import HoverCard from './layer/HoverCard.svelte';
  import { CallTimeline } from './timeline.ts';
  import { Steps } from './steps.ts';
  import Select, { type SelectOption } from './Select.svelte';
  import Segmented, { type SegmentedOption } from './Segmented.svelte';
  import ConnectClient from './ConnectClient.svelte';
  import RecordingName from './RecordingName.svelte';
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
  /** The turn open beside the calls. */
  let turnOpen = $state<TurnId | null>(null);
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
    LiveFollow.summary(workbench.live, updated ? `Updated ${clock(updated)}` : null),
  );
  const loading = $derived(workbench.manual || !!navigating.to);
  /** The daemon's per-body capture limit, to tell a trimmed body from lost bytes. */
  const bodyLimit = $derived(page.data.gateway?.value?.storage.capture?.body_limit_bytes ?? null);
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
  let filters = $state(emptyFilters());
  $effect(() => {
    filters = filtersFromParams(new URLSearchParams(applied));
  });
  let filterError = $state('');
  let moreFilters = $state(false);
  let searchInput = $state<HTMLInputElement>();
  const linkedCall = $derived(page.url.searchParams.get('call'));
  // A clicked row is selected at once; the URL catches up a moment later.
  let pending = $state<CallId | null>(null);
  $effect(() => {
    if (pending !== null && linkedCall === pending) pending = null;
  });
  /** The call open beside the list: only one the operator picked. */
  const selectedId = $derived(pending ?? (linkedCall !== null ? CallId.parse(linkedCall) : null));
  let observed = $state<CallView | undefined>();
  // A linked call that is not on this page is read on its own; the listing
  // never reloads for a selection.
  let offPage = $state<OffPageSelection>({ kind: 'none' });
  $effect(() => {
    const raw = linkedCall;
    const listed = calls;
    if (raw === null || listed.some((call) => call.id === raw)) {
      offPage = { kind: 'none' };
      return;
    }
    const current = untrack(() => offPage);
    if (current.kind === 'loaded' && current.call.id === raw) return;
    const controller = new AbortController();
    void offPageSelection(fetch, raw, listed, controller.signal).then((result) => {
      if (!controller.signal.aborted) offPage = result;
    });
    return () => controller.abort();
  });
  const selected = $derived(
    selectedCall(selectedId, calls, observed, offPage.kind === 'loaded' ? offPage.call : undefined),
  );
  const selectionError = $derived(
    linkedCall !== null && !selected && offPage.kind === 'failed' ? offPage.message : '',
  );
  /** The page's longest finished call: each row's duration bar is its share. */
  const longest = $derived(Math.max(0, ...calls.map((call) => duration(call) ?? 0)));
  /** When the page was read: an open call runs to it on the timeline. */
  const readAt = $derived(updated ?? 0);
  const groups = $derived(
    recordingId
      ? turnGroups(calls)
      : [{ id: 'recent', label: 'Recent calls', turn: null, rows: calls }],
  );
  function callHref(id: CallId) {
    const url = new URL(page.url.href);
    url.searchParams.set('call', id);
    url.searchParams.delete('inspect');
    return url.pathname + url.search;
  }
  /** A click anywhere on a row selects it, except on its own links and
   * buttons, or when the click ended a text selection. */
  function selectRow(event: MouseEvent, id: CallId) {
    if (event.target instanceof Element && event.target.closest('a,button,input,select')) return;
    if (window.getSelection()?.toString()) return;
    selectCall(event, id);
  }
  function selectCall(event: MouseEvent, id: CallId) {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)
      return;
    event.preventDefault();
    pending = id;
    void goto(callHref(id), {
      reset: false,
      state: page.state,
      persistState: true,
    }).finally(() => {
      // Whether it landed or was superseded, the URL decides from here.
      if (pending === id) pending = null;
    });
  }
  async function closeInspector() {
    const id = selectedId;
    const url = new URL(page.url.href);
    url.searchParams.delete('call');
    await goto(url, { reset: false, state: page.state, persistState: true });
    if (id) document.getElementById(`call-${id}`)?.focus();
  }
  function updateCall(record: CallView) {
    observed = record;
  }
  function showCallRecording(id: RecordingId) {
    const url = new URL(`/monitor/sessions/${encodeURIComponent(id)}`, page.url.origin);
    if (selectedId) url.searchParams.set('call', selectedId);
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
  function applyFilters() {
    try {
      const query = trafficParams(filters);
      filterError = '';
      void goto(`${page.url.pathname}${query ? `?${query}` : ''}`, { reset: false });
    } catch (error) {
      filterError = error instanceof Error ? error.message : 'Invalid filters.';
    }
  }
  function resetFilters() {
    filters = emptyFilters();
    applyFilters();
  }
  function quickFilter(status: string, toolsOnly = false) {
    filters.status = status;
    filters.has_tools = toolsOnly;
    applyFilters();
  }
  function removeFilter(key: string) {
    if (key === 'from_ms') filters.from = '';
    else if (key === 'until_ms') filters.until = '';
    else if (key === 'has_tools') filters.has_tools = false;
    else if (key === 'kind') filters.kind = 'model';
    else if ((TEXT_FILTERS as readonly string[]).includes(key)) filters[key as TextFilter] = '';
    applyFilters();
  }
  /** Narrow to one choice and apply at once. */
  function pick(change: (filters: Filters) => void) {
    change(filters);
    applyFilters();
  }
  const recordingChoices = $derived(retained?.value?.recordings ?? []);
  const recordingOptions = $derived<SelectOption<string>[]>([
    { value: '', label: 'All sessions' },
    ...recordingChoices.map((choice) => ({
      value: choice.id,
      label: short(choice.native_id),
      hint: `${providerName(choice.provider)} · ${date(choice.last_seen)} ${clock(choice.last_seen)}`,
    })),
  ]);
  const clientOptions: SegmentedOption<string>[] = [
    { value: '', label: 'All clients' },
    ...clientChoices.map((choice) => ({ value: choice.client, label: choice.label })),
  ];
  // A custom from–until range (`range`) is none of these, so none is chosen.
  const sinceOptions: SegmentedOption<string>[] = [
    { value: '', label: 'Any', title: 'No time limit' },
    ...sinceChoices.map((choice) => ({
      value: choice.since,
      label: choice.label,
      title: choice.title,
    })),
  ];
  const providerOptions: SelectOption<string>[] = [
    { value: '', label: 'All providers' },
    { value: 'anthropic', label: 'Anthropic' },
    { value: 'openai', label: 'OpenAI' },
    { value: 'codex', label: 'Codex' },
    { value: 'deepseek', label: 'DeepSeek' },
  ];
  const statusOptions: SelectOption<string>[] = [
    { value: '', label: 'All statuses' },
    { value: 'active', label: 'In progress' },
    { value: 'finished', label: 'Finished' },
    { value: 'errors', label: 'Errors' },
    { value: 'interrupted', label: 'Interrupted / cancelled' },
  ];
  const hidden = $derived(retained?.value?.hidden ?? null);
  function recordingName(id: string) {
    const found = recordingChoices.find((item) => item.id === id);
    return found ? short(found.native_id) : short(id);
  }
  function chipLabel(key: string, value: string) {
    if (key === 'recording') return `Session: ${recordingName(value)}`;
    if (key === 'user') return `User: ${value}`;
    if (key === 'client')
      return `Client: ${clientChoices.find((item) => item.client === value)?.label ?? value}`;
    if (key === 'since')
      return sinceChoices.find((item) => item.since === value)?.title ?? `Since: ${value}`;
    if (key === 'kind') return value === 'other' ? 'Bookkeeping calls only' : 'All call kinds';
    if (key === 'from_ms' || key === 'until_ms')
      return `${key === 'from_ms' ? 'From' : 'Until'}: ${new Date(Number(value)).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}`;
    return key === 'has_tools'
      ? 'Has tool calls'
      : `${key === 'q' ? 'Search' : title(key)}: ${value}`;
  }
  /** `j` and `k` select the next and previous call, as in a mail reader. */
  function step(direction: 1 | -1) {
    const next = neighbour(calls, selectedId, direction);
    if (!next || next.id === selectedId) return;
    pending = next.id;
    void goto(callHref(next.id), {
      reset: false,
      state: page.state,
      persistState: true,
    }).finally(() => {
      if (pending === next.id) pending = null;
    });
    document.getElementById(`call-${next.id}`)?.closest('tr')?.scrollIntoView({ block: 'nearest' });
  }
  function keydown(event: KeyboardEvent) {
    const typing =
      event.target instanceof HTMLElement &&
      (event.target.matches('input,textarea,select') || event.target.isContentEditable);
    if (!typing && !event.metaKey && !event.ctrlKey && !event.altKey) {
      if (event.key === 'j' || event.key === 'k') {
        event.preventDefault();
        step(event.key === 'j' ? 1 : -1);
        return;
      }
    }
    if (
      event.key === '/' &&
      !recordingId &&
      !(
        event.target instanceof HTMLElement &&
        (event.target.matches('input,textarea,select') || event.target.isContentEditable)
      )
    ) {
      event.preventDefault();
      searchInput?.focus();
    }
  }
</script>

{#snippet callHead()}
  <thead
    ><tr
      >{#each callColumns as column (column.class)}<th
          class={column.class}
          class:numeric={column.numeric}
          >{#if column.class === 'col-context'}<HoverCard label="What context counts"
              >{#snippet trigger()}<button type="button" class="tip-trigger">{column.label}</button
                >{/snippet}
              <strong>The prompt the model saw</strong>
              <span
                >As the provider reported it: new input, plus what was read from the cache and
                written to it. The bar is the share read from the cache.</span
              >
              <span class="muted"
                >Provider-reported tokens, not subscription allowance or charges.</span
              ></HoverCard
            >{:else}{column.label}{/if}</th
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
{#if selectionError}<div class="notice danger" role="alert">
    <Icon name="alert" size={14} /><span>Selected call: {selectionError}</span><button
      class="text-button"
      onclick={closeInspector}>Dismiss</button
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
        now={Math.max(readAt, Date.now())}
      />{/if}
    {#if !recordingId && !outlined && calls.length}<CallOverview
        {calls}
        now={readAt}
        name={(id) =>
          details.get(id)?.prompt?.text.trim().split('\n')[0] ||
          short(recordingsById.get(id)?.native_id ?? id)}
        selected={selectedId}
        onrecording={(recording) => pick((f) => (f.recording = recording))}
        href={callHref}
        oncall={selectCall}
      />{/if}
    {#if !recordingId}
      <form
        onsubmit={(e) => {
          e.preventDefault();
          applyFilters();
        }}
        aria-label="Traffic filters"
      >
        <div class="traffic-bar">
          <div class="request-tabs" aria-label="Quick filters">
            <button
              class:active={!applied.get('status') && !applied.get('has_tools')}
              aria-pressed={!applied.get('status') && !applied.get('has_tools')}
              onclick={() => quickFilter('')}>All calls</button
            >
            <button
              class:active={applied.get('status') === 'active'}
              aria-pressed={applied.get('status') === 'active'}
              onclick={() => quickFilter('active')}><Glyph mark="responding" /> In progress</button
            >
            <button
              class:active={applied.get('status') === 'errors'}
              aria-pressed={applied.get('status') === 'errors'}
              onclick={() => quickFilter('errors')}>Errors</button
            >
            <button
              class:active={applied.get('has_tools') === 'true'}
              aria-pressed={applied.get('has_tools') === 'true'}
              onclick={() => quickFilter('', true)}>Tool requests</button
            >
            <span class="table-update" role="status">{liveStatus}</span>
          </div>
          <div class="filter-toolbar">
            <label class="search-input main-search"
              ><Icon name="search" /><input
                bind:this={searchInput}
                aria-label="Search traffic"
                placeholder="Search call ID, model, or tool…"
                maxlength="128"
                bind:value={filters.q}
              /><kbd>/</kbd></label
            ><button
              class="button"
              type="button"
              class:chosen={moreFilters}
              aria-expanded={moreFilters}
              aria-controls="advanced-filters"
              onclick={() => (moreFilters = !moreFilters)}
              ><Icon name="filter" size={14} /> Filters {#if filterCount}<span class="count-pill"
                  >{filterCount}</span
                >{/if}</button
            >
          </div>
        </div>
        {#if moreFilters}<div class="advanced-filters" id="advanced-filters">
            <div class="filter-field wide">
              <span aria-hidden="true">Client</span><Segmented
                label="Client"
                options={clientOptions}
                value={applied.get('client') ?? ''}
                onchange={(client) => pick((f) => (f.client = client))}
              />
            </div>
            <div class="filter-field">
              <span aria-hidden="true">Session</span><Select
                label="Session"
                options={recordingOptions}
                value={applied.get('recording') ?? ''}
                placeholder={recordingName(applied.get('recording') ?? '')}
                onchange={(recording) => pick((f) => (f.recording = recording))}
              />
            </div>
            <div class="filter-field">
              <span aria-hidden="true">Provider</span><Select
                label="Provider"
                options={providerOptions}
                bind:value={filters.provider}
                onchange={applyFilters}
              />
            </div>
            <label
              class="toggle-control model-only"
              title="Hide token counts, model listings, and WebSocket handshakes"
              ><input
                type="checkbox"
                checked={(applied.get('kind') ?? 'model') === 'model'}
                onchange={(e) => pick((f) => (f.kind = e.currentTarget.checked ? 'model' : 'all'))}
              /><span class="switch"></span><span
                >Model calls only{#if hidden}<span class="muted hidden-count"
                    >{hidden >= 100 ? '100+' : hidden} hidden</span
                  >{/if}</span
              ></label
            >
            <label
              >User<input
                placeholder="Named user id…"
                maxlength="64"
                bind:value={filters.user}
              /></label
            ><label
              >Model<input
                placeholder="Model contains…"
                maxlength="128"
                bind:value={filters.model}
              /></label
            ><label
              >Tool name<input
                placeholder="Tool contains…"
                maxlength="128"
                bind:value={filters.tool}
              /></label
            >
            <div class="filter-field">
              <span aria-hidden="true">Status</span><Select
                label="Status"
                options={statusOptions}
                bind:value={filters.status}
              />
            </div>
            <label>From · local time<input type="datetime-local" bind:value={filters.from} /></label
            ><label
              >Until · local time<input type="datetime-local" bind:value={filters.until} /></label
            ><label class="check-label"
              ><input type="checkbox" bind:checked={filters.has_tools} /> Has observed tool calls</label
            >
            <div class="filter-note">
              <span>Model and tool filters use observations from finished responses.</span><button
                class="button small"
                type="button"
                onclick={resetFilters}>Reset</button
              ><button class="button primary small" type="submit">Apply filters</button>
            </div>
          </div>{/if}
        {#if filterCount}<div class="filter-chips">
            {#each [...applied.entries()] as [key, value]}<button
                type="button"
                class="filter-chip"
                onclick={() => removeFilter(key)}
                aria-label={`Remove ${chipLabel(key, value)}`}
                >{chipLabel(key, value)}<Icon name="close" size={12} /></button
              >{/each}<button type="button" class="text-button" onclick={resetFilters}
              >Clear all</button
            >
          </div>{/if}
        {#if filterError}<div class="inline-error" role="alert">
            <Icon name="alert" size={14} />{filterError}
          </div>{/if}
      </form>
    {/if}
    {#if recordingId && !outlined && calls.length}<ContextChart
        {calls}
        selected={selectedId}
        href={callHref}
        oncall={selectCall}
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
          <p class="muted">
            Nothing is shown because nothing could be read, not because nothing was recorded.
          </p>
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
          <p>
            {loading
              ? 'Reading captured traffic from your gateway.'
              : filterCount || !!after
                ? 'Try a broader search or adjust your filters.'
                : recordingId
                  ? 'Refresh to check for new activity in this session.'
                  : 'Point a client at Ren or use a launch command below.'}
          </p>
          {#if !loading && (filterCount || !!after)}<button class="button" onclick={resetFilters}
              >Clear filters & return to first page</button
            >
          {:else if !loading && !recordingId}<ConnectClient
              layout="box"
              gatewayOrigin={page.data.gatewayUrl}
            />
            <span class="empty-footnote"
              ><Icon name="shield" size={12} /> Uses your client’s existing sign-in</span
            >{/if}
        </div>
      {:else}
        <div class="attempt-list">
          <table class="calls-table">
            {@render callHead()}
            {#each groups as group (group.id)}
              <tbody>
                {#if recordingId}{@const turn = group.turn}{@const prompt = turn
                    ? turns?.prompts.get(turn)
                    : null}<tr class="turn-row" class:open={turn !== null && turnOpen === turn}
                    ><th colspan={callColumns.length}
                      >{#if turn}<button
                          class="turn-label"
                          onclick={() => (turnOpen = turnOpen === turn ? null : turn)}
                          aria-expanded={turnOpen === turn}
                          title="This turn's details">{prompt || group.label}</button
                        >{:else}<span class="turn-label">{group.label}</span
                        >{/if}{#if group.rows.some((call) => CallTimeline.mark(call).mark === 'failed')}<Glyph
                          mark="failed"
                          label={true}
                        />{/if}<span class="muted mono"
                        >{group.rows.length} call{group.rows.length === 1 ? '' : 's'} · {clock(
                          group.rows[0]?.started_at ?? 0,
                        )}</span
                      ></th
                    ></tr
                  >{/if}
                {#each group.rows as c, index (c.id)}{@const said =
                    CallTimeline.mark(c)}{@const http = httpChip(c.status)}{@const stream =
                    streamChip(c)}{@const capture = captureChip(c, bodyLimit)}{@const context =
                    CallTimeline.context(c)}{@const took = duration(c)}
                  <tr
                    class="call-row"
                    class:selected={selectedId === c.id}
                    onclick={(event) => selectRow(event, c.id)}
                  >
                    <td class="col-time mono" title={new Date(c.started_at).toLocaleString()}
                      >{clock(c.started_at)}{#if date(c.started_at) !== today}<small
                          >{date(c.started_at)}</small
                        >{/if}</td
                    >
                    <td class="col-mark"><Glyph mark={said.mark} label={said.label} /></td>
                    <td class="col-call"
                      ><a
                        class="row-link mono"
                        id={`call-${c.id}`}
                        href={callHref(c.id)}
                        onclick={(event) => selectCall(event, c.id)}
                        aria-label={`Inspect call ${c.id}`}
                        aria-current={selectedId === c.id ? 'true' : undefined}>{short(c.id)}</a
                      >{#if stream.tone !== 'success' && stream.tone !== 'live'}<span
                          class="chip {stream.tone}"
                          title={stream.title}>{stream.label}</span
                        >{/if}{#if capture.tone === 'warning'}<span
                          class="chip warning"
                          title={capture.title}>{capture.label}</span
                        >{/if}{#if !recordingId}{@const previous = group.rows[index - 1]}<small
                          class="mono recording-line"
                          title={c.attribution.hint?.session ??
                            'No session identifier was reported'}
                          >{#if c.recording}{@const recording = c.recording}{@const known =
                              recordingsById.get(recording)}<HoverCard
                              label="Recording {short(recording)}"
                              >{#snippet trigger()}<button
                                  type="button"
                                  class="recording-filter"
                                  title="Show only this session's calls"
                                  aria-label={`Show only recording ${short(c.attribution.hint?.session ?? recording)}`}
                                  onclick={() => pick((f) => (f.recording = recording))}
                                  >{#if recordingsById.get(recording) && (!previous || previous.recording !== c.recording)}<RecordingName
                                      recording={recordingsById.get(recording)!}
                                      detail={details.get(recording)}
                                      client={false}
                                    />{:else}<span
                                      class="recording-dot tone-{recordingTone(recording)}"
                                    ></span>{#if !previous || previous.recording !== c.recording}{c
                                        .attribution.hint
                                        ? short(c.attribution.hint.session)
                                        : 'Session'}{/if}{/if}</button
                                >{/snippet}
                              {#if known}
                                <RecordingName recording={known} detail={details.get(recording)} />
                                <span class="card-figures mono"
                                  ><span>{num(known.call_count)} calls</span><span
                                    >{num(known.turn_count)} turns</span
                                  >{#if known.error_count}<span class="card-errors"
                                      ><Glyph mark="failed" label="Failed calls" />{num(
                                        known.error_count,
                                      )}</span
                                    >{/if}<span
                                    >{compactOrDash(known.tokens.input)} in · {compactOrDash(
                                      known.tokens.output,
                                    )} out</span
                                  ></span
                                >
                                <a
                                  class="text-button"
                                  href="/monitor/sessions/{encodeURIComponent(recording)}"
                                  >Open the session <Icon name="arrow" size={11} /></a
                                >
                              {:else}<span class="muted">Not in this page's listing.</span>{/if}
                            </HoverCard>{:else if !previous || previous.recording !== c.recording}No
                            recording{/if}{#if c.caller.user || c.caller.login}{@const caller =
                              callerShown(c.caller)}<span class="caller-line"
                              >{SEPARATOR}{#if caller.user}{@const user = caller.user}<button
                                  type="button"
                                  class="recording-filter"
                                  title={`${caller.title}. Show only this user's calls`}
                                  aria-label={`Show only user ${user}'s calls`}
                                  onclick={() => pick((f) => (f.user = user))}
                                  >{caller.label}</button
                                >{:else}<span title={caller.title}>{caller.label}</span>{/if}</span
                            >{/if}</small
                        >{/if}</td
                    >
                    <td class="col-model"
                      ><span class="model-line"
                        ><span
                          class="provider-mark tiny {c.provider}"
                          title={providerName(c.provider)}>{providerMark(c.provider)}</span
                        ><span class="model-name mono" title={c.model ?? title(c.endpoint)}
                          >{c.model ??
                            (isActive(c) ? 'Awaiting observation' : title(c.endpoint))}</span
                        ></span
                      ></td
                    >
                    <td class="col-tools"
                      >{#if c.tool_names.length}<span class="tool-chips"
                          >{#each c.tool_names.slice(0, 2) as name, at (at)}<span
                              class="chip tool-chip mono">{name}</span
                            >{/each}{#if c.tool_names.length > 2}<span class="chip mono"
                              >+{c.tool_names.length - 2}</span
                            >{/if}</span
                        >{/if}</td
                    >
                    <td class="col-context numeric mono"
                      >{#if context}<span
                          class="figure"
                          title="{context.tokens.toLocaleString()} tokens of prompt, {Math.round(
                            context.cached * 100,
                          )}% read from the cache"
                          ><svg class="meter" viewBox="0 0 100 4" preserveAspectRatio="none"
                            ><rect class="meter-whole" width="100" height="4" /><rect
                              class="meter-cached"
                              width={context.cached * 100}
                              height="4"
                            /></svg
                          >{compactOrDash(context.tokens)}</span
                        >{:else}<span class="muted">—</span>{/if}</td
                    >
                    <td class="col-out numeric mono"
                      >{compactOrDash(c.usage?.output_tokens ?? null)}</td
                    >
                    <td class="col-duration numeric mono"
                      ><span class="figure"
                        >{#if took !== null && longest > 0}<svg
                            class="meter"
                            viewBox="0 0 100 4"
                            preserveAspectRatio="none"
                            ><rect class="meter-whole" width="100" height="4" /><rect
                              class="meter-took state-{said.mark}"
                              width={Math.max(2, (took / longest) * 100)}
                              height="4"
                            /></svg
                          >{/if}{took === null ? '' : Steps.time(took)}</span
                      ></td
                    >
                    <td class="col-http numeric mono"
                      ><span class="status-code {http.tone}" title={http.title}>{http.label}</span
                      ></td
                    >
                  </tr>
                {/each}
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
  {#if selected}<SlideOver
      label="Call details"
      pin="call"
      onclose={() => void closeInspector()}
      size="wide"
      >{#key selected.id}<Inspector
          call={selected}
          onrecording={showCallRecording}
          onupdate={updateCall}
        />{/key}</SlideOver
    >{/if}
</div>

{#if recordingId && turnOpen}
  <SlideOver label="Turn details" size="wide" pin="turn" onclose={() => (turnOpen = null)}>
    <TurnPanel
      turn={turnOpen}
      metrics={turns?.metrics.get(turnOpen) ?? null}
      report={`/monitor/sessions/${encodeURIComponent(recordingId)}/report`}
    />
  </SlideOver>
{/if}

<style>
  .traffic-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    border-bottom: 1px solid var(--line);
  }
  .traffic-bar .request-tabs {
    border: 0;
    padding: 0;
  }
  .traffic-bar .filter-toolbar {
    flex: 1;
    min-width: 280px;
    padding: 0;
    border: 0;
  }
  .filter-field.wide {
    grid-column: 1 / -1;
  }
  .advanced-filters .model-only {
    align-self: end;
    justify-self: start;
    gap: 8px;
    height: 30px;
  }
  .calls-table {
    table-layout: fixed;
  }
  .calls-table :global(td) {
    vertical-align: middle;
  }
  .calls-table .col-time {
    width: 88px;
    white-space: nowrap;
  }
  .calls-table .col-mark {
    width: 26px;
    padding-right: 0;
  }
  .calls-table .col-call {
    width: 24%;
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
  .recording-line :global(.recording-filter) {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tip-trigger {
    padding: 0;
    border: 0;
    border-bottom: 1px dotted currentColor;
    border-radius: 0;
    background: none;
    color: inherit;
    font: inherit;
    cursor: help;
  }
  .card-figures {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    color: var(--muted);
    font-size: 11px;
  }
  .card-errors {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .turn-row th {
    padding: 6px 10px;
    border-bottom: 1px solid var(--line);
    background: var(--surface-subtle);
    color: var(--ink);
    font-weight: 400;
    text-align: left;
  }
  .turn-row th > * {
    vertical-align: middle;
  }
  .turn-label {
    max-width: 70%;
    margin-right: 10px;
    overflow: hidden;
    font-size: 12.5px;
    font-weight: 500;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  button.turn-label {
    display: inline-block;
    text-align: left;
  }
  button.turn-label:hover {
    color: var(--accent);
  }
  .turn-row.open th {
    background: var(--row-selected);
  }
  .turn-row .muted {
    float: right;
    color: var(--muted);
    font-size: 11px;
  }
  .col-time small {
    display: block;
    color: var(--muted);
  }
  .col-call {
    overflow: hidden;
  }
  .col-call .chip {
    margin-left: 6px;
  }
  .model-line {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .model-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tool-chips {
    display: flex;
    gap: 4px;
    overflow: hidden;
  }
  .figure {
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
    width: 100%;
  }
  .meter {
    flex: 1;
    max-width: 52px;
    height: 4px;
  }
  .meter-whole {
    fill: var(--line);
  }
  .meter-cached {
    fill: var(--success);
  }
  .meter-took {
    fill: var(--muted);
  }
  .meter-took.state-failed {
    fill: var(--error);
  }
  .meter-took.state-warning,
  .meter-took.state-unknown {
    fill: var(--warning);
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
