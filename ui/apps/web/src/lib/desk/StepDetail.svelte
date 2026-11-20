<!--
  One step, whole, beside the transcript: what it did and on what, its
  outcome and time, and everything its client reported of it: a command as
  run and its output, the files a change touched, what a tool's input named.
  ↑ and ↓ move to the conversation's step before or after.
-->
<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import { Steps, type Step } from '#lib/steps.ts';
  import { STEP_ICONS } from './StepGroup.svelte';

  let { step }: { step: Step } = $props();
  const entry = $derived(step.entry);
</script>

<div class="detail">
  <header>
    <span class="line">
      {#if step.state === 'running'}<Glyph mark="running" label={true} />
      {:else if step.state === 'failed'}<Glyph mark="failed" label={true} />
      {:else if step.state === 'declined'}<Glyph mark="idle" label="Declined" />
      {:else}<Glyph mark="passed" label="Done" />{/if}
      <span class="verb" title={Steps.VERBS[step.act]} aria-label={Steps.VERBS[step.act]} role="img"
        ><Icon name={STEP_ICONS[step.act]} size={13} /></span
      >
      <span class="target mono">{step.targets.join(', ')}</span>
    </span>
    <span class="facts muted">
      {#if step.said}<span>{step.said}</span>{/if}
      {#if step.failure}<span class="mono">{step.failure}</span>{/if}
      {#if step.took !== null}<span class="mono">{Steps.time(step.took)}</span>{/if}
      <span class="keys"><kbd>↑</kbd> <kbd>↓</kbd> steps</span>
    </span>
  </header>

  {#if entry.kind === 'command'}
    <section>
      <h3>Command</h3>
      <pre class="mono">{entry.command}</pre>
    </section>
    <section class="grow">
      <h3>Output</h3>
      {#if entry.output}<pre
          class="mono output"
          class:failed={step.state === 'failed'}>{entry.output}</pre>
      {:else}<p class="muted">{step.state === 'running' ? 'Still running.' : 'No output.'}</p>{/if}
    </section>
  {:else if entry.kind === 'file_change'}
    <section>
      <h3>Files</h3>
      <ul class="mono">
        {#each step.targets as path (path)}<li>{path}</li>{/each}
      </ul>
    </section>
  {:else}
    <section>
      <h3>{entry.name}</h3>
      {#if entry.effects.length}
        <ul class="mono">
          {#each entry.effects as effect, index (index)}<li>
              <span class="muted">{effect.kind}</span>
              {effect.target}
            </li>{/each}
        </ul>
      {:else}<p class="muted">Its client said nothing of what it acted on.</p>{/if}
    </section>
  {/if}
</div>

<style>
  .detail {
    display: flex;
    flex-direction: column;
    min-height: 100%;
    font-size: 12.5px;
  }
  header {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 52px 12px 16px;
    border-bottom: 1px solid var(--line);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .verb {
    display: inline-flex;
    color: var(--muted);
  }
  .target {
    min-width: 0;
    overflow: hidden;
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    font-size: 11.5px;
  }
  .keys {
    margin-left: auto;
  }
  kbd {
    padding: 0 4px;
    border: 1px solid var(--control-border);
    border-bottom-width: 2px;
    border-radius: 2px;
    font-family: var(--mono, monospace);
    font-size: 10px;
  }
  section {
    padding: 12px 16px;
    border-bottom: 1px solid var(--line);
  }
  section.grow {
    flex: 1;
  }
  h3 {
    margin: 0 0 8px;
    color: var(--muted);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  pre {
    margin: 0;
    padding: 10px 12px;
    overflow: auto;
    border: 1px solid var(--line);
    background: var(--surface-inset);
    font-size: 11.5px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  pre.output.failed {
    box-shadow: inset 2px 0 var(--error);
  }
  ul {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0;
    padding: 0;
    font-size: 11.5px;
    list-style: none;
  }
  li .muted {
    margin-right: 6px;
  }
  p {
    margin: 0;
  }
</style>
