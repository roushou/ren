<script lang="ts">
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { num, clock, SEARCH_FIELDS, type SearchField, type SearchHit } from '#lib/api.ts';
  import { date, sessionTone } from '#lib/display.ts';
  import { searchFieldLabel, snippetParts } from '#lib/activity.ts';
  import Icon from '#lib/Icon.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);
  const snapshot = createSnapshot(() => data.search);
  const current = $derived(snapshot.current.value);
  const page = $derived(current?.page ?? null);
  const fieldHref = (field: SearchField | null) => {
    const query = new URLSearchParams();
    if (current?.q) query.set('q', current.q);
    if (field) query.set('field', field);
    return `/search?${query}`;
  };
  const hitHref = (hit: SearchHit) =>
    hit.turn
      ? `/turns/${encodeURIComponent(hit.turn)}`
      : hit.session
        ? `/sessions/${encodeURIComponent(hit.session)}?call=${encodeURIComponent(hit.call)}`
        : `/traffic?call=${encodeURIComponent(hit.call)}`;
</script>

{#snippet scope()}
  <form class="scope-group search-input scope-search" method="GET" action="/search">
    <Icon name="search" />
    <input
      name="q"
      aria-label="Search history"
      placeholder={'Words, or "a phrase", in prompts, replies, and tools…'}
      value={current?.q ?? ''}
    />
    {#if current?.field}<input type="hidden" name="field" value={current.field} />{/if}
  </form>
  <nav class="scope-group segmented" aria-label="Field">
    <a
      href={fieldHref(null)}
      class:chosen={!current?.field}
      aria-current={!current?.field ? 'page' : undefined}>All</a
    >
    {#each SEARCH_FIELDS as field (field)}<a
        href={fieldHref(field)}
        class:chosen={current?.field === field}
        aria-current={current?.field === field ? 'page' : undefined}>{searchFieldLabel(field)}</a
      >{/each}
  </nav>
{/snippet}

<section class="panel list-panel">
  <div class="list-content">
    {#if !current}
      <div class="empty-state">
        <div class="empty-icon"><Icon name="alert" size={25} /></div>
        <h2>Search could not run</h2>
        <p>{snapshot.current.error?.message ?? 'Loading…'}</p>
      </div>
    {:else if !page}
      <div class="empty-state">
        <div class="empty-icon"><Icon name="search" size={25} /></div>
        <h2>Search recorded history</h2>
        <p>
          Find what prompts, replies, and tool calls said: a command, a file path, an error. All
          words must match; quote a phrase. Punctuation separates words, so search
          <span class="mono">"install-vps"</span> as a phrase.
        </p>
      </div>
    {:else if page.hits.length}
      <ul class="search-hits">
        {#each page.hits as hit (`${hit.call}-${hit.field}`)}
          <li>
            <a class="search-hit" href={hitHref(hit)}>
              <span class="search-hit-meta">
                {#if hit.session}<span class="session-dot tone-{sessionTone(hit.session)}"
                  ></span>{/if}
                <span class="chip">{searchFieldLabel(hit.field)}</span>
                <span class="muted">{date(hit.started_at)} {clock(hit.started_at)}</span>
                <span class="muted end"
                  >{hit.turn ? 'Open turn' : 'Open call'}
                  <Icon name="arrow" size={12} /></span
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
        <h2>No matches</h2>
        <p>Try fewer words, another field, or a phrase in quotes.</p>
      </div>
    {/if}
  </div>
  {#if page}<div class="table-footer">
      <span
        >{num(page.hits.length)}{page.hits.length === 50 ? '+' : ''} matches, newest first · {num(
          page.indexed_calls,
        )} calls indexed</span
      >
    </div>{/if}
</section>
