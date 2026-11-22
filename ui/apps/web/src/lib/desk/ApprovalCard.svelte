<!--
  A permission the session's client asks for: what it would do, where, and
  why, with each decision the operator can give.
-->
<script lang="ts">
  import Icon from '#lib/Icon.svelte';
  import { decisions, Runs, type Decision, type Waiting } from '#lib/runs.ts';

  let {
    waiting,
    index,
    busy,
    onanswer,
  }: {
    waiting: Waiting;
    /** Its place among those waiting, from 0. */
    index: number;
    busy: boolean;
    onanswer: (decision: Decision) => void;
  } = $props();

  const line = $derived(Runs.ask(waiting.ask));
</script>

<div class="approval" role="group" aria-label="Approval {index + 1}">
  <div class="approval-text">
    <strong><Icon name="shield" size={13} /> {line.action}</strong>
    {#if line.detail}<pre class="approval-detail mono">{line.detail}</pre>{/if}
    {#if waiting.ask.kind === 'command' && waiting.ask.cwd}<span class="muted mono"
        >in {waiting.ask.cwd}</span
      >{/if}
    {#if line.reason}<span class="muted">{line.reason}</span>{/if}
  </div>
  <div class="approval-actions">
    {#each decisions as decision (decision)}
      {@const words = Runs.decision(decision)}
      <button
        class="button small"
        class:primary={decision === 'accept'}
        disabled={busy}
        onclick={() => onanswer(decision)}
        title={words.hint}>{words.label}</button
      >
    {/each}
  </div>
</div>

<style>
  .approval {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 10px 16px;
    font-size: 12px;
  }
  .approval-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }
  .approval-text strong {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--warning);
  }
  .approval-detail {
    margin: 0;
    padding: 6px 8px;
    background: var(--surface);
    border: 1px solid var(--warning-border);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 160px;
    overflow: auto;
  }
  .approval-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  @media (max-width: 760px) {
    .approval {
      flex-direction: column;
    }
  }
</style>
