<!--
  The steps between two things the agent said, as one line of what they did
  ("Read supervisor.rs ×2 · Ran 3 commands") with their outcomes counted and
  their time added up, folded until opened. Open, each step is a row: its
  verb, what it acted on, its outcome, and a bar of its time against the
  group's longest. A row opens the step whole beside the transcript
  (`onopen`); its box picks it to act on with others (`onpick`). The
  operator opens the group; it never opens itself.
-->
<script lang="ts">
  import { Elapsed } from '#lib/time.ts';
  import Meter from '../Meter.svelte';
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import type { ItemId } from '#lib/runs.ts';
  import { Steps, type GroupState, type Step } from '#lib/steps.ts';

  let {
    steps,
    state,
    open,
    ontoggle,
    onopen,
    chosen,
    picked,
    onpick,
  }: {
    steps: readonly Step[];
    state: GroupState;
    open: boolean;
    ontoggle: (open: boolean) => void;
    /** Open `step` beside the transcript. */
    onopen: (step: Step) => void;
    /** The step open beside the transcript, if it is one of these. */
    chosen: ItemId | null;
    /** The steps picked to act on together, across the conversation. */
    picked: ReadonlySet<ItemId>;
    onpick: (step: Step, on: boolean) => void;
  } = $props();

  const summary = $derived(Steps.summary(steps));
  const failed = $derived(steps.filter((step) => step.state === 'failed').length);
  const passed = $derived(steps.filter((step) => step.state === 'done').length);
  const took = $derived(Steps.took(steps));
  /** The longest step's time: each row's bar is its share of it. */
  const longest = $derived(Math.max(0, ...steps.map((step) => step.took ?? 0)));
</script>

{#snippet outcome(step: Step)}
  {#if step.state === 'running'}<Glyph mark="running" label={true} />
  {:else if step.state === 'failed'}<Glyph
      mark="failed"
      label={step.failure ? `Failed, ${step.failure}` : true}
    />
  {:else if step.state === 'declined'}<Glyph mark="idle" label="Declined" />
  {:else}<Glyph mark="passed" label="Done" />{/if}
{/snippet}

<section class="steps" class:failed={state === 'failed'}>
  <button class="steps-line" aria-expanded={open} onclick={() => ontoggle(!open)}>
    <span class="fold" class:open><Icon name="chevron" size={10} /></span>
    <span class="summary">{summary}</span>
    {#if state === 'running'}<Glyph mark="running" label={true} />{/if}
    <span class="tally mono">
      {#if failed}<span class="count"
          ><Glyph mark="failed" label={`${failed} failed`} />{failed}</span
        >{/if}
      {#if passed}<span class="count"><Glyph mark="passed" label={`${passed} done`} />{passed}</span
        >{/if}
    </span>
    <span class="time mono muted">{took === null ? '' : Elapsed.stopwatch(took)}</span>
  </button>
  {#if open}
    <ol class="step-list">
      {#each steps as step (step.entry.item)}
        <li class:picked={picked.has(step.entry.item)}>
          <input
            class="pick"
            type="checkbox"
            checked={picked.has(step.entry.item)}
            onchange={(event) => onpick(step, event.currentTarget.checked)}
            aria-label="Pick {Steps.VERBS[step.act]} {step.targets.join(', ')}"
          />
          <button
            class="step-row"
            class:chosen={chosen === step.entry.item}
            aria-pressed={chosen === step.entry.item}
            onclick={() => onopen(step)}
          >
            <span
              class="verb"
              title={Steps.VERBS[step.act]}
              aria-label={Steps.VERBS[step.act]}
              role="img"><Icon name={Steps.ICONS[step.act]} size={12} /></span
            >
            <span class="what">
              {#if step.said}<span class="said">{step.said}</span>{/if}
              <span class="target mono" class:muted={step.said !== null}
                >{step.targets.join(', ')}</span
              >
            </span>
            <span class="outcome">{@render outcome(step)}</span>
            <span class="track"
              ><Meter
                share={step.took !== null && longest > 0 ? step.took / longest : 0}
                least={0.02}
                tone={step.state === 'failed'
                  ? 'error'
                  : step.state === 'running'
                    ? 'live'
                    : 'muted'}
              /></span
            >
            <span class="time mono muted"
              >{step.took === null ? '' : Elapsed.stopwatch(step.took)}</span
            >
          </button>
        </li>
      {/each}
    </ol>
  {/if}
</section>

<style>
  .steps {
    margin: 6px 16px;
    border: 1px solid var(--line);
    background: var(--surface);
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
    height: 28px;
    padding: 0 12px;
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
    color: var(--ink);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tally {
    display: inline-flex;
    flex: none;
    gap: 8px;
    margin-left: auto;
    font-size: 11px;
  }
  .count {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .time {
    flex: none;
    min-width: 40px;
    font-size: 11px;
    text-align: right;
  }
  .step-list {
    margin: 0;
    padding: 0;
    border-top: 1px solid var(--line);
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
  }
  li.picked {
    background: var(--surface-inset);
  }
  .pick {
    flex: none;
    width: 12px;
    height: 12px;
    margin: 0 0 0 12px;
    accent-color: var(--accent);
    opacity: 0.35;
  }
  li:hover .pick,
  li.picked .pick,
  .pick:focus-visible {
    opacity: 1;
  }
  .step-row {
    flex: 1;
    min-width: 0;
    display: grid;
    grid-template-columns: 14px minmax(0, 1fr) 14px 80px 40px;
    align-items: center;
    column-gap: 10px;
    width: 100%;
    height: 26px;
    padding: 0 12px 0 10px;
    border: 0;
    border-radius: 0;
    background: none;
    color: var(--ink);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .step-row:hover {
    background: var(--surface-hover);
  }
  .step-row.chosen {
    background: var(--surface-inset);
    box-shadow: inset 2px 0 var(--accent);
  }
  .verb {
    display: inline-flex;
    color: var(--muted);
  }
  .what {
    display: flex;
    gap: 8px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
  .said {
    flex: none;
    max-width: 50%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .target {
    min-width: 0;
    overflow: hidden;
    font-size: 11.5px;
    text-overflow: ellipsis;
  }
  .outcome {
    display: inline-flex;
    justify-content: center;
  }
  .track {
    display: block;
    width: 100%;
    --meter-track: var(--line);
  }
  @media (max-width: 760px) {
    .steps {
      margin-left: 16px;
    }
    .step-row {
      grid-template-columns: 14px minmax(0, 1fr) 14px 40px;
    }
    .track {
      display: none;
    }
  }
</style>
