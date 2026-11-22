<!--
  The Live page's band: output tokens per second over the last
  `Band.spanMs`, by client, provider, or model, drawn as a ribbon or a row
  per series, with each series' share in the legend beside it. As an
  outline while the page's reads are on their way. The series under the
  pointer is the page's: the band reports it through `onhover`.
-->
<script lang="ts">
  import { Band, LivePlace, type BandBy, type BandSeries, type LiveView } from '#lib/board.ts';
  import { compactOrDash } from '#lib/display.ts';
  import Skeleton from '#lib/Skeleton.svelte';
  import { Elapsed } from '#lib/time.ts';

  type Props =
    /** The page's reads are on their way. */
    | { kind: 'outline'; by: BandBy }
    | {
        kind: 'band';
        band: Band;
        view: LiveView;
        /** The series under the pointer: its layer stands out. */
        hovered: string | null;
        onhover: (key: string | null) => void;
      };
  let props: Props = $props();

  /** The band's height in SVG units. */
  const HEIGHT = 120;
  const axis = Band.axis();
  /** The rules between the axis' divisions, across a 1000-wide box. */
  const ticks = axis.slice(1, -1).map((division) => division.at * 1000);
</script>

{#snippet rules()}
  {#each ticks as x (x)}
    <line class="live-band-tick" x1={x} y1="0" x2={x} y2={HEIGHT} />
  {/each}
{/snippet}

{#snippet axisLabels()}
  <div class="live-band-axis mono" aria-hidden="true">
    {#each axis as division (division.at)}<span>{division.label}</span>{/each}
  </div>
{/snippet}

{#snippet marks(series: BandSeries, top: number)}
  {#each series.marks as mark (mark.key)}
    <rect
      class="live-band-mark {mark.tone}"
      x={mark.at * 1000 - 1.5}
      y={top + 2}
      width="3"
      height="9"><title>{series.label}: {mark.label}</title></rect
    >
  {/each}
{/snippet}

{#if props.kind === 'outline'}
  <section class="live-band" aria-label="Output tokens by {props.by}" aria-busy="true">
    <svg
      class="live-band-chart"
      viewBox="0 0 1000 {HEIGHT}"
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      {@render rules()}
    </svg>
    <div class="live-legend" aria-hidden="true">
      {#each ['6em', '4.5em'] as width (width)}
        <span class="live-legend-chip"
          ><Skeleton {width} /><span class="mono"><Skeleton width="3.5em" /></span></span
        >
      {/each}
    </div>
    {@render axisLabels()}
  </section>
{:else if props.band.series.length}
  {@const { band, view, hovered, onhover } = props}
  <section class="live-band" aria-label="Output tokens by {view.by}">
    <div class="live-band-main">
      <div class="live-band-head">
        <span class="cap">Output tokens per second · last {Elapsed.span(Band.spanMs)}</span>
        <span class="live-band-rate mono">{Band.rateText(band.rate)}</span>
        <span class="meta"
          >tok/s now · peak {view.band === 'ribbon' ? band.ribbonMax : band.max}</span
        >
      </div>
      <div class="live-band-plot">
        <svg
          class="live-band-chart"
          class:hovering={hovered !== null}
          viewBox="0 0 1000 {HEIGHT}"
          preserveAspectRatio="none"
          aria-hidden="true"
        >
          {@render rules()}
          {#if band.unread > 0}
            <rect class="live-band-unread" x="0" y="0" width={band.unread * 1000} height={HEIGHT} />
          {/if}
          {#if view.band === 'ribbon'}
            <line class="live-band-tick" x1="0" y1={HEIGHT / 2} x2="1000" y2={HEIGHT / 2} />
            {#each band.series as series, index (series.key)}
              {#if series.bins.some((value) => value > 0)}
                {@const layer = band.layer(index, HEIGHT)}
                <g class={series.swatch} class:lit={hovered === series.key}>
                  <path class="live-band-area" d={layer.area} />
                  <path class="live-band-line" d={layer.line} />
                </g>
              {/if}
            {/each}
            {#each band.series as series (series.key)}
              {@render marks(series, 0)}
            {/each}
          {:else}
            {#each band.series as series, index (series.key)}
              {@const row = band.row(index, HEIGHT)}
              <g class={series.swatch} class:lit={hovered === series.key}>
                {#if index > 0}<line
                    class="live-band-rule"
                    x1="0"
                    y1={row.top}
                    x2="1000"
                    y2={row.top}
                  />{/if}
                <path class="live-band-area" d={row.area} />
                <path class="live-band-line" d={row.line} />
              </g>
              {@render marks(series, row.top)}
            {/each}
          {/if}
        </svg>
      </div>
      {@render axisLabels()}
    </div>
    <div class="live-band-side">
      <span class="cap">By {view.by} · last {Elapsed.span(Band.spanMs)}</span>
      {#each band.series as series (series.key)}
        <a
          class="live-legend-row {series.swatch}"
          class:chosen={view.series === series.key}
          href={LivePlace.href(LivePlace.toggleSeries(view, series.key))}
          title={view.series === series.key
            ? 'Show every session'
            : `Show only ${series.label} sessions`}
          onpointerenter={() => onhover(series.key)}
          onpointerleave={() => onhover(null)}
          onfocus={() => onhover(series.key)}
          onblur={() => onhover(null)}
          ><span class="harness-mark"></span><span class="ellipsis">{series.label}</span><span
            class="mono">{compactOrDash(Band.seriesTokens(series))}</span
          ><span class="mono muted share">{band.share(series)}</span></a
        >
      {/each}
      {#if band.series.some((series) => series.marks.length) || band.unread > 0}
        <span class="live-legend-key" aria-hidden="true"
          >{#if band.series.some((series) => series.marks.length)}<span class="live-key-mark error"
            ></span>failed<span class="live-key-mark warning"></span>cut off{/if}{#if band.unread > 0}<span
              class="live-key-mark unread"
            ></span>not read{/if}</span
        >
      {/if}
    </div>
  </section>
{:else}
  <section class="live-band empty" aria-label="Output tokens">
    <span class="mono">No output in the last {Elapsed.span(Band.spanMs)}</span>
  </section>
{/if}

<style>
  /* Providers and models take a slot's colour; clients keep their own. */
  .series-0 {
    --harness: var(--accent);
  }
  .series-1 {
    --harness: var(--harness-codex);
  }
  .series-2 {
    --harness: var(--role-tool);
  }
  .series-3 {
    --harness: var(--role-thinking);
  }
  .series-other {
    --harness: var(--muted);
  }

  /* The band: output tokens as calls finish, with each series' share beside it. */
  .live-band {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    gap: 1px;
    border-bottom: 1px solid var(--line);
    background: var(--line);
  }
  .live-band-main {
    flex: 999 1 640px;
    min-width: 0;
    padding: 10px 16px 6px;
    background: var(--surface);
  }
  .live-band-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .live-band-head .cap,
  .live-band-side .cap {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .live-band-rate {
    margin-left: 8px;
    font-size: 18px;
    font-weight: 500;
  }
  .live-band-head .meta {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11px;
  }
  .live-band-plot {
    position: relative;
    height: 92px;
    margin-top: 8px;
    background: var(--surface-inset);
  }
  .live-band-side {
    display: flex;
    flex: 1 1 260px;
    flex-direction: column;
    gap: 2px;
    padding: 10px 16px;
    background: var(--surface);
  }
  .live-band-side .cap {
    margin-bottom: 4px;
  }
  .live-legend-row {
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) auto 36px;
    column-gap: 8px;
    align-items: center;
    height: 24px;
    color: var(--ink);
    font-size: 12px;
  }
  .live-legend-row:hover,
  .live-legend-row.chosen {
    background: var(--row-hover);
  }
  .live-legend-row .harness-mark {
    width: 8px;
    height: 8px;
    margin: 0;
    border-radius: 0;
    background: var(--harness);
  }
  .live-legend-row .mono {
    font-size: 11.5px;
  }
  .live-legend-row .share {
    color: var(--muted);
    text-align: right;
  }
  .live-band.empty {
    display: flex;
    align-items: center;
    height: 30px;
    padding: 0 16px;
    color: var(--muted);
    font-size: 11px;
  }
  .live-band-chart {
    display: block;
    width: 100%;
    height: 100%;
  }
  .live-band-unread {
    fill: var(--surface-inset);
    opacity: 0.7;
  }
  .live-band-tick {
    stroke: var(--line);
    stroke-dasharray: 2 3;
    vector-effect: non-scaling-stroke;
  }
  .live-band-rule {
    stroke: var(--line);
    vector-effect: non-scaling-stroke;
  }
  .live-band-area {
    fill: var(--harness);
    opacity: 0.28;
  }
  .live-band-line {
    fill: none;
    stroke: var(--harness);
    stroke-width: 1.4;
    vector-effect: non-scaling-stroke;
  }
  .live-band-chart g {
    transition: opacity 120ms;
  }
  .live-band-chart.hovering g:not(.lit) {
    opacity: 0.25;
  }
  .live-band-mark.warning {
    fill: var(--warning);
  }
  .live-legend-key {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--muted);
    font-size: 11px;
  }
  .live-key-mark {
    display: inline-block;
    width: 3px;
    height: 9px;
    margin-left: 6px;
  }
  .live-key-mark.error {
    background: var(--error);
  }
  .live-key-mark.warning {
    background: var(--warning);
  }
  .live-key-mark.unread {
    width: 9px;
    border: 1px solid var(--line);
    background: var(--surface-inset);
  }
  .live-band-mark.error {
    fill: var(--error);
  }
  div.live-band-axis.mono {
    display: flex;
    justify-content: space-between;
    padding-top: 3px;
    color: var(--muted);
    font-size: 10px;
  }
</style>
