<script lang="ts">
  import { Clock } from '#lib/time.ts';
  import Toolbar from '#lib/Toolbar.svelte';
  import { page as location } from '$app/state';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import {
    num,
    parseSearchField,
    searchFields,
    type SearchField,
    type SearchHit,
  } from '#lib/api.ts';
  import { SearchResults } from '#lib/search.ts';
  import { localDayStart } from '#lib/usage.ts';
  import { date, recordingTone } from '#lib/display.ts';
  import { searchFieldLabel, searchHitHref, snippetParts } from '#lib/activity.ts';
  import Icon from '#lib/Icon.svelte';
  import RecordingName from '#lib/RecordingName.svelte';
  import Segmented, { type SegmentedOption } from '#lib/Segmented.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  const arrival = createArrival(() => data.search);
  const arrived = $derived(arrival.current);
  /** The search is on its way: the page draws its outline. */
  const pending = $derived(arrived.state === 'pending');
  const snapshot = $derived(arrived.state === 'ready' ? arrived.snapshot : null);
  const current = $derived(snapshot?.value ?? null);
  const page = $derived(current?.page ?? null);
  /** What was searched for, from the URL until the search is read. */
  const q = $derived(current?.q ?? (location.url.searchParams.get('q') ?? '').trim());
  /** The project the search is narrowed to, kept across searches. */
  const project = $derived(location.url.searchParams.get('project') || null);
  const field = $derived(
    current ? current.field : parseSearchField(location.url.searchParams.get('field')),
  );
  /** Varied snippet widths for the outline, so its rows read as text. */
  const outlineWidths = ['82%', '64%', '91%', '58%', '76%', '69%'];
  const details = $derived(current?.details ?? new Map());
  const recordingsById = $derived(
    new Map((current?.recordings ?? []).map((item) => [item.id, item])),
  );
  /** The page's hits grouped, and counted for the side column. */
  const results = $derived(page ? new SearchResults(page.hits, localDayStart) : null);
  /** Hits grouped by recording, or one flat list. */
  let grouped = $state(true);
  const groupOptions: SegmentedOption<'recording' | 'flat'>[] = [
    { value: 'recording', label: 'By session' },
    { value: 'flat', label: 'Flat' },
  ];
  const fieldOptions: SegmentedOption<SearchField | ''>[] = [
    { value: '', label: 'Everything' },
    ...searchFields.map((field) => ({ value: field, label: searchFieldLabel(field) })),
  ];
  const fieldHref = (field: SearchField | null) => {
    const query = new URLSearchParams();
    if (q) query.set('q', q);
    if (field) query.set('field', field);
    if (project) query.set('project', project);
    return `/monitor/search?${query}`;
  };
  /** The input takes the keyboard when the page opens, and on `/`. */
  let input = $state<HTMLInputElement>();
  $effect(() => {
    if (!q) input?.focus();
  });
  function onKeydown(event: KeyboardEvent) {
    const typing =
      event.target instanceof HTMLElement &&
      (event.target.matches('input,textarea,select') || event.target.isContentEditable);
    if (event.key === '/' && !typing) {
      event.preventDefault();
      input?.focus();
      input?.select();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet hitRow(hit: SearchHit, named: boolean)}
  <li>
    <a class="search-hit" href={searchHitHref(hit)}>
      <span class="search-hit-meta">
        {#if named}{#if hit.recording && recordingsById.get(hit.recording)}<RecordingName
              recording={recordingsById.get(hit.recording)!}
              detail={details.get(hit.recording)}
            />{:else if hit.recording}<span
              class="recording-dot tone-{recordingTone(hit.recording)}"
            ></span>{/if}{/if}
        <span class="field-name field-{hit.field}">{searchFieldLabel(hit.field)}</span>
        <span class="muted">{date(hit.started_at)} {Clock.second(hit.started_at)}</span>
        <span class="muted end"
          >{hit.turn ? 'Open turn' : 'Open call'} <Icon name="arrow" size={12} /></span
        >
      </span>
      <span class="search-snippet"
        >{#each snippetParts(hit.snippet) as part, index (index)}{#if part.match}<mark
              >{part.text}</mark
            >{:else}{part.text}{/if}{/each}</span
      >
    </a>
  </li>
{/snippet}

{#snippet searchForm(hero: boolean)}
  <form class="search-form" class:hero method="GET" action="/monitor/search" role="search">
    {#if project}<input type="hidden" name="project" value={project} />{/if}
    <label class="search-box">
      <Icon name="search" size={hero ? 18 : 15} />
      <input
        bind:this={input}
        name="q"
        type="search"
        aria-label="Search recorded history"
        placeholder={'Search prompts, replies, commands, file paths, errors…'}
        value={q}
        autocomplete="off"
        spellcheck="false"
      />
      {#if field}<input type="hidden" name="field" value={field} />{/if}
      <button class="button primary search-submit" type="submit">Search</button>
    </label>
    <Segmented
      label="Search in"
      class="search-fields"
      options={fieldOptions}
      value={field ?? ''}
      href={(field) => fieldHref(field || null)}
    />
  </form>
{/snippet}

<Toolbar />
<section class="panel list-panel search-panel">
  {#if !current && !pending}
    <div class="list-content">
      <div class="empty-state">
        <div class="empty-icon"><Icon name="alert" size={25} /></div>
        <h2>Search could not run</h2>
        <p>{snapshot?.error?.message ?? 'Loading…'}</p>
      </div>
    </div>
  {:else if pending ? !q : !page}
    <div class="list-content">
      <div class="search-hero">
        {@render searchForm(true)}
      </div>
    </div>
  {:else}
    <div class="search-bar">{@render searchForm(false)}</div>
    <div class="list-content" aria-busy={pending}>
      {#if pending}
        <ul class="search-hits">
          {#each outlineWidths as width, index (index)}
            <li>
              <div class="search-hit">
                <span class="search-hit-meta">
                  <Skeleton width="9em" /><Skeleton width="4em" /><Skeleton width="7em" />
                </span>
                <span class="search-snippet"><Skeleton {width} /></span>
              </div>
            </li>
          {/each}
        </ul>
      {:else if page?.hits.length && results}
        <div class="results">
          <div class="hits">
            <div class="hits-bar">
              <span class="muted"
                >{num(page.hits.length)}{page.hits.length === 50 ? '+' : ''} hits on this page · in {results
                  .groups.length} recording{results.groups.length === 1 ? '' : 's'} · newest first</span
              >
              <Segmented
                label="Group hits"
                options={groupOptions}
                value={grouped ? 'recording' : 'flat'}
                onchange={(value) => (grouped = value === 'recording')}
              />
            </div>
            {#if grouped}
              {#each results.groups as group (group.recording ?? '')}
                <section class="group">
                  <div class="group-head">
                    {#if group.recording && recordingsById.get(group.recording)}<RecordingName
                        recording={recordingsById.get(group.recording)!}
                        detail={details.get(group.recording)}
                      />{:else if group.recording}<span
                        class="recording-dot tone-{recordingTone(group.recording)}"
                      ></span><span class="mono">{group.recording.slice(0, 8)}</span>{:else}<span
                        class="muted">No session</span
                      >{/if}
                    <svg
                      class="ticks"
                      viewBox="0 0 100 10"
                      preserveAspectRatio="none"
                      aria-label="Where the hits fall in the session"
                      >{#each group.ticks as tick, index (index)}<rect
                          class="tick field-{tick.field}"
                          x={tick.at * 99}
                          y="0"
                          width="1"
                          height="10"
                        />{/each}</svg
                    >
                    <span class="mono muted">{group.hits.length}</span>
                  </div>
                  <ul class="search-hits">
                    {#each group.hits as hit (`${hit.call}-${hit.field}`)}{@render hitRow(
                        hit,
                        false,
                      )}{/each}
                  </ul>
                </section>
              {/each}
            {:else}
              <ul class="search-hits">
                {#each page.hits as hit (`${hit.call}-${hit.field}`)}{@render hitRow(
                    hit,
                    true,
                  )}{/each}
              </ul>
            {/if}
          </div>
          <aside class="facets" aria-label="On this page">
            <section>
              <h3>When · by day</h3>
              <svg
                class="days"
                viewBox="0 0 {Math.max(7, results.days.length) * 10} 40"
                preserveAspectRatio="none"
                aria-hidden="true"
                >{#each results.days as day, index (day.key)}<rect
                    x={index * 10 + 1.5}
                    y={40 - day.share * 38}
                    width="7"
                    height={day.share * 38}><title>{date(day.key)}: {day.count}</title></rect
                  >{/each}</svg
              >
              <p class="axis muted mono">
                <span>{date(results.days[0]!.key)}</span><span
                  >{date(results.days.at(-1)!.key)}</span
                >
              </p>
            </section>
            <section>
              <h3>Where it matched</h3>
              {#each results.fields as field (field.key)}
                <div class="facet">
                  <span class="field-name field-{field.key}">{searchFieldLabel(field.key)}</span>
                  <svg viewBox="0 0 100 6" preserveAspectRatio="none" aria-hidden="true"
                    ><rect
                      class="facet-bar field-{field.key}"
                      width={field.share * 100}
                      height="6"
                    /></svg
                  >
                  <span class="mono">{field.count}</span>
                </div>
              {/each}
            </section>
            <section>
              <h3>Sessions</h3>
              {#each results.recordings as one (one.key ?? '')}
                <div class="facet">
                  <span class="facet-name"
                    >{one.key
                      ? (details.get(one.key)?.prompt?.text.split('\n')[0] ?? one.key.slice(0, 8))
                      : 'No session'}</span
                  >
                  <svg viewBox="0 0 100 6" preserveAspectRatio="none" aria-hidden="true"
                    ><rect class="facet-bar" width={one.share * 100} height="6" /></svg
                  >
                  <span class="mono">{one.count}</span>
                </div>
              {/each}
            </section>
          </aside>
        </div>
      {:else}
        <div class="empty-state">
          <div class="empty-icon"><Icon name="search" size={25} /></div>
          <h2>No matches for “{q}”</h2>
        </div>
      {/if}
    </div>
    <div class="table-footer">
      <span
        >{#if page}{num(page.hits.length)}{page.hits.length === 50 ? '+' : ''} matches, newest first ·
          {num(page.indexed_calls)} calls indexed{:else}<Skeleton width="16em" />{/if}</span
      >
    </div>
  {/if}
</section>

<style>
  .results {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 300px;
    align-items: start;
  }
  .hits {
    min-width: 0;
  }
  .hits-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  .group-head {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 160px auto;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--line);
    background: var(--surface-subtle);
    font-size: 12px;
  }
  .ticks {
    width: 100%;
    height: 10px;
    background: var(--surface-inset);
  }
  .tick {
    fill: var(--muted);
  }
  .field-name {
    padding: 0 5px;
    border: 1px solid currentColor;
    border-radius: 2px;
    font-family: var(--mono);
    font-size: 10.5px;
  }
  .field-prompt {
    color: var(--accent);
    fill: var(--accent);
  }
  .field-reply {
    color: var(--chart);
    fill: var(--chart);
  }
  .field-tool_input {
    color: var(--role-tool);
    fill: var(--role-tool);
  }
  .field-tool_output {
    color: var(--success);
    fill: var(--success);
  }
  .facets {
    position: sticky;
    top: 0;
    border-left: 1px solid var(--line);
    font-size: 12px;
  }
  .facets section {
    padding: 12px 14px;
    border-bottom: 1px solid var(--line);
  }
  .facets h3 {
    margin: 0 0 8px;
    color: var(--muted);
    font-size: 10.5px;
    font-weight: 500;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .days {
    display: block;
    width: 100%;
    height: 40px;
  }
  .days rect {
    fill: var(--chart);
  }
  .axis {
    display: flex;
    justify-content: space-between;
    margin: 4px 0 0;
    font-size: 10.5px;
  }
  .facet {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 70px 28px;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
  }
  .facet .field-name {
    justify-self: start;
  }
  .facet-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .facet svg {
    width: 100%;
    height: 6px;
    background: var(--surface-inset);
  }
  .facet-bar {
    fill: var(--muted);
  }
  .facet span.mono {
    text-align: right;
  }
  @media (max-width: 960px) {
    .results {
      grid-template-columns: minmax(0, 1fr);
    }
    .facets {
      display: none;
    }
  }
</style>
