<!--
  A conversation of more than one turn at a glance, above it: each turn as
  a bar as wide as its share of the time and as tall as its share of the
  steps, then as a line of the outline. Either scrolls the conversation to
  its turn, keeping the address as it is.
-->
<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import { Elapsed } from '#lib/time.ts';
  import { TurnLines, type TurnLine } from '#lib/steps.ts';

  let {
    lines,
    onturn,
  }: {
    lines: TurnLine[];
    /** The conversation was scrolled to a turn. */
    onturn: () => void;
  } = $props();

  /** The ruler's height, in the units its bars are drawn in. */
  const RULER = 36;
  const bars = $derived(TurnLines.ruler(lines, RULER));
  const failed = $derived(lines.reduce((sum, line) => sum + line.failed, 0));
  const steps = $derived(lines.reduce((sum, line) => sum + line.steps, 0));

  /** Scroll the conversation to a turn, keeping the address as it is. */
  function toTurn(event: MouseEvent, index: number) {
    event.preventDefault();
    document
      .getElementById(`turn-${index}`)
      ?.scrollIntoView({ block: 'start', behavior: 'smooth' });
    onturn();
  }
</script>

<section class="turn-ruler" aria-label="Turns">
  <div class="ruler-head">
    <span class="cap">Turns</span>
    <span class="muted mono"
      >{lines.length} turns · {steps} steps{#if failed}{' '}·
        <Glyph mark="failed" label="{failed} failed" />{failed}{/if}</span
    >
  </div>
  <svg viewBox="0 0 100 {RULER}" preserveAspectRatio="none" role="img" aria-label="Turns by time">
    {#each bars as bar (bar.index)}
      <a href="#turn-{bar.index}" onclick={(event) => toTurn(event, bar.index)}
        ><title>{lines[bar.index]?.prompt || 'Turn'} · {lines[bar.index]?.steps} steps</title><rect
          class="ruler-bar {bar.state}"
          x={bar.x + 0.1}
          width={Math.max(0.2, bar.width - 0.2)}
          y={RULER - bar.height}
          height={bar.height}
        /></a
      >
    {/each}
  </svg>
  <ol class="turn-outline">
    {#each lines as line (line.index)}
      <li>
        <a href="#turn-{line.index}" onclick={(event) => toTurn(event, line.index)}>
          <span class="outline-prompt">{line.prompt || 'Turn'}</span>
          <span class="muted mono"
            >{line.steps} steps{#if line.failed}{' '}·
              <Glyph mark="failed" label="{line.failed} failed" />{line.failed}{/if}</span
          >
          <span class="muted mono">{line.ms === null ? '' : Elapsed.span(line.ms)}</span>
        </a>
      </li>
    {/each}
  </ol>
</section>

<style>
  .turn-ruler {
    flex: none;
    padding: 8px 16px 6px;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
  }
  .ruler-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 6px;
    font-size: 11px;
  }
  .ruler-head .cap {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .turn-ruler svg {
    display: block;
    width: 100%;
    height: 36px;
    background: var(--surface-inset);
  }
  .ruler-bar.done {
    fill: color-mix(in srgb, var(--muted) 55%, transparent);
  }
  .ruler-bar.failed {
    fill: var(--error);
  }
  .ruler-bar.running {
    fill: var(--accent);
  }
  .turn-ruler a:hover .ruler-bar {
    fill: var(--ink);
  }
  .turn-outline {
    max-height: 104px;
    margin: 6px 0 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }
  .turn-outline a {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 90px;
    column-gap: 12px;
    align-items: center;
    height: 24px;
    white-space: nowrap;
    color: inherit;
    font-size: 12px;
  }
  .turn-outline a:hover {
    color: var(--accent);
  }
  .turn-outline .mono {
    font-size: 11px;
    text-align: right;
  }
  .outline-prompt {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
</style>
