<script lang="ts">
  // One series of columns over time. Geometry is SVG attributes only, so the
  // strict CSP (no inline styles) holds; the tooltip is drawn inside the SVG.
  import { axisTicks, measureValue, type UsageMeasure, type UsagePoint } from './usage.ts';
  import { compact } from './display.ts';
  let {
    points,
    measure,
    label,
    period,
  }: {
    points: UsagePoint[];
    measure: UsageMeasure;
    /** The measure's name, for the tooltip and the table. */
    label: string;
    /** How a bar's period reads: an hour or a day. */
    period: (start: number) => { short: string; long: string };
  } = $props();

  const HEIGHT = 200;
  const AXIS = 24;
  const GUTTER = 48;
  const TOP = 12;
  /** A bar's share of its slot; the rest is the gap between bars. */
  const BAR = 0.6;
  /** The most period labels under the bars. */
  const LABELS = 8;
  // The plot is laid out in fractions of its own width, so the server draws
  // it as the browser will: nothing waits on a measured width. Only the
  // tooltip, shown on hover, keeps to the plot's measured edges.
  let width = $state(640);
  const values = $derived(points.map((point) => measureValue(point, measure)));
  const ticks = $derived(axisTicks(Math.max(0, ...values)));
  const top = $derived(ticks.at(-1) ?? 1);
  const slots = $derived(Math.max(1, points.length));
  /** A slot's left edge, or a point within it, as a percentage of the plot. */
  const at = (index: number, within = 0) => `${((index + within) / slots) * 100}%`;
  const y = (value: number) => TOP + (HEIGHT - TOP) * (1 - value / top);
  // Label every bar when they fit, else every few: never more than LABELS.
  const every = $derived(Math.max(1, Math.ceil(points.length / LABELS)));
  let active = $state<number | null>(null);

  const full = (value: number) => value.toLocaleString();
  const tip = $derived.by(() => {
    if (active === null) return null;
    const value = full(values[active]);
    const when = period(points[active].start).long;
    const textWidth = Math.max(value.length * 8, when.length * 6.4) + 20;
    const centre = ((active + 0.5) / slots) * width;
    // Shifted from the bar's centre so it stays inside the plot.
    const left = Math.min(Math.max(centre - textWidth / 2, 0), Math.max(0, width - textWidth - 2));
    const yAt = Math.max(0, Math.min(y(values[active]) - 52, HEIGHT - 50));
    return { value, when, centre: at(active, 0.5), shift: left - centre, y: yAt, width: textWidth };
  });
</script>

<div class="usage-chart">
  <div class="usage-plot">
    <svg class="usage-axis-column" width={GUTTER} height={HEIGHT + AXIS} aria-hidden="true">
      {#each ticks as tick}
        <text class="usage-axis" x={GUTTER - 8} y={y(tick) + 4} text-anchor="end"
          >{compact(tick)}</text
        >
      {/each}
    </svg>
    <!-- Measured only for the tooltip; the plot itself is in percentages. -->
    <div class="usage-bars" bind:clientWidth={width}>
      <svg
        width="100%"
        height={HEIGHT + AXIS}
        role="img"
        aria-label={`${label} per period; the table below lists every value`}
      >
        {#each ticks as tick}
          <line class="usage-grid" x1="0" x2="100%" y1={y(tick)} y2={y(tick)} />
        {/each}
        {#each points as point, index (point.start)}
          {#if HEIGHT - y(values[index]) > 0}<rect
              class="usage-bar"
              class:active={active === index}
              x={at(index, (1 - BAR) / 2)}
              width={`${(BAR / slots) * 100}%`}
              y={y(values[index])}
              height={HEIGHT - y(values[index])}
            />{/if}
          {#if index % every === 0}
            <text class="usage-axis" x={at(index, 0.5)} y={HEIGHT + 16} text-anchor="middle"
              >{period(point.start).short}</text
            >
          {/if}
          <!-- The whole slot is the target, not just the painted bar. -->
          <rect
            class="usage-hit"
            x={at(index)}
            y={TOP}
            width={`${100 / slots}%`}
            height={HEIGHT - TOP}
            tabindex="0"
            role="button"
            aria-label={`${period(point.start).long}: ${full(values[index])} ${label.toLowerCase()}`}
            onpointerenter={() => (active = index)}
            onpointerleave={() => (active = null)}
            onfocus={() => (active = index)}
            onblur={() => (active = null)}
          />
        {/each}
        {#if tip}
          <svg x={tip.centre} y={tip.y} overflow="visible" aria-hidden="true">
            <g class="usage-tip" transform="translate({tip.shift},0)">
              <rect width={tip.width} height="44" />
              <text x="10" y="19" class="usage-tip-value">{tip.value}</text>
              <text x="10" y="35" class="usage-tip-label">{tip.when}</text>
            </g>
          </svg>
        {/if}
      </svg>
    </div>
  </div>
  <details class="usage-table">
    <summary>Show as table</summary>
    <table>
      <thead><tr><th>Period</th><th class="numeric">{label}</th></tr></thead>
      <tbody>
        {#each points as point, index (point.start)}
          <tr
            ><td>{period(point.start).long}</td><td class="mono numeric">{full(values[index])}</td
            ></tr
          >
        {/each}
      </tbody>
    </table>
  </details>
</div>
