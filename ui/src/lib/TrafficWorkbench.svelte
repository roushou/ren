<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import { invalidateAll } from '$app/navigation';
  import { goto } from '$app/navigation';
  import { page, navigating } from '$app/state';
  import { getWorkbench } from './workbench.svelte.ts';
  import { createSnapshot } from './snapshot.svelte.ts';
  import type { Snapshot } from './snapshots.ts';
  import { offPageSelection, type OffPageSelection, type TrafficData } from './loaders.ts';
  import { CallId, type CallView, type SessionId } from './api.ts';
  import { num, short, title, clock, providerName, providerMark } from './api.ts';
  import {
    isActive,
    callStatus,
    captureChip,
    statusBadge,
    duration,
    elapsed,
    date,
    httpChip,
    streamChip,
    tokenSummary,
    sessionTone,
    compact,
  } from './display.ts';
  import {
    type Filters,
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
  import Inspector from './Inspector.svelte';
  import CopyButton from './CopyButton.svelte';
  let { snapshot, sessionId }: { snapshot: Snapshot<TrafficData>; sessionId: SessionId | null } =
    $props();
  const workbench = getWorkbench();
  // The scope bar shows the narrowing controls, or the session itself.
  workbench.showScope(scope);
  const retained = createSnapshot(() => snapshot);
  const calls = $derived(retained.current.value?.calls ?? []);
  const nextAfter = $derived(retained.current.value?.next ?? null);
  const session = $derived(retained.current.value?.session);
  const updated = $derived(retained.current.updated);
  const today = date(Date.now());
  const liveStatus = $derived(
    LiveFollow.summary(workbench.live, updated ? `Updated ${clock(updated)}` : null),
  );
  const loading = $derived(workbench.manual || !!navigating.to);
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
  let connectClient = $state('claude');
  let searchInput = $state<HTMLInputElement>();
  const gatewayOrigin = $derived(page.data.gatewayUrl);
  const connectionCommand = $derived(
    `tgv run ${connectClient}${gatewayOrigin !== 'http://127.0.0.1:8787' && gatewayOrigin !== 'http://localhost:8787' ? ` --gateway ${gatewayOrigin}` : ''}`,
  );
  // On a phone the list and the call are separate screens: nothing is
  // selected until a row is tapped.
  let narrow = $state(false);
  onMount(() => {
    const query = window.matchMedia('(max-width: 760px)');
    narrow = query.matches;
    const change = () => (narrow = query.matches);
    query.addEventListener('change', change);
    return () => query.removeEventListener('change', change);
  });
  const linkedCall = $derived(page.url.searchParams.get('call'));
  // A clicked row is selected at once; the URL catches up a moment later.
  let pending = $state<CallId | null>(null);
  $effect(() => {
    if (pending !== null && linkedCall === pending) pending = null;
  });
  const selectedId = $derived(
    pending ??
      (linkedCall !== null
        ? CallId.parse(linkedCall)
        : page.url.searchParams.get('inspect') === 'off' || narrow
          ? null
          : calls[0]?.id),
  );
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
  $effect(() => {
    workbench.selected = selected;
    workbench.callCount = calls.length;
    workbench.activeCount = calls.filter(isActive).length;
  });
  onDestroy(() => {
    workbench.selected = undefined;
    workbench.callCount = 0;
    workbench.activeCount = 0;
  });
  const groups = $derived(
    sessionId
      ? turnGroups(calls)
      : [{ id: 'recent', label: 'Recent calls', turn: null, rows: calls }],
  );
  function callHref(id: CallId) {
    const url = new URL(page.url.href);
    url.searchParams.set('call', id);
    url.searchParams.delete('inspect');
    return url.pathname + url.search;
  }
  $effect(() => {
    // Canonicalize the initial selection through Kit's data-loading navigation. Live
    // refreshes must never switch the inspector when a newer call arrives.
    if (
      !narrow &&
      !page.url.searchParams.has('call') &&
      page.url.searchParams.get('inspect') !== 'off' &&
      calls.length
    ) {
      void goto(callHref(calls[0].id), {
        replace: true,
        reset: false,
        state: page.state,
        persistState: true,
      });
    }
  });
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
    url.searchParams.set('inspect', 'off');
    await goto(url, { reset: false, state: page.state, persistState: true });
    if (id) document.getElementById(`call-${id}`)?.focus();
  }
  function updateCall(record: CallView) {
    observed = record;
  }
  function showCallSession(id: SessionId) {
    const url = new URL(`/sessions/${encodeURIComponent(id)}`, page.url.origin);
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
    else if (
      ['q', 'session', 'client', 'since', 'provider', 'model', 'tool', 'status'].includes(key)
    )
      filters[
        key as 'q' | 'session' | 'client' | 'since' | 'provider' | 'model' | 'tool' | 'status'
      ] = '';
    applyFilters();
  }
  /** Narrow to one choice and apply at once. */
  function pick(change: (filters: Filters) => void) {
    change(filters);
    applyFilters();
  }
  const sessionChoices = $derived(retained.current.value?.sessions ?? []);
  const hidden = $derived(retained.current.value?.hidden ?? null);
  function sessionName(id: string) {
    const found = sessionChoices.find((item) => item.id === id);
    return found ? short(found.native_id) : short(id);
  }
  function chipLabel(key: string, value: string) {
    if (key === 'session') return `Session: ${sessionName(value)}`;
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
    if (event.key === 'Escape' && selected) void closeInspector();
    if (
      event.key === '/' &&
      !sessionId &&
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

{#snippet scope()}
  {#if sessionId}
    <div class="scope-group scope-identity">
      {#if session}<span class="session-dot tone-{sessionTone(session.id)}"></span><span
          class="provider-mark tiny {session.provider}">{providerMark(session.provider)}</span
        ><strong class="mono" title="The session ID your client reported"
          >{short(session.native_id)}</strong
        ><small class="mono muted" title={`TGV's own ID for this session: ${session.id}`}
          >TGV {short(session.id)}</small
        >{:else}<strong class="mono">{short(sessionId)}</strong>{/if}
    </div>
    {#if session}<div class="scope-group scope-figures">
        <span><b class="mono">{num(session.call_count)}</b> calls</span>
        <span><b class="mono">{num(session.turn_count)}</b> turns</span>
        <span class:error-count={!!session.error_count}
          ><b class="mono">{num(session.error_count)}</b> errors</span
        >
        <span
          title="Provider reported; input includes cached context. Not subscription allowance or charges."
          ><b class="mono">{session.tokens.input === null ? '—' : compact(session.tokens.input)}</b>
          in ·
          <b class="mono">{session.tokens.output === null ? '—' : compact(session.tokens.output)}</b
          > out</span
        >
      </div>{/if}
    <div class="scope-group end">
      <a class="text-button" href={`/sessions/${encodeURIComponent(sessionId)}/report`}
        >Turns and report <Icon name="arrow" size={12} /></a
      ><a class="text-button" href={`/traffic?session=${encodeURIComponent(sessionId)}`}
        >Narrow in Traffic <Icon name="arrow" size={12} /></a
      ><a class="text-button" href="/sessions">All sessions <Icon name="arrow" size={12} /></a>
    </div>
  {:else}
    <div class="scope-group">
      <div class="segmented" aria-label="Client">
        <button
          type="button"
          class:chosen={!applied.get('client')}
          aria-pressed={!applied.get('client')}
          onclick={() => pick((f) => (f.client = ''))}>All clients</button
        >{#each clientChoices as choice}<button
            type="button"
            class:chosen={applied.get('client') === choice.client}
            aria-pressed={applied.get('client') === choice.client}
            onclick={() => pick((f) => (f.client = choice.client))}>{choice.label}</button
          >{/each}
      </div>
    </div>
    <div class="scope-group">
      <select
        class="compact-select"
        aria-label="Session"
        value={applied.get('session') ?? ''}
        onchange={(e) => pick((f) => (f.session = e.currentTarget.value))}
        ><option value="">All sessions</option>{#each sessionChoices as choice (choice.id)}<option
            value={choice.id}
            >{short(choice.native_id)} · {providerName(choice.provider)} · {date(choice.last_seen)}
            {clock(choice.last_seen)}</option
          >{/each}</select
      >
    </div>
    <div class="scope-group">
      <div class="segmented" aria-label="Time">
        <button
          type="button"
          class:chosen={!applied.get('since') && !applied.get('from_ms')}
          aria-pressed={!applied.get('since') && !applied.get('from_ms')}
          onclick={() =>
            pick((f) => {
              f.since = '';
              f.from = '';
              f.until = '';
            })}
          title="No time limit">Any</button
        >{#each sinceChoices as choice}<button
            type="button"
            class:chosen={applied.get('since') === choice.since}
            aria-pressed={applied.get('since') === choice.since}
            onclick={() =>
              pick((f) => {
                f.since = choice.since;
                f.from = '';
                f.until = '';
              })}
            title={choice.title}>{choice.label}</button
          >{/each}
      </div>
    </div>
    <div class="scope-group end">
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
    </div>
  {/if}
{/snippet}

<svelte:window onkeydown={keydown} />
{#if selectionError}<div class="notice danger" role="alert">
    <Icon name="alert" size={14} /><span>Selected call: {selectionError}</span><button
      class="text-button"
      onclick={closeInspector}>Dismiss</button
    >
  </div>{/if}
<div class="traffic-workspace" class:with-inspector={!!selected}>
  <section class="panel list-panel traffic-panel" aria-label="Recorded calls">
    {#if !sessionId}
      <div class="request-tabs" aria-label="Quick filters">
        <button
          class:active={!applied.get('status') && !applied.get('has_tools')}
          aria-pressed={!applied.get('status') && !applied.get('has_tools')}
          onclick={() => quickFilter('')}>All calls</button
        >
        <button
          class:active={applied.get('status') === 'active'}
          aria-pressed={applied.get('status') === 'active'}
          onclick={() => quickFilter('active')}
          ><span class="status-dot live"></span> In progress</button
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
      <form
        onsubmit={(e) => {
          e.preventDefault();
          applyFilters();
        }}
        aria-label="Traffic filters"
      >
        <div class="filter-toolbar">
          <label class="search-input main-search"
            ><Icon name="search" /><input
              bind:this={searchInput}
              aria-label="Search traffic"
              placeholder="Search call ID, model, or tool…"
              maxlength="128"
              bind:value={filters.q}
            /><kbd>/</kbd></label
          ><select
            class="compact-select"
            aria-label="Provider"
            bind:value={filters.provider}
            onchange={() => {
              queueMicrotask(applyFilters);
            }}
            ><option value="">All providers</option><option value="anthropic">Anthropic</option
            ><option value="openai">OpenAI</option><option value="codex">Codex</option></select
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
          ><button class="button primary" type="submit"
            >Search <Icon name="arrow" size={14} /></button
          >
        </div>
        {#if moreFilters}<div class="advanced-filters" id="advanced-filters">
            <label
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
            ><label
              >Status<select bind:value={filters.status}
                ><option value="">All statuses</option><option value="active">In progress</option
                ><option value="finished">Finished</option><option value="errors">Errors</option
                ><option value="interrupted">Interrupted / cancelled</option></select
              ></label
            ><label
              >From · local time<input type="datetime-local" bind:value={filters.from} /></label
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
    <div class="list-content" class:busy={loading && calls.length > 0} aria-busy={loading}>
      {#if calls.length === 0 && retained.current.error && !loading}
        <div class="empty-state" role="alert">
          <div class="empty-icon"><Icon name="alert" size={26} /></div>
          <h2>
            {retained.current.error.kind === 'incompatible'
              ? 'This UI and the daemon disagree'
              : "Can't reach TGV"}
          </h2>
          <p>{retained.current.error.message}</p>
          <p class="muted">
            Nothing is shown because nothing could be read, not because nothing was recorded.
          </p>
          <button class="button" onclick={() => void invalidateAll()}
            ><Icon name="refresh" size={14} /> Retry</button
          >
        </div>
      {:else if calls.length === 0}
        <div class="empty-state" class:first-run={!filterCount && !sessionId && !after}>
          <div class="empty-icon">
            <Icon name={loading ? 'refresh' : filterCount ? 'search' : 'logs'} size={26} />
          </div>
          <h2>
            {loading
              ? 'Loading requests…'
              : filterCount || !!after
                ? 'No requests match these filters'
                : sessionId
                  ? 'No requests on this page'
                  : 'No recorded traffic'}
          </h2>
          <p>
            {loading
              ? 'Reading captured traffic from your gateway.'
              : filterCount || !!after
                ? 'Try a broader search or adjust your filters.'
                : sessionId
                  ? 'Refresh to check for new activity in this session.'
                  : 'Point a client at TGV or use a launch command below.'}
          </p>
          {#if !loading && (filterCount || !!after)}<button class="button" onclick={resetFilters}
              >Clear filters & return to first page</button
            >
          {:else if !loading && !sessionId}<div class="connect-box">
              <div class="connect-tabs" aria-label="Client to connect">
                {#each ['claude', 'codex', 'pi'] as client}<button
                    class:active={connectClient === client}
                    onclick={() => (connectClient = client)}
                    >{client === 'claude'
                      ? 'Claude Code'
                      : client === 'codex'
                        ? 'Codex'
                        : 'Pi'}</button
                  >{/each}
              </div>
              <div class="command-line">
                <Icon name="terminal" size={16} /><code>{connectionCommand}</code><CopyButton
                  text={connectionCommand}
                  label="Copy launch command"
                />
              </div>
            </div>
            <span class="empty-footnote"
              ><Icon name="shield" size={12} /> Uses your client’s existing sign-in</span
            >{/if}
        </div>
      {:else}
        <div class="attempt-list">
          {#each groups as group (group.id)}
            {#if sessionId}<div class="turn-heading">
                <Icon name="sessions" size={13} /><strong>{group.label}</strong><span
                  >{group.rows.length} call{group.rows.length === 1 ? '' : 's'}</span
                >{#if group.turn}<a
                    class="text-button turn-link"
                    href={`/turns/${encodeURIComponent(group.turn)}`}
                    >What this turn did <Icon name="arrow" size={12} /></a
                  >{/if}
              </div>{/if}
            <table class="requests-table">
              <thead
                ><tr
                  ><th class="time-column">Time</th><th>Call</th><th>Model / upstream</th><th
                    >Outcome / duration</th
                  ></tr
                ></thead
              >
              <tbody>
                {#each group.rows as c, index (c.id)}{@const badge = statusBadge(
                    callStatus(c),
                  )}{@const http = httpChip(c.status)}{@const stream =
                    streamChip(c)}{@const capture = captureChip(c)}{@const tokens = tokenSummary(
                    c.usage,
                  )}
                  <tr
                    class="call-row"
                    class:selected={selectedId === c.id}
                    onclick={(event) => selectRow(event, c.id)}
                  >
                    <td class="time-column mono" title={new Date(c.started_at).toLocaleString()}
                      >{clock(c.started_at)}{#if date(c.started_at) !== today}<small
                          >{date(c.started_at)}</small
                        >{/if}</td
                    >
                    <td
                      ><a
                        class="row-link mono"
                        id={`call-${c.id}`}
                        href={callHref(c.id)}
                        onclick={(event) => selectCall(event, c.id)}
                        aria-label={`Inspect call ${c.id}`}
                        aria-current={selectedId === c.id ? 'true' : undefined}>{short(c.id)}</a
                      >{#if !sessionId}{@const previous = group.rows[index - 1]}<small
                          class="mono session-line"
                          title={c.attribution.hint?.session ??
                            'No session identifier was reported'}
                          >{#if c.session}{@const session = c.session}<button
                              type="button"
                              class="session-filter"
                              title="Show only this session's calls"
                              aria-label={`Show only session ${short(c.attribution.hint?.session ?? session)}`}
                              onclick={() => pick((f) => (f.session = session))}
                              ><span class="session-dot tone-{sessionTone(session)}"
                              ></span>{#if !previous || previous.session !== c.session}{c
                                  .attribution.hint
                                  ? short(c.attribution.hint.session)
                                  : 'Session'}{/if}</button
                            >{:else if !previous || previous.session !== c.session}No session{/if}</small
                        >{/if}</td
                    >
                    <td
                      ><span class="model-line"
                        ><span
                          class="provider-mark tiny {c.provider}"
                          title={providerName(c.provider)}>{providerMark(c.provider)}</span
                        ><span class="model-name" title={c.model ?? title(c.endpoint)}
                          >{c.model ??
                            (isActive(c) ? 'Awaiting observation' : title(c.endpoint))}</span
                        ></span
                      >{#if c.tool_names.length}<span class="tool-chips"
                          >{#each c.tool_names.slice(0, 3) as name}<span class="chip tool-chip"
                              ><Icon name="tool" size={10} />{name}</span
                            >{/each}{#if c.tool_names.length > 3}<span class="chip"
                              >+{c.tool_names.length - 3}</span
                            >{/if}</span
                        >{/if}</td
                    >
                    <td
                      ><span class="badge {badge.tone}">{badge.label}</span><small
                        class="outcome-line"
                        ><span class="status-code {http.tone}" title={http.title}>{http.label}</span
                        ><span>{elapsed(duration(c))}</span>{#if tokens}<span title={tokens.title}
                            >{tokens.input} in · {tokens.output} out</span
                          >{/if}</small
                      >{#if stream.tone !== 'success' && stream.tone !== 'live'}<span
                          class="chip {stream.tone}"
                          title={stream.title}>{stream.label}</span
                        >{/if}{#if capture.tone === 'warning'}<span
                          class="chip warning"
                          title={capture.title}>{capture.label}</span
                        >{/if}</td
                    >
                  </tr>
                {/each}
              </tbody>
            </table>
          {/each}
        </div>
      {/if}
    </div>
    <div class="table-footer">
      <span
        >{num(calls.length)} call{calls.length === 1 ? '' : 's'}
        <span class="muted">· {sessionId ? 'Oldest first' : 'Newest first'}</span></span
      >
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
  {#if selected}{#key selected.id}<Inspector
        call={selected}
        close={closeInspector}
        onsession={showCallSession}
        onupdate={updateCall}
      />{/key}{/if}
</div>
