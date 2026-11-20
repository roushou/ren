<!--
  What waits on the operator: approvals a working run asks (answered here),
  a canary to promote, a preview to check, work still queued. The Projects
  view shows it across every project, a project's Overview for its own
  (`ProjectBoard.of`).
-->
<script lang="ts">
  import Icon from '#lib/Icon.svelte';
  import { invalidateAll } from '$app/navigation';
  import { ProjectBoard, type Approval } from '#lib/projects.ts';
  import { RunCommandError, Runs, RunsClient, Sessions, type Decision } from '#lib/runs.ts';
  import { Work } from '#lib/work.ts';

  let { board }: { board: ProjectBoard } = $props();
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
</script>

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
          {#if waiting.ask.kind === 'questions'}
            <!-- Questions are answered in their session, or skipped here. -->
            <a
              class="button primary small"
              href={Sessions.deskHref(waiting.project, { session: waiting.session })}>Answer</a
            >
            <button
              class="button small"
              disabled={answering !== null}
              onclick={() => void answer(waiting, 'decline')}>Skip</button
            >
          {:else}
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
          {/if}
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
        {#if home}<a href={Work.href(home, item.id)}>{item.title}</a>{:else}<span>{item.title}</span
          >{/if}
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
          <a href={ProjectBoard.href(queued.project)}>{queued.project}</a>
        {:else}
          <span>No project</span>
        {/if}
        <span class="muted">{queued.count} to send</span>
      </li>
    {/each}
  </ul>
{/if}

<style>
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
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line-subtle);
  }
  .muted {
    font-size: 12px;
  }
  .out {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
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
  .quiet {
    padding: 12px 16px;
    margin: 0;
  }
  .muted {
    color: var(--muted);
  }
  .error {
    color: var(--error);
  }
</style>
