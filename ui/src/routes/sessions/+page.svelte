<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { num, short, clock, providerName, providerMark } from '#lib/api.ts';
  import { compactOrDash, span } from '#lib/display.ts';
  import { localDayStart } from '#lib/usage.ts';
  import { ago } from '#lib/overview.ts';
  import { SessionList, type SessionRow } from '#lib/sessions.ts';
  import { Board } from '#lib/board.ts';
  import Icon from '#lib/Icon.svelte';
  import StateGlyph from '#lib/StateGlyph.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);
  const snapshot = createSnapshot(() => data.sessions);
  const current = $derived(snapshot.current.value);
  const sessions = $derived(current?.sessions ?? []);
  let query = $state('');
  // Relative times tick between refreshes.
  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => (now = Date.now()), 15_000);
    return () => clearInterval(timer);
  });
  const list = $derived(
    new SessionList({
      sessions,
      details: current?.details ?? new Map(),
      open: current?.open ?? [],
      now,
      query,
      dayStart: localDayStart,
    }),
  );
  const shown = $derived(list.groups.reduce((sum, group) => sum + group.rows.length, 0));
  const sessionHref = (id: string) => `/sessions/${encodeURIComponent(id)}`;
  function open(event: MouseEvent, row: SessionRow) {
    if (event.target instanceof Element && event.target.closest('a,button,summary')) return;
    if (window.getSelection()?.toString()) return;
    void goto(sessionHref(row.session.id));
  }
</script>

