<!--
  What waits on the operator, across every project: approvals a working run
  asks, a client signed out, a canary to promote, a preview to check, work
  still queued. An
  approval is a decision with consequences, so it is answered in a modal
  that shows it whole (or in its session), never from the list itself.
-->
<script lang="ts">
  import Icon from '#lib/Icon.svelte';
  import Modal from '#lib/layer/Modal.svelte';
  import { toasts } from '#lib/layer/toasts.svelte.ts';
  import { invalidateAll } from '$app/navigation';
  import { ProjectBoard, type Approval } from '#lib/projects.ts';
  import {
    decisions,
    RunCommandError,
    Runs,
    RunsClient,
    Sessions,
    type Decision,
  } from '#lib/runs.ts';
  import { Work } from '#lib/work.ts';
  import { harnessLabel } from '#lib/activity.ts';

  let { board }: { board: ProjectBoard } = $props();
  const runs = new RunsClient();
  /** The approval under review in the modal; null when none is. */
  let reviewing = $state<Approval | null>(null);
  /** Whether an answer is on its way, and what went wrong with the last. */
  let answering = $state(false);
  let failed = $state<string | null>(null);
  /** The modal's answers, the primary one (Allow) last. */
  const answers = decisions.toReversed();

  function review(waiting: Approval) {
    failed = null;
    reviewing = waiting;
  }

  async function answer(waiting: Approval, decision: Decision) {
    answering = true;
    failed = null;
    try {
      await runs.toSession(waiting.session, {
        kind: 'answer',
        approval: waiting.approval,
        decision,
      });
      const said = Runs.decision(decision);
      toasts.show({
        mark: said.mark,
        text: said.done,
        detail: Runs.ask(waiting.ask).action,
        link: {
          label: 'Open session',
          href: Sessions.deskHref(waiting.project, { session: waiting.session }),
        },
      });
      reviewing = null;
      await invalidateAll();
    } catch (failure) {
      failed = failure instanceof RunCommandError ? failure.message : 'The answer did not go.';
    } finally {
      answering = false;
    }
  }

  /** Where an approval comes from: its project, and its session's prompt. */
  const origin = (waiting: Approval) =>
    [waiting.project ?? 'No project', waiting.prompt].filter((part) => part !== null).join(' · ');
</script>

<h2>Pending</h2>
{#if !board.pending}
  <p class="muted quiet">Nothing waits on you.</p>
{/if}
{#if board.approvals.length}
  <h3>Approvals</h3>
  <ul>
    {#each board.approvals as waiting (`${waiting.session}:${waiting.approval}`)}
      {@const said = Runs.ask(waiting.ask)}
      <li>
        <span class="muted">{origin(waiting)}</span>
        <span>{said.action}</span>
        {#if said.detail}<code class="ask">{said.detail}</code>{/if}
        <span class="actions">
          {#if waiting.ask.kind === 'questions'}
            <!-- Questions are answered in their session. -->
            <a
              class="button primary small"
              href={Sessions.deskHref(waiting.project, { session: waiting.session })}>Answer</a
            >
          {:else}
            <button class="button primary small" onclick={() => review(waiting)}>Review…</button>
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
{#if reviewing}
  {@const waiting = reviewing}
  {@const said = Runs.ask(waiting.ask)}
  <Modal title={said.action} eyebrow={origin(waiting)} onclose={() => (reviewing = null)}>
    {#if said.detail}<pre class="detail mono">{said.detail}</pre>{/if}
    {#if waiting.ask.kind === 'command' && waiting.ask.cwd}<span class="muted mono"
        >in {waiting.ask.cwd}</span
      >{/if}
    {#if said.reason}<p class="reason">{said.reason}</p>{/if}
    {#if failed}<p class="error quiet" role="alert">{failed}</p>{/if}
    {#snippet actions()}
      {#each answers as decision (decision)}
        {@const words = Runs.decision(decision)}
        <button
          class="button small"
          class:primary={decision === 'accept'}
          disabled={answering}
          title={words.hint}
          onclick={() => void answer(waiting, decision)}>{words.label}</button
        >
      {/each}
    {/snippet}
  </Modal>
{/if}
{#if board.signedOut.length}
  <h3>Signed out</h3>
  <ul>
    {#each board.signedOut as client (client.harness)}
      <li>
        <span>{harnessLabel(client.harness)}</span>
        <span class="muted">Runs cannot start with it until it is signed in on this host.</span>
        <span class="actions"><a class="button primary small" href="/tools">Sign in…</a></span>
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
  .detail {
    margin: 0;
    padding: 10px 12px;
    overflow: auto;
    border: 1px solid var(--line);
    background: var(--surface-inset);
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .reason {
    margin: 0;
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
