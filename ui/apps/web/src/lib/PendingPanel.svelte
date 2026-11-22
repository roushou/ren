<!--
  What waits on the operator, across every project, one row each: a glyph
  for what kind of wait it is, its name (a link to where its details are),
  and the one thing to do about it. Approvals a working run asks, a client
  signed out, a canary to promote, a preview to check, work still queued.
  An approval is a decision with consequences, so it is answered in a modal
  that shows it whole (or in its session), never from the list itself.
-->
<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
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

<header class="head">
  <h2>
    Pending{#if board.waiting}<span class="count">·</span><span class="count">{board.waiting}</span
      >{/if}
  </h2>
  <span class="muted mono">across all projects</span>
</header>
{#if !board.pending}
  <p class="quiet muted"><Glyph mark="passed" label={true} /> Nothing waits on you.</p>
{:else}
  <ul>
    {#each board.approvals as waiting (`${waiting.session}:${waiting.approval}`)}
      {@const said = Runs.ask(waiting.ask)}
      {@const desk = Sessions.deskHref(waiting.project, { session: waiting.session })}
      <li>
        <Glyph mark="yours" label="Waits for you: {said.action}" />
        <a class="name" href={desk} title="{origin(waiting)} · {said.action}"
          >{waiting.prompt?.split('\n')[0] ?? said.action}</a
        >
        {#if waiting.ask.kind === 'questions'}
          <!-- Questions are answered in their session. -->
          <a class="button primary small" href={desk}>Answer</a>
        {:else}
          <button class="button primary small" onclick={() => review(waiting)}>Review…</button>
        {/if}
      </li>
    {/each}
    {#each board.signedOut as client (client.harness)}
      <li>
        <Glyph mark="warning" label="Signed out on this host" />
        <span class="name">{harnessLabel(client.harness)}</span>
        <a class="button small" href="/tools">Sign in</a>
      </li>
    {/each}
    {#each board.toPromote as project (project.id)}
      {@const canary = project.release?.canary ?? null}
      <li>
        <Glyph mark="passed" label="Canary passed" />
        <a
          class="name"
          href={ProjectBoard.releaseHref(project)}
          title="Canary {canary?.commit.slice(0, 12) ?? ''} passed · {project.release?.policy}"
          >{project.name} release</a
        >
        {#if project.factory === 'ren' && canary}<a
            class="button small"
            href="{ProjectBoard.releaseHref(project)}?promote={encodeURIComponent(canary.commit)}"
            >Promote…</a
          >{:else}<a class="button small" href={ProjectBoard.releaseHref(project)}>Release</a>{/if}
      </li>
    {/each}
    {#each board.previews as item (item.id)}
      {@const home = board.homeOf(item)}
      <li>
        <Glyph mark="info" label="A preview to check" />
        {#if home}<a class="name" href={Work.href(home, item.id)} title={home}>{item.title}</a
          >{:else}<span class="name">{item.title}</span>{/if}
        {#if item.preview}<a
            class="button small"
            href={item.preview.ui}
            target="_blank"
            rel="noopener">Open</a
          >{/if}
      </li>
    {/each}
    {#each board.queued as queued (queued.project)}
      <li>
        <Glyph mark="queued" label="{queued.count} queued to send" />
        <span class="name"
          >{queued.project ?? 'No project'}<span class="muted sep">· {queued.count} queued</span
          ></span
        >
        {#if queued.project}<a class="button small" href={ProjectBoard.href(queued.project)}>Open</a
          >{/if}
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
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--line);
  }
  h2 {
    margin: 0;
    font-family: var(--mono, monospace);
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  h2 {
    display: flex;
    gap: 8px;
  }
  .count {
    color: var(--muted);
  }
  .head .muted {
    font-size: 11px;
  }
  ul {
    margin: 0;
    padding: 4px 0;
    list-style: none;
  }
  li {
    display: grid;
    grid-template-columns: 12px minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 12px;
    min-height: 40px;
    padding: 4px 16px;
  }
  li:hover {
    background: var(--surface-hover);
  }
  .name {
    overflow: hidden;
    color: var(--ink);
    font-size: 13px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  a.name:hover {
    text-decoration: underline;
  }
  .sep {
    margin-left: 6px;
  }
  .quiet {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    padding: 14px 16px;
  }
  .muted {
    color: var(--muted);
  }
  .error {
    color: var(--error);
  }
</style>
