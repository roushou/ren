<script lang="ts">
  import { goto } from '$app/navigation';
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { num, short, clock, providerName, providerMark } from '#lib/api.ts';
  import { date } from '#lib/display.ts';
  import Icon from '#lib/Icon.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  const snapshot = createSnapshot(() => data.sessions);
  const sessions = $derived(snapshot.current.value ?? []);
  let query = $state('');
  const visibleSessions = $derived(
    sessions.filter((s) =>
      `${s.provider} ${s.id} ${s.native_id}`.toLowerCase().includes(query.toLowerCase()),
    ),
  );
</script>

<section class="panel list-panel sessions-panel">
  <div class="list-toolbar">
    <div>
      <h2>
        Reported sessions <span class="count-pill"
          >{sessions.length}{sessions.length === 100 ? '+' : ''}</span
        >
      </h2>
    </div>
    <label class="search-input"
      ><Icon name="search" /><input
        aria-label="Filter sessions"
        placeholder="Search provider or session ID…"
        bind:value={query}
      /></label
    >
  </div>
  <div class="list-content">
    {#if visibleSessions.length}<div class="table-scroll">
        <table>
          <thead
            ><tr
              ><th>Session</th><th>Provider</th><th>Requests</th><th>Turns</th><th>Last activity</th
              ><th></th></tr
            ></thead
          ><tbody
            >{#each visibleSessions as s (s.id)}<tr
                class="call-row"
                onclick={(event) => {
                  if (event.target instanceof Element && event.target.closest('a,button')) return;
                  if (window.getSelection()?.toString()) return;
                  void goto(`/sessions/${encodeURIComponent(s.id)}`);
                }}
                ><td
                  ><a class="row-link" href={`/sessions/${encodeURIComponent(s.id)}`}
                    ><Icon name="sessions" /><span
                      ><strong class="mono">{short(s.native_id)}</strong><small class="mono"
                        >{short(s.id)}</small
                      ></span
                    ></a
                  ></td
                ><td
                  ><span class="provider-cell"
                    ><span class="provider-mark {s.provider}">{providerMark(s.provider)}</span
                    >{providerName(s.provider)}</span
                  ></td
                ><td class="mono">{num(s.call_count)}</td><td class="mono">{num(s.turn_count)}</td
                ><td><span>{date(s.last_seen)}</span><small>{clock(s.last_seen)}</small></td><td
                  ><a
                    class="icon-button"
                    aria-label={`Open session ${s.id}`}
                    href={`/sessions/${encodeURIComponent(s.id)}`}><Icon name="chevron" /></a
                  ></td
                ></tr
              >{/each}</tbody
          >
        </table>
      </div>
    {:else}<div class="empty-state">
        <div class="empty-icon"><Icon name="sessions" size={25} /></div>
        <h2>{query ? 'No matching sessions' : 'No reported sessions'}</h2>
        <p>
          {query
            ? 'Try a different provider or session ID.'
            : 'Sessions appear when clients send session identifiers through TGV.'}
        </p>
        {#if query}<button class="button" onclick={() => (query = '')}>Clear search</button
          >{:else}<button class="button" onclick={() => goto('/traffic')}
            >Go to request logs <Icon name="arrow" size={14} /></button
          >{/if}
      </div>{/if}
  </div>
  <div class="table-footer">
    <span>Latest {sessions.length} recorded sessions · client-reported grouping</span>
  </div>
</section>
