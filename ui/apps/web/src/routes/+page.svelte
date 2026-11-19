<!--
  Projects: the factory's home (docs/plans/hierarchy.md). Each registered
  project with its open work, its runs, and where its release stands; and,
  beside them, what waits on the operator across every project: a canary to
  promote, a preview to check, work still queued.
-->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import Icon from '#lib/Icon.svelte';
  import { invalidateAll } from '$app/navigation';
  import { ProjectBoard, type Approval } from '#lib/projects.ts';
  import { ReleaseLine } from '#lib/release.ts';
  import { RunCommandError, Runs, RunsClient, Sessions, type Decision } from '#lib/runs.ts';
  import { Work } from '#lib/work.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const board = $derived(new ProjectBoard(data.projects, data.work, data.live, data.sessions));
  const runs = new RunsClient();
  /** The approval being answered, and what went wrong with the last answer. */
  let answering = $state<string | null>(null);
  let failed = $state<string | null>(null);

  async function answer(waiting: Approval, decision: Decision) {
    answering = waiting.approval;
    failed = null;
    try {
      await runs.toSession(waiting.session, {
        kind: 'answer',
        approval: waiting.approval,
        decision,
      });
      await invalidateAll();
    } catch (failure) {
      failed = failure instanceof RunCommandError ? failure.message : 'The answer did not go.';
    } finally {
      answering = null;
    }
  }
  const running = $derived(board.active.reduce((sum, project) => sum + project.running, 0));
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
              <th>Runs</th>
            </tr>
          </thead>
          <tbody>
            {#each board.active as project (project.id)}
              {@const line = project.release ? ReleaseLine.of(project.release) : null}
              <tr>
                <td>
                  <a class="name" href={ProjectBoard.href(project.name)}>{project.name}</a>
                  <span class="muted mono path">{project.repository}</span>
                  {#if project.factory === 'ren'}<span class="tag">Ren itself</span>{/if}
                </td>
                <td>
                  {#if line}
                    <a class="release tone-{line.tone}" href={ProjectBoard.releaseHref(project)}
                      >{line.text}</a
                    >
                  {:else}
                    <span class="muted">No canary yet</span>
                  {/if}
                </td>
                <td>{ProjectBoard.openWork(project)}</td>
                <td>
                  {#if project.running}
                    <span class="live"><span class="dot"></span>{project.running} running</span>
                  {:else}
                    <span class="muted">none</span>
                  {/if}
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

  <aside class="pending" aria-label="Pending">
    <h2>Pending</h2>
    {#if !board.pending}
      <p class="muted quiet">Nothing waits on you.</p>
    {/if}
    {#if board.approvals.length}
      <h3>Approvals</h3>
      {#if failed}<p class="error quiet" role="alert">{failed}</p>{/if}
      <ul>
        {#each board.approvals as waiting (`${waiting.session}:${waiting.approval}`)}
          {@const said = Runs.ask(waiting.ask)}
          <li>
            <span class="muted"
              >{[waiting.project ?? 'no project', waiting.prompt]
                .filter((part) => part !== null)
                .join(' · ')}</span
            >
            <span>{said.action}</span>
            {#if said.detail}<code class="ask">{said.detail}</code>{/if}
            {#if said.reason}<span class="muted">{said.reason}</span>{/if}
            <span class="actions">
              <button
                class="button primary small"
                disabled={answering !== null}
                onclick={() => void answer(waiting, 'accept')}>Approve</button
              >
              <button
                class="button small"
                disabled={answering !== null}
                onclick={() => void answer(waiting, 'decline')}>Decline</button
              >
              <a
                class="button small"
                href={Sessions.deskHref(waiting.project, { session: waiting.session })}
                >Open session</a
              >
            </span>
          </li>
        {/each}
      </ul>
    {/if}
    {#if board.toPromote.length}
      <h3>Ready to promote</h3>
      <ul>
        {#each board.toPromote as project (project.id)}
          <li>
            <span class="muted">{project.name} · {project.release?.policy}</span>
            <a href={ProjectBoard.releaseHref(project)}
              >Canary {project.release?.canary?.commit.slice(0, 12)} passed</a
            >
          </li>
        {/each}
      </ul>
    {/if}
    {#if board.previews.length}
      <h3>Previews to check</h3>
      <ul>
        {#each board.previews as item (item.id)}
          {@const home = board.homeOf(item)}
          <li>
            <span class="muted">{home ?? 'no project'}</span>
            <a href={home ? Work.href(home, { item: item.id }) : `/work?item=${item.id}`}
              >{item.title}</a
            >
            {#if item.preview}<a class="out" href={item.preview.ui} target="_blank" rel="noopener"
                ><Icon name="arrow" size={11} />Open the preview</a
              >{/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if board.queued.length}
      <h3>Queued work</h3>
      <ul>
        {#each board.queued as queued (queued.project)}
          <li>
            {#if queued.project}
              <a href={Work.href(queued.project)}>{queued.project}</a>
            {:else}
              <span>No project</span>
            {/if}
            <span class="muted">{queued.count} to send</span>
          </li>
        {/each}
      </ul>
    {/if}
  </aside>
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
  .path {
    display: block;
    font-size: 11px;
  }
  .tag {
    display: inline-block;
    margin-top: 4px;
    padding: 0 5px;
    border: 1px solid var(--accent-border);
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: 11px;
  }
  .release {
    color: var(--ink);
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
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--live);
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
  h2 {
    margin: 0;
    padding: 12px 16px;
    font-size: 13px;
    font-weight: 600;
    border-bottom: 1px solid var(--line);
  }
  h3 {
    margin: 0;
    padding: 12px 16px 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .pending ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .pending li {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line-subtle);
  }
  .pending .muted {
    font-size: 12px;
  }
  .out {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
  }
  .quiet {
    padding: 12px 16px;
    margin: 0;
  }
  .ask {
    font-size: 12px;
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 4px;
  }
</style>
