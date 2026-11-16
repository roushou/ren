<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import { page as location } from '$app/state';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { num, clock, parseSearchField, SEARCH_FIELDS, type SearchField } from '#lib/api.ts';
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
  const field = $derived(
    current ? current.field : parseSearchField(location.url.searchParams.get('field')),
  );
  /** Varied snippet widths for the outline, so its rows read as text. */
  const outlineWidths = ['82%', '64%', '91%', '58%', '76%', '69%'];
  const details = $derived(current?.details ?? new Map());
  const recordingsById = $derived(
    new Map((current?.recordings ?? []).map((item) => [item.id, item])),
  );
  const fieldOptions: SegmentedOption<SearchField | ''>[] = [
    { value: '', label: 'Everything' },
    ...SEARCH_FIELDS.map((field) => ({ value: field, label: searchFieldLabel(field) })),
  ];
  const fieldHref = (field: SearchField | null) => {
    const query = new URLSearchParams();
    if (q) query.set('q', q);
    if (field) query.set('field', field);
    return `/search?${query}`;
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

{#snippet scope()}
  <div class="scope-group">
    <span class="scope-title">Search</span>
    <span class="muted">Prompts, replies, and tool input and output · press / to search</span>
  </div>
{/snippet}

{#snippet searchForm(hero: boolean)}
  <form class="search-form" class:hero method="GET" action="/search" role="search">
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

<Toolbar {scope} />
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
        <h2>Search recorded history</h2>
        <p class="muted">
          Find what your agents' prompts, replies, and tool calls said: a command, a file path, an
          error message.
        </p>
        {@render searchForm(true)}
        <p class="search-hint muted">
          All words must match. Put a phrase in quotes. Punctuation separates words, so search
          <span class="mono">"install-vps"</span> as a phrase.
        </p>
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
      {:else if page?.hits.length}
        <ul class="search-hits">
          {#each page.hits as hit (`${hit.call}-${hit.field}`)}
            <li>
              <a class="search-hit" href={searchHitHref(hit)}>
                <span class="search-hit-meta">
                  {#if hit.recording && recordingsById.get(hit.recording)}<RecordingName
                      recording={recordingsById.get(hit.recording)!}
                      detail={details.get(hit.recording)}
                    />{:else if hit.recording}<span
                      class="recording-dot tone-{recordingTone(hit.recording)}"
                    ></span>{/if}
                  <span class="chip">{searchFieldLabel(hit.field)}</span>
                  <span class="muted">{date(hit.started_at)} {clock(hit.started_at)}</span>
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
          {/each}
        </ul>
      {:else}
        <div class="empty-state">
          <div class="empty-icon"><Icon name="search" size={25} /></div>
          <h2>No matches for “{q}”</h2>
          <p>Try fewer words, search everything, or put a phrase in quotes.</p>
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
