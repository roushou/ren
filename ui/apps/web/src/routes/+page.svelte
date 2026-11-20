<!--
  Projects: the factory's home (docs/plans/hierarchy.md). Each registered
  project with its open work, its sessions, and where its release stands
  (a row opens the project; each cell its page); and,
  beside them, what waits on the operator across every project: a canary to
  promote, a preview to check, work still queued.
-->
<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import Toolbar from '#lib/Toolbar.svelte';
  import PendingPanel from '#lib/PendingPanel.svelte';
  import { goto } from '$app/navigation';
  import { ProjectBoard } from '#lib/projects.ts';
  import { ReleaseStrip } from '#lib/release.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const board = $derived(new ProjectBoard(data.projects, data.work, data.live, data.sessions));
  const running = $derived(board.active.reduce((sum, project) => sum + project.running, 0));

  /** A click on a project's row opens it, unless it was on one of the
   * row's own links. */
  function open(event: MouseEvent, href: string) {
    if (event.target instanceof Element && event.target.closest('a, button')) return;
    void goto(href);
  }
</script>

{#snippet scope()}
  <div class="scope-group">
    <strong>{board.active.length}</strong>
    <span class="muted">projects</span>
    <span class="muted">·</span>
    <strong>{running}</strong>
    <span class="muted">{running === 1 ? 'run' : 'runs'} under way</span>
  </div>
{/snippet}

<Toolbar {scope} />

<section class="projects-view" aria-label="Projects">
  <div class="board">
    {#if data.failed}
      <p class="error" role="alert">{data.failed}</p>
    {:else if !board.projects.length}
      <p class="empty">
        No project is registered. <code>ren project add --dir ~/dev/acme</code> registers one;
        <code>cargo xtask install-vps</code> registers Ren itself.
      </p>
    {:else}
      <div class="table-scroll">
        <table>
          <thead>
            <tr>
              <th>Project</th>
              <th>Release</th>
              <th>Work</th>
              <th>Sessions</th>
            </tr>
          </thead>
          <tbody>
            {#each board.active as project (project.id)}
              {@const strip = project.release ? ReleaseStrip.of(project.release) : null}
              {@const href = ProjectBoard.href(project.name)}
              <!-- The row is a larger target for the project's own link, which
                   is what the keyboard reaches. -->
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <tr class="project-row" onclick={(event) => open(event, href)}>
                <td>
                  <a class="name" {href}>{project.name}</a>
                  <span class="muted mono path">{project.repository}</span>
                </td>
                <td>
                  {#if strip}
                    <a
                      class="release"
                      href={ProjectBoard.releaseHref(project)}
                      title={strip.line.detail}
                      aria-label={strip.label}
                    >
                      <span class="strip" aria-hidden="true">
                        <span class="stage">main</span>
                        <span class="stage canary-{strip.canary}"
                          >canary <span class="mark mono"
                            >{strip.canary === 'passed'
                              ? '✓'
                              : strip.canary === 'failed'
                                ? '✗'
                                : '–'}</span
                          ></span
                        >
                        <span class="stage production-{strip.production.kind}"
                          >prod <span class="mark mono"
                            >{#if strip.production.kind === 'behind'}−{strip.production
                                .commits}{:else if strip.production.kind === 'on_main'}✓{:else}?{/if}</span
                          ></span
                        >
                      </span>
                      <span class="word tone-{strip.promotion.tone}" aria-hidden="true"
                        >{strip.promotion.word}</span
                      >
                    </a>
                  {:else}
                    <span class="muted">No canary yet</span>
                  {/if}
                </td>
                <td><a class="cell" {href}>{ProjectBoard.openWork(project)}</a></td>
                <td>
                  <a class="cell" href={`${href}/sessions`}>
                    {#if project.running}
                      <span class="live"><Glyph mark="responding" />{project.running} running</span>
                      <span class="muted">· {project.sessions} open</span>
                    {:else if project.sessions}
                      {project.sessions} open
                    {:else}
                      <span class="muted">none</span>
                    {/if}
                  </a>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      {#if data.sessions.length}
        <p class="scratch">
          <a href="/scratch">Scratch</a>
          <span class="muted"
            >{board.scratch}
            {board.scratch === 1 ? 'session works' : 'sessions work'} for no project</span
          >
        </p>
      {/if}
      {#if board.archived.length}
        <p class="archived muted">
          Archived: {board.archived.map((project) => project.name).join(', ')}
        </p>
      {/if}
    {/if}
  </div>

  <aside class="pending" aria-label="Pending"><PendingPanel {board} /></aside>
</section>

<style>
  .projects-view {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 1px;
    background: var(--line);
    overflow: auto;
  }
  .board {
    flex: 999 1 560px;
    min-width: 0;
    background: var(--surface);
  }
  .pending {
    flex: 1 1 300px;
    background: var(--surface);
  }
  .table-scroll {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--muted);
    padding: 8px 16px;
    background: var(--surface-subtle);
    border-bottom: 1px solid var(--line);
  }
  td {
    padding: 10px 16px;
    border-bottom: 1px solid var(--line-subtle);
    vertical-align: top;
  }
  .name {
    font-weight: 600;
    color: var(--ink);
  }
  .project-row {
    cursor: pointer;
  }
  .project-row:hover td {
    background: var(--surface-hover);
  }
  .cell {
    color: var(--ink);
  }
  .cell:hover {
    color: var(--accent-text);
  }
  .path {
    display: block;
    font-size: 11px;
  }
  .release {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 10px;
    color: var(--ink);
    white-space: nowrap;
  }
  .strip {
    display: inline-flex;
    gap: 1px;
    background: var(--line);
    border: 1px solid var(--line);
    font-size: 11px;
  }
  .stage {
    padding: 1px 6px;
    background: var(--surface-subtle);
    color: var(--muted);
  }
  .mark {
    font-weight: 600;
    color: var(--ink);
  }
  .canary-passed .mark,
  .production-on_main .mark {
    color: var(--success);
  }
  .canary-failed .mark {
    color: var(--error);
  }
  .canary-none .mark,
  .production-uncounted .mark {
    color: var(--muted);
  }
  .word {
    font-weight: 600;
  }
  .release:hover .word {
    text-decoration: underline;
  }
  .tone-ok {
    color: var(--success);
  }
  .tone-warning {
    color: var(--warning);
  }
  .tone-bad {
    color: var(--error);
  }
  .live {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .archived,
  .scratch {
    padding: 10px 16px;
    margin: 0;
  }
  .scratch {
    display: flex;
    gap: 8px;
    border-bottom: 1px solid var(--line-subtle);
  }
  .empty {
    padding: 16px;
    margin: 0;
    color: var(--muted);
  }
</style>
