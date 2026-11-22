<!--
  A recording's calls as their prompts grew: each call a bar stacked from
  what was read from the cache, what was written to it, and what was new,
  against the page's largest; each turn shaded behind its calls; a failed
  call marked under its bar. A bar opens its call.
-->
<script lang="ts">
  import { type CallId, type CallView } from '#lib/api.ts';
  import { compactOrDash } from '#lib/display.ts';
  import { CallTimeline } from '#lib/timeline.ts';

  let {
    calls,
    selected,
    href,
    oncall,
  }: {
    calls: readonly CallView[];
    selected: CallId | null;
    href: (call: CallId) => string;
    oncall: (event: MouseEvent, call: CallId) => void;
  } = $props();

  const chart = $derived(CallTimeline.contextChart(calls));
  /** Each bar's slot, in the chart's 0–width units. */
  const SLOT = 10;
  const width = $derived(Math.max(1, chart.bars.length) * SLOT);
</script>

{#if chart.bars.length}
  <section class="context-chart" aria-label="Context per call">
    <div class="head">
      <span class="cap">Context per call · turns shaded</span>
      <span class="key"><span class="swatch read"></span>from cache</span>
      <span class="key"><span class="swatch written"></span>written to cache</span>
      <span class="key"><span class="swatch fresh"></span>new input</span>
      <span class="key"><span class="swatch failed"></span>failed</span>
      <span class="muted peak"
        >peak <span class="mono">{compactOrDash(chart.peak)}</span> · model limit not reported</span
      >
    </div>
    <svg viewBox="0 0 {width} 112" preserveAspectRatio="none" class="plot">
      {#each chart.bands as band, index (index)}<rect
          class="band"
          class:odd={index % 2 === 1}
          x={band.from * SLOT}
          y="0"
          width={(band.to - band.from) * SLOT}
          height="112"
        />{/each}
      {#each chart.bars as bar, index (bar.call.id)}{@const x = index * SLOT + 1.5}<a
          href={href(bar.call.id)}
          onclick={(event) => oncall(event, bar.call.id)}
          aria-label="Call {index + 1}: {Math.round(
            (bar.read + bar.written + bar.fresh) * chart.peak,
          ).toLocaleString()} tokens"
          ><rect class="hit" {x} y="12" width={SLOT - 3} height="96" /><rect
            class="read"
            class:chosen={bar.call.id === selected}
            {x}
            y={108 - bar.read * 94}
            width={SLOT - 3}
            height={bar.read * 94}
          /><rect
            class="written"
            {x}
            y={108 - (bar.read + bar.written) * 94}
            width={SLOT - 3}
            height={bar.written * 94}
          /><rect
            class="fresh"
            {x}
            y={108 - (bar.read + bar.written + bar.fresh) * 94}
            width={SLOT - 3}
            height={bar.fresh * 94}
          />{#if bar.failed}<rect class="failed" {x} y="109" width={SLOT - 3} height="3" />{/if}</a
        >{/each}
    </svg>
  </section>
{/if}

<style>
  .context-chart {
    flex: none;
    padding: 10px 16px 12px;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
    font-size: 11px;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 14px;
    margin-bottom: 8px;
  }
  .cap {
    color: var(--muted);
    font-size: 10.5px;
    font-weight: 500;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .key {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--muted);
  }
  .peak {
    margin-left: auto;
  }
  .swatch {
    width: 8px;
    height: 8px;
  }
  .plot {
    display: block;
    width: 100%;
    height: 120px;
    background: var(--surface-inset);
  }
  .band {
    fill: transparent;
  }
  .band.odd {
    fill: color-mix(in srgb, var(--ink) 4%, transparent);
  }
  .hit {
    fill: transparent;
  }
  .read {
    fill: var(--cache-read);
    background: var(--cache-read);
  }
  .written {
    fill: var(--cache-written);
    background: var(--cache-written);
  }
  .fresh {
    fill: var(--input-fresh);
    background: var(--input-fresh);
  }
  .failed {
    fill: var(--error);
    background: var(--error);
  }
  a:hover .read,
  .read.chosen {
    fill: var(--accent);
  }
</style>