{#snippet scope()}
  <div class="scope-group">
    <strong>{sessions.length}{sessions.length === 100 ? '+' : ''}</strong>
    <span class="muted">sessions · grouped by the IDs clients report</span>
  </div>
  <label class="scope-group end search-input scope-search"
    ><Icon name="search" /><input
      aria-label="Filter sessions"
      placeholder="Search prompt, folder, client, or ID…"
      bind:value={query}
    /></label
  >
{/snippet}

{#snippet bar(share: number)}
  <svg class="size-bar" viewBox="0 0 100 4" preserveAspectRatio="none" aria-hidden="true"
    ><rect x="0" y="0" height="4" width={Math.max(share > 0 ? 1.5 : 0, share * 100)} /></svg
  >
{/snippet}

{#snippet columns()}
  <colgroup
    ><col class="col-session" /><col class="col-now" /><col class="col-requests" /><col
      class="col-turns"
    /><col class="col-tokens" /><col class="col-active" /></colgroup
  >
{/snippet}

{#snippet row(row: SessionRow)}
  {@const session = row.session}
  <tr class="call-row session-row" onclick={(event) => open(event, row)}>
    <!-- The edge sits on the cell: on a row, its ::before would become a cell of its own. -->
    <td class="session-cell state-edge {row.now.kind === 'open' ? `state-${row.now.mark}` : ''}">
      <a class="row-link session-link" href={sessionHref(session.id)}>
        <span class="session-title">
          <span class="harness-mark harness-{row.harness ?? 'unknown'}"></span>
          <strong>{row.client}</strong>
          {#if row.detail?.prompt}<span class="session-prompt">{row.detail.prompt.text}</span
            >{:else}<span class="session-prompt muted">no prompt yet</span>{/if}
        </span>
        <span class="session-meta mono">
          {#if row.detail?.workdir}<span class="session-folder" title={row.detail.workdir}
              >{Board.folder(row.detail.workdir)}</span
            >{/if}
          <span title="The session ID your client reported">{short(session.native_id)}</span>
          <span class="provider-mark tiny {session.provider}" title={providerName(session.provider)}
            >{providerMark(session.provider)}</span
          >
          {#if session.error_count}<span class="session-errors"
              >{num(session.error_count)} error{session.error_count === 1 ? '' : 's'}</span
            >{/if}
        </span>
      </a>
    </td>
    <td class="session-now mono">
      {#if row.now.kind === 'open'}<span class="now-open state-{row.now.mark}"
          ><StateGlyph mark={row.now.mark} />{row.now.label}</span
        >{:else if row.now.kind === 'recent'}<span class="now-recent"
          ><span class="recent-dot"></span>active {ago(row.now.at, now)}</span
        >{:else}<span class="muted" title={clock(row.now.at)}>{clock(row.now.at)}</span>{/if}
    </td>
    <td class="numeric mono sized">{num(session.call_count)}{@render bar(row.requests)}</td>
    <td class="numeric mono">{num(session.turn_count)}</td>
    <td
      class="numeric mono sized"
      title="Provider reported; input includes cached context. Not subscription allowance or charges."
      >{compactOrDash(session.tokens.input)}
      <span class="muted">· {compactOrDash(session.tokens.output)}</span>{@render bar(
        row.tokens,
      )}</td
    >
    <td class="numeric mono muted">{span(session.last_seen - session.first_seen)}</td>
  </tr>
{/snippet}

<section class="panel list-panel sessions-panel">
  <div class="list-content">
    {#if shown || list.folded.length}
      <div class="table-scroll">
        <table class="sessions-table">
          {@render columns()}
          <thead
            ><tr
              ><th>Session</th><th>Now</th><th class="numeric">Requests</th><th class="numeric"
                >Turns</th
              ><th class="numeric">Tokens in · out</th><th class="numeric">Active for</th></tr
            ></thead
          >
          {#each list.groups as group (group.key)}
            <tbody>
              <tr class="day-row"
                ><th colspan="6">{group.label}<span class="muted"> · {group.rows.length}</span></th
                ></tr
              >
              {#each group.rows as each (each.session.id)}{@render row(each)}{/each}
            </tbody>
          {/each}
        </table>
      </div>
      {#if list.folded.length}
        <details class="sessions-folded">
          <summary
            >{list.folded.length} single-call session{list.folded.length === 1 ? '' : 's'} with no turns<span
              class="muted"
            >
              · a client's own checks, most often; select to list them</span
            ></summary
          >
          <table class="sessions-table">
            {@render columns()}
            <tbody>
              {#each list.folded as each (each.session.id)}{@render row(each)}{/each}
            </tbody>
          </table>
        </details>
      {/if}
    {:else}<div class="empty-state">
        <div class="empty-icon"><Icon name="sessions" size={25} /></div>
        <h2>{query ? 'No matching sessions' : 'No reported sessions'}</h2>
        <p>
          {query
            ? 'Try a different prompt, folder, client, or session ID.'
            : 'Sessions appear when clients send session identifiers through Ren.'}
        </p>
        {#if query}<button class="button" onclick={() => (query = '')}>Clear search</button
          >{:else}<button class="button" onclick={() => goto('/traffic')}
            >Go to request logs <Icon name="arrow" size={14} /></button
          >{/if}
      </div>{/if}
  </div>
</section>

<style>
  .muted {
    color: var(--muted);
  }
  /* Fixed columns, so the folded table lines up with the one above it. */
  .sessions-table {
    width: 100%;
    table-layout: fixed;
  }
  .col-session {
    width: auto;
  }
  .col-now {
    width: 190px;
  }
  .col-requests {
    width: 110px;
  }
  .col-turns {
    width: 70px;
  }
  .col-tokens {
    width: 150px;
  }
  .col-active {
    width: 110px;
  }
  .sessions-table td {
    vertical-align: middle;
  }
  /* A row: who and what, then the state now, then how big it got. */
  .session-cell {
    overflow: hidden;
  }
  .session-link {
    display: flex;
    flex-direction: column;
    /* The global row link centres its items; in a column that would centre them across. */
    align-items: stretch;
    gap: 3px;
    min-width: 0;
    padding-left: 4px;
  }
  .session-title {
    display: flex;
    align-items: center;
    gap: 0;
    min-width: 0;
  }
  .session-title strong {
    flex: none;
    margin-right: 10px;
    font-weight: 600;
  }
  .session-prompt {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .session-meta {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-left: 15px;
    color: var(--muted);
    font-size: 11px;
  }
  .session-folder {
    color: var(--ink);
  }
  .session-errors {
    padding: 0 5px;
    border: 1px solid currentColor;
    border-radius: 2px;
    color: var(--error);
  }
  .session-now {
    font-size: 11.5px;
    white-space: nowrap;
  }
  .now-open {
    color: var(--ink);
  }
  .now-open.state-yours {
    color: var(--success);
  }
  .now-recent {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--ink);
  }
  .recent-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--success);
  }
  /* Sizes as thin bars under the numbers, against the largest in the list. */
  .sized {
    position: relative;
  }
  .size-bar {
    display: block;
    width: 100%;
    max-width: 120px;
    height: 4px;
    margin: 4px 0 0 auto;
  }
  .size-bar rect {
    fill: color-mix(in srgb, var(--chart) 70%, transparent);
  }
  .day-row th {
    padding: 6px 12px;
    border-bottom: 1px solid var(--line);
    background: var(--surface-subtle);
    color: var(--ink);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-align: left;
    text-transform: uppercase;
  }
  .day-row .muted {
    font-weight: 400;
  }
  .sessions-folded {
    border-top: 1px solid var(--line);
  }
  .sessions-folded summary {
    padding: 10px 12px;
    color: var(--ink);
    font-size: 12px;
    cursor: pointer;
  }
  .sessions-folded summary:hover {
    background: var(--row-hover);
  }
  .sessions-folded[open] summary {
    border-bottom: 1px solid var(--line);
  }
  @media (max-width: 760px) {
    .sessions-table th:nth-child(n + 3),
    .sessions-table td:nth-child(n + 3),
    .sessions-table col:nth-child(n + 3) {
      display: none;
    }
    /* Narrow: two columns that share the width, the session taking what is left. */
    .sessions-table {
      table-layout: auto;
    }
    .session-cell {
      width: 100%;
      max-width: 0;
    }
    .session-now {
      white-space: normal;
    }
    .day-row th {
      display: table-cell !important;
    }
  }
</style>
