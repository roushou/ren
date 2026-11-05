<script lang="ts">
  import { onDestroy } from 'svelte';
  import { goto } from '$app/navigation';
  import { page, navigating } from '$app/state';
  import { getWorkbench } from './workbench.svelte.ts';
  import { createSnapshot } from './snapshot.svelte.ts';
  import type { Snapshot } from './snapshots.ts';
  import type { TrafficData } from './loaders.ts';
  import { CallId, type CallView, type SessionId } from './api.ts';
  import { num, short, title, clock, providerName, providerMark } from './api.ts';
  import {
    isActive,
    callStatus,
    statusBadge,
    duration,
    elapsed,
    date,
    transportState,
  } from './display.ts';
  import { emptyFilters, trafficParams, filtersFromParams, selectedCall } from './traffic.ts';
  import { LiveFollow } from './live.ts';
  import Icon from './Icon.svelte';
  import Inspector from './Inspector.svelte';
  import CopyButton from './CopyButton.svelte';
  let { snapshot, sessionId }: { snapshot: Snapshot<TrafficData>; sessionId: SessionId | null } =
    $props();
  const workbench = getWorkbench();
  const retained = createSnapshot(() => snapshot);
  const calls = $derived(retained.current.value?.calls ?? []);
  const nextAfter = $derived(retained.current.value?.next ?? null);
  const session = $derived(retained.current.value?.session);
  const updated = $derived(retained.current.updated);
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
  const linkedCall = $derived(page.url.searchParams.get('call'));
  const selectedId = $derived(
    linkedCall !== null
      ? CallId.parse(linkedCall)
      : page.url.searchParams.get('inspect') === 'off'
        ? null
        : calls[0]?.id,
  );
  let observed = $state<CallView | undefined>();
  const offPage = $derived(retained.current.value?.selection);
  const selected = $derived(
    selectedCall(
      selectedId,
      calls,
      observed,
      offPage?.kind === 'loaded' ? offPage.call : undefined,
    ),
  );
  const selectionError = $derived(
    linkedCall !== null && !selected && snapshot.value?.selection.kind === 'failed'
      ? snapshot.value.selection.message
      : '',
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
  const groups = $derived.by(() => {
    if (!sessionId) return [{ id: 'recent', label: 'Recent calls', rows: calls }];
    const map = new Map<string, { id: string; label: string; rows: CallView[] }>();
    for (const call of calls) {
      const id = call.turn ?? `unassigned-${call.id}`;
      let group = map.get(id);
      if (!group) {
        group = { id, label: call.turn ? `Turn ${short(call.turn)}` : 'Turn unassigned', rows: [] };
        map.set(id, group);
      }
      group.rows.push(call);
    }
    return [...map.values()];
  });
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
  function selectCall(event: MouseEvent, id: CallId) {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)
      return;
    event.preventDefault();
    void goto(callHref(id), {
      reset: false,
      state: page.state,
      persistState: true,
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
    else if (['q', 'provider', 'model', 'tool', 'status'].includes(key))
      filters[key as 'q' | 'provider' | 'model' | 'tool' | 'status'] = '';
    applyFilters();
  }
  function chipLabel(key: string, value: string) {
    if (key === 'from_ms' || key === 'until_ms')
      return `${key === 'from_ms' ? 'From' : 'Until'}: ${new Date(Number(value)).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}`;
    return key === 'has_tools'
      ? 'Has tool calls'
      : `${key === 'q' ? 'Search' : title(key)}: ${value}`;
  }
  function keydown(event: KeyboardEvent) {
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
    {:else}<div class="list-toolbar">
        <h2>
          <span class="provider-mark {session?.provider ?? ''}"
            >{providerMark(session?.provider ?? '')}</span
          >{providerName(session?.provider ?? 'Client')}
          <span class="mono muted">{short(session?.native_id)}</span>
        </h2>
        <button class="text-button" onclick={() => goto('/sessions')}
          >All sessions <Icon name="arrow" size={14} /></button
        >
      </div>{/if}
    <div class="list-content">
      {#if calls.length === 0}
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
                >
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
                {#each group.rows as c (c.id)}{@const badge = statusBadge(callStatus(c))}
                  <tr class:selected={selectedId === c.id}>
                    <td class="time-column mono" title={new Date(c.started_at).toLocaleString()}
                      >{clock(c.started_at)}<small>{date(c.started_at)}</small></td
                    >
                    <td
                      ><a
                        class="row-link mono"
                        id={`call-${c.id}`}
                        href={callHref(c.id)}
                        onclick={(event) => selectCall(event, c.id)}
                        aria-label={`Inspect call ${c.id}`}
                        aria-current={selectedId === c.id ? 'true' : undefined}>{short(c.id)}</a
                      ><small
                        class="mono"
                        title={c.attribution.hint?.session ?? 'No session identifier was reported'}
                        >{c.attribution.hint
                          ? short(c.attribution.hint.session)
                          : 'Unattributed'}</small
                      ></td
                    >
                    <td
                      ><span class="model-name" title={c.model ?? title(c.endpoint)}
                        >{c.model ?? (isActive(c) ? 'Awaiting observation' : 'Not observed')}</span
                      ><small>{providerName(c.provider)}</small>{#if c.tool_names.length}<small
                          class="tool-summary"
                          ><Icon name="tool" size={11} />{c.tool_names.length} tool request{c
                            .tool_names.length === 1
                            ? ''
                            : 's'}</small
                        >{/if}</td
                    >
                    <td
                      ><span class="badge {badge.tone}">{badge.label}</span><small
                        class="transport-label"
                        class:warning={c.terminal === 'interrupted' || c.terminal === 'cancelled'}
                        >{elapsed(duration(c))} · {transportState(c)}</small
                      >{#if !c.capture_complete && !isActive(c)}<small class="capture-note"
                          >Partial capture</small
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
