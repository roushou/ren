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
  import { toasts } from '#lib/layer/toasts.svelte.ts';

  let {
    step,
    place,
    calls,
  }: {
    step: Step;
    /** Where it sits: its turn's number, and its place among the turn's
     * steps. */
    place: { turn: number; index: number; of: number } | null;
    /** Traffic narrowed to the calls around it, when its session has a
     * recording. */
    calls: string | null;
  } = $props();
  const entry = $derived(step.entry);
  /** Which of a command's parts shows: its output first. */
  let showing = $state<'output' | 'command'>('output');

  async function copyOutput() {
    if (entry.kind !== 'command') return;
    try {
      await navigator.clipboard.writeText(entry.output ?? entry.command);
      toasts.show({ mark: 'passed', text: 'Copied the output', detail: null });
    } catch {
      toasts.show({ mark: 'failed', text: 'The browser refused the copy', detail: null });
    }
  }
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
    <span class="facts muted mono">
      {[
        place ? `step ${place.index} of ${place.of}` : null,
        step.failure,
        step.took === null ? null : Steps.time(step.took),
      ]
        .filter((part) => part !== null)
        .join(' · ')}
    </span>
    {#if step.said}<span class="said">{step.said}</span>{/if}
  </header>

  {#if entry.kind === 'command'}
    <nav class="tabs" aria-label="What of the command shows">
      <button aria-pressed={showing === 'output'} onclick={() => (showing = 'output')}
        >Output</button
      ><button aria-pressed={showing === 'command'} onclick={() => (showing = 'command')}
        >Command</button
      >
    </nav>
    <section class="grow">
      {#if showing === 'command'}<pre class="mono">{entry.command}</pre>
      {:else if entry.output}<pre
          class="mono output">{#each Steps.lines(entry.output) as line, index (index)}<span
              class="line {line.reads ?? ''}">{line.text || ' '}</span
            >{/each}</pre>
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
  <footer>
    {#if entry.kind === 'command'}<button class="button small" onclick={() => void copyOutput()}
        >Copy output</button
      >{/if}
    {#if calls}<a class="button small" href={calls}>Open its calls in Traffic</a>{/if}
    <span class="keys muted"><kbd>↑</kbd> <kbd>↓</kbd> other steps</span>
  </footer>
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
    padding: 12px 80px 12px 16px;
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
  .said {
    font-size: 12px;
  }
  .keys {
    margin-left: auto;
    font-size: 11px;
  }
  .tabs {
    display: flex;
    padding: 0 16px;
    border-bottom: 1px solid var(--line);
  }
  .tabs button {
    padding: 8px 12px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .tabs button[aria-pressed='true'] {
    border-bottom-color: var(--accent);
    color: var(--ink);
  }
  .line {
    display: block;
  }
  .line.failed {
    background: color-mix(in srgb, var(--error) 14%, transparent);
    color: var(--error);
  }
  .line.passed {
    color: var(--success);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
    border-top: 1px solid var(--line);
  }
  kbd {
    padding: 0 4px;
    border: 1px solid var(--control-border);
    border-bottom-width: 2px;
    border-radius: 2px;
    font-family: var(--mono);
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
