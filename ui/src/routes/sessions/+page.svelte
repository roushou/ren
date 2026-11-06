<script lang="ts">
  import { goto } from '$app/navigation';
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { num, short, clock, providerName, providerMark } from '#lib/api.ts';
  import { compact, date, span, sessionTone } from '#lib/display.ts';
  import Icon from '#lib/Icon.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);
  const snapshot = createSnapshot(() => data.sessions);
  const sessions = $derived(snapshot.current.value ?? []);
  let query = $state('');
  const visibleSessions = $derived(
    sessions.filter((s) =>
      `${s.provider} ${s.id} ${s.native_id}`.toLowerCase().includes(query.toLowerCase()),
    ),
  );
</script>

{#snippet scope()}
  <div class="scope-group">
    <strong>{sessions.length}{sessions.length === 100 ? '+' : ''}</strong>
    <span class="muted">reported sessions · client-reported grouping</span>
  </div>
  <label class="scope-group end search-input scope-search"
    ><Icon name="search" /><input
      aria-label="Filter sessions"
      placeholder="Search provider or session ID…"
      bind:value={query}
    /></label
  >
{/snippet}

<section class="panel list-panel sessions-panel">
  <div class="list-content">
    {#if visibleSessions.length}<div class="table-scroll">
        <table>
          <thead
            ><tr
              ><th>Session</th><th>Provider</th><th class="numeric">Requests</th><th class="numeric"
                >Turns</th
              ><th class="numeric">Errors</th><th class="numeric">Tokens in · out</th><th
                class="numeric">Active for</th
              ><th>Last activity</th><th></th></tr
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
                    ><span class="session-dot tone-{sessionTone(s.id)}"></span><span
                      ><strong class="mono" title="The session ID your client reported"
                        >{short(s.native_id)}</strong
                      ><small class="mono" title={`TGV's own ID for this session: ${s.id}`}
                        >TGV {short(s.id)}</small
                      ></span
                    ></a
                  ></td
                ><td
                  ><span class="provider-cell"
                    ><span class="provider-mark {s.provider}">{providerMark(s.provider)}</span
                    >{providerName(s.provider)}</span
                  ></td
                ><td class="mono numeric">{num(s.call_count)}</td><td class="mono numeric"
                  >{num(s.turn_count)}</td
                ><td
                  class="mono numeric"
                  class:error-count={!!s.error_count}
                  class:zero={s.error_count === 0}>{num(s.error_count)}</td
                ><td
                  class="mono numeric"
                  title="Provider reported; input includes cached context. Not subscription allowance or charges."
                  >{s.tokens.input === null ? '—' : compact(s.tokens.input)} · {s.tokens.output ===
                  null
                    ? '—'
                    : compact(s.tokens.output)}</td
                ><td class="mono numeric">{span(s.last_seen - s.first_seen)}</td><td
                  ><span>{date(s.last_seen)}</span><small>{clock(s.last_seen)}</small></td
                ><td
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
