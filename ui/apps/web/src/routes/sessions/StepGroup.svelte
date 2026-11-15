<!--
  The steps between two things the agent said, as one line of what they did
  ("Read supervisor.rs ×2 · Ran 3 commands"), folded until opened. Open, each
  step is a row: what it did and on what, in its client's words when it gave
  some; a command opens again to the command exactly as run and its output.
  Success says nothing: only a step that runs, failed, or was declined is
  marked, and a failed one never folds away unseen (the page opens its group).
-->
<script lang="ts">
  import Icon, { type IconName } from '#lib/Icon.svelte';
  import { Steps, type Act, type GroupState, type Step } from '#lib/steps.ts';

  let {
    steps,
    state,
    open,
    ontoggle,
  }: {
    steps: readonly Step[];
    state: GroupState;
    open: boolean;
    ontoggle: (open: boolean) => void;
  } = $props();

  const summary = $derived(Steps.summary(steps));
  const failed = $derived(steps.filter((step) => step.state === 'failed').length);

  const ICONS: Record<Act, IconName> = {
    read: 'file',
    search: 'search',
    list: 'rows',
    edit: 'pencil',
    run: 'terminal',
    delegate: 'spark',
    fetch: 'globe',
    tool: 'tool',
  };
</script>

{#snippet mark(step: Step)}
  {#if step.state === 'running'}<span class="phase-dot tone-live" title="Running"></span>
  {:else if step.state === 'failed'}<span class="chip danger">{step.failure}</span>
  {:else if step.state === 'declined'}<span class="chip neutral">declined</span>{/if}
{/snippet}

{#snippet line(step: Step)}
  <Icon name={ICONS[step.act]} size={12} />
  <span class="verb">{Steps.VERBS[step.act]}</span>
  {#if step.said}
    <span class="said">{step.said}</span>
    <span class="target mono muted" title={step.targets.join('\n')}>{step.targets.join(', ')}</span>
  {:else}
    <span class="target mono" title={step.targets.join('\n')}>{step.targets.join(', ')}</span>
  {/if}
  {@render mark(step)}
{/snippet}

<section class="steps" class:failed={state === 'failed'}>
  <button class="steps-line" aria-expanded={open} onclick={() => ontoggle(!open)}>
    <span class="fold" class:open><Icon name="chevron" size={10} /></span>
    <span class="summary">{summary}</span>
    {#if state === 'running'}<span class="phase-dot tone-live" title="Running"></span>{/if}
    {#if failed}<span class="chip danger">{failed} failed</span>{/if}
    <span class="count muted">{steps.length === 1 ? '1 step' : `${steps.length} steps`}</span>
  </button>
  {#if open}
    <ol class="step-list">
      {#each steps as step (step.entry.item)}
        <li>
          {#if step.entry.kind === 'command'}
            {@const command = step.entry}
            <details class="step">
              <summary class="step-row">{@render line(step)}</summary>
              <pre class="activity-pre">{command.command}</pre>
              {#if command.output}<pre class="activity-pre output">{command.output}</pre>{/if}
            </details>
          {:else}
            <div class="step-row">{@render line(step)}</div>
          {/if}
        </li>
      {/each}
    </ol>
  {/if}
</section>

<style>
  .steps {
    border-bottom: 1px solid var(--line-subtle);
    font-size: 12px;
  }
  .steps.failed {
    box-shadow: inset 2px 0 var(--error);
  }
  .steps-line {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 16px;
    border: 0;
    border-radius: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .steps-line:hover {
    color: var(--ink);
  }
  .fold {
    display: inline-flex;
    flex: none;
    transition: transform 0.12s;
  }
  .fold.open {
    transform: rotate(90deg);
  }
  .summary {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    flex: none;
    margin-left: auto;
  }
  .steps-line > .chip,
  .steps-line > .phase-dot {
    flex: none;
  }
  .step-list {
    margin: 0;
    padding: 0 0 4px;
    list-style: none;
  }
  .step-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 16px 3px 34px;
    color: var(--ink);
  }
  summary.step-row {
    cursor: pointer;
    list-style: none;
  }
  summary.step-row::-webkit-details-marker {
    display: none;
  }
  .step[open] > summary.step-row {
    background: var(--surface-inset);
  }
  .step > pre {
    margin: 0 16px 4px 34px;
  }
  .verb {
    flex: none;
    width: 52px;
    color: var(--muted);
  }
  .said {
    flex: none;
    max-width: 50%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .step-row > .chip,
  .step-row > .phase-dot {
    flex: none;
    margin-left: auto;
  }
  /* Scoped here: the page's own dots are drawn by its styles. */
  .phase-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--live);
  }
</style>
