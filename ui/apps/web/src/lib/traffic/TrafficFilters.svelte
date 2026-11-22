<!--
  The traffic page's filter form. It starts from the filters the address
  applies, and its parent draws it afresh (`{#key}`) whenever they change, so
  what is typed here is a draft until it is applied through the address.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import Glyph from '../Glyph.svelte';
  import Icon from '../Icon.svelte';
  import Select, { type SelectOption } from '../Select.svelte';
  import Segmented, { type SegmentedOption } from '../Segmented.svelte';
  import { short, providerName, type RecordingView } from '../api.ts';
  import { date } from '../display.ts';
  import { Clock } from '../time.ts';
  import {
    AppliedFilter,
    Filters,
    QuickFilter,
    clientChoices,
    emptyFilters,
    filtersFromParams,
    providerChoices,
    statusChoices,
    trafficParams,
  } from '../traffic.ts';

  let {
    applied,
    live,
    recordings,
    hidden,
    more = $bindable(false),
  }: {
    /** The filters the address applies. */
    applied: URLSearchParams;
    /** How fresh the listing is. */
    live: string;
    /** The recordings the session picker offers. */
    recordings: RecordingView[];
    /** How many calls "model calls only" hides, or null when not hiding. */
    hidden: number | null;
    /** Whether the advanced filters are open; kept by the parent across drafts. */
    more?: boolean;
  } = $props();

  // The parent draws the form afresh when the applied filters change.
  let filters = $state(untrack(() => filtersFromParams(applied)));
  let error = $state('');
  let searchInput = $state<HTMLInputElement>();
  const current = $derived(filtersFromParams(applied));
  const filterCount = $derived([...applied.keys()].length);
  const chips = $derived(AppliedFilter.list(applied));

  function apply() {
    try {
      const query = trafficParams(filters);
      error = '';
      void goto(`${page.url.pathname}${query ? `?${query}` : ''}`, { reset: false });
    } catch (failure) {
      error = failure instanceof Error ? failure.message : 'Invalid filters.';
    }
  }
  function reset() {
    filters = emptyFilters();
    apply();
  }
  function quick(choice: QuickFilter) {
    filters = QuickFilter.apply(choice, filters);
    apply();
  }
  function remove(filter: AppliedFilter) {
    filters = Filters.without(filters, filter);
    apply();
  }
  /** Narrow to one choice and apply at once. */
  export function pick(change: (filters: Filters) => void) {
    change(filters);
    apply();
  }
  /** Put the cursor in the search box (`/`). */
  export function focusSearch() {
    searchInput?.focus();
  }

  function recordingName(id: string) {
    const found = recordings.find((item) => item.id === id);
    return found ? short(found.native_id) : short(id);
  }
  const recordingOptions = $derived<SelectOption<string>[]>([
    { value: '', label: 'All sessions' },
    ...recordings.map((choice) => ({
      value: choice.id,
      label: short(choice.native_id),
      hint: `${providerName(choice.provider)} · ${date(choice.last_seen)} ${Clock.second(choice.last_seen)}`,
    })),
  ]);
  const clientOptions: SegmentedOption<string>[] = [
    { value: '', label: 'All clients' },
    ...clientChoices.map((choice) => ({ value: choice.client, label: choice.label })),
  ];
  const context = { session: recordingName };
</script>

<form
  onsubmit={(e) => {
    e.preventDefault();
    apply();
  }}
  aria-label="Traffic filters"
>
  <div class="traffic-bar">
    <div class="request-tabs" aria-label="Quick filters">
      <button
        class:active={QuickFilter.chosen('all', current)}
        aria-pressed={QuickFilter.chosen('all', current)}
        onclick={() => quick('all')}>All calls</button
      >
      <button
        class:active={QuickFilter.chosen('active', current)}
        aria-pressed={QuickFilter.chosen('active', current)}
        onclick={() => quick('active')}><Glyph mark="responding" /> In progress</button
      >
      <button
        class:active={QuickFilter.chosen('errors', current)}
        aria-pressed={QuickFilter.chosen('errors', current)}
        onclick={() => quick('errors')}>Errors</button
      >
      <button
        class:active={QuickFilter.chosen('tools', current)}
        aria-pressed={QuickFilter.chosen('tools', current)}
        onclick={() => quick('tools')}>Tool requests</button
      >
      <span class="table-update" role="status">{live}</span>
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
        class:chosen={more}
        aria-expanded={more}
        aria-controls="advanced-filters"
        onclick={() => (more = !more)}
        ><Icon name="filter" size={14} /> Filters {#if filterCount}<span class="count-pill"
            >{filterCount}</span
          >{/if}</button
      >
    </div>
  </div>
  {#if more}<div class="advanced-filters" id="advanced-filters">
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
          options={providerChoices}
          bind:value={filters.provider}
          onchange={apply}
        />
      </div>
      <label
        class="toggle-control model-only"
        title="Hide token counts, model listings, and WebSocket handshakes"
        ><input
          type="checkbox"
          checked={current.kind === 'model'}
          onchange={(e) => pick((f) => (f.kind = e.currentTarget.checked ? 'model' : 'all'))}
        /><span class="switch"></span><span
          >Model calls only{#if hidden}<span class="muted hidden-count"
              >{hidden >= 100 ? '100+' : hidden} hidden</span
            >{/if}</span
        ></label
      >
      <label
        >User<input placeholder="Named user id…" maxlength="64" bind:value={filters.user} /></label
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
          options={statusChoices}
          bind:value={filters.status}
        />
      </div>
      <label>From · local time<input type="datetime-local" bind:value={filters.from} /></label
      ><label>Until · local time<input type="datetime-local" bind:value={filters.until} /></label
      ><label class="check-label"
        ><input type="checkbox" bind:checked={filters.has_tools} /> Has observed tool calls</label
      >
      <div class="filter-note">
        <button class="button small" type="button" onclick={reset}>Reset</button><button
          class="button primary small"
          type="submit">Apply filters</button
        >
      </div>
    </div>{/if}
  {#if filterCount}<div class="filter-chips">
      {#each chips as filter}<button
          type="button"
          class="filter-chip"
          onclick={() => remove(filter)}
          aria-label={`Remove ${AppliedFilter.label(filter, context)}`}
          >{AppliedFilter.label(filter, context)}<Icon name="close" size={12} /></button
        >{/each}<button type="button" class="text-button" onclick={reset}>Clear all</button>
    </div>{/if}
  {#if error}<div class="inline-error" role="alert">
      <Icon name="alert" size={14} />{error}
    </div>{/if}
</form>

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
</style>
