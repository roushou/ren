<!--
  An approval reviewed in full, outside its session: the question it puts,
  the command and where it runs, why it was asked, and the four answers,
  the primary one (Allow once) last. Nothing is focused, so Enter answers
  nothing; Escape closes without an answer. It draws `answers.reviewing`.
-->
<script lang="ts">
  import Modal from '#lib/layer/Modal.svelte';
  import { harnessLabel } from '#lib/activity.ts';
  import { decisions, Runs } from '#lib/runs.ts';
  import { ApprovalAnswers } from '#lib/approvals.svelte.ts';

  let { answers }: { answers: ApprovalAnswers } = $props();
  /** The answers, the primary one last. */
  const order = decisions.toReversed();
</script>

{#if answers.reviewing}
  {@const waiting = answers.reviewing}
  {@const said = Runs.ask(waiting.ask)}
  <Modal
    title={Runs.question(waiting.ask, harnessLabel(waiting.harness))}
    eyebrow="Approval · {ApprovalAnswers.origin(waiting)}"
    onclose={() => answers.close()}
  >
    {#if said.detail}<pre class="detail mono">{said.detail}</pre>{/if}
    {#if waiting.ask.kind === 'command' && waiting.ask.cwd}<span class="muted mono"
        >runs in {waiting.ask.cwd}</span
      >{/if}
    {#if said.reason}<p class="reason">{said.reason}</p>{/if}
    {#if answers.failed}<p class="error quiet" role="alert">{answers.failed}</p>{/if}
    {#snippet actions()}
      {#each order as decision (decision)}
        {@const words = Runs.decision(decision)}
        <button
          class="button small"
          class:primary={decision === 'accept'}
          disabled={answers.answering}
          title={words.hint}
          onclick={() => void answers.answer(waiting, decision)}>{words.label}</button
        >
        {#if decision === 'decline'}<kbd class="esc" title="Escape closes without answering"
            >esc</kbd
          >{/if}
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
  .esc {
    align-self: center;
    margin-right: auto;
    padding: 0 4px;
    border: 1px solid var(--control-border);
    border-bottom-width: 2px;
    border-radius: 2px;
    color: var(--muted);
    font-family: var(--mono, monospace);
    font-size: 10px;
  }
  .error {
    color: var(--error);
  }
</style>
