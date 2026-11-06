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
  let width = $state(640);
  const values = $derived(points.map((point) => measureValue(point, measure)));
  const ticks = $derived(axisTicks(Math.max(0, ...values)));
  const top = $derived(ticks.at(-1) ?? 1);
  const plot = $derived(Math.max(1, width - GUTTER - 8));
  const band = $derived(plot / Math.max(1, points.length));
  const barWidth = $derived(Math.max(2, Math.min(24, band - 2)));
  const y = (value: number) => TOP + (HEIGHT - TOP) * (1 - value / top);
  // Label every bar when they fit, else every few: never more than ~8.
  const every = $derived(
    Math.max(1, Math.ceil(points.length / Math.max(1, Math.floor(plot / 64)))),
  );
  let active = $state<number | null>(null);

  /** A square column on the baseline. */
  function bar(index: number): string {
    const value = values[index];
    const x = GUTTER + index * band + (band - barWidth) / 2;
    const yTop = y(value);
    return HEIGHT - yTop <= 0 ? '' : `M${x},${HEIGHT} V${yTop} H${x + barWidth} V${HEIGHT} Z`;
  }
  const full = (value: number) => value.toLocaleString();
  const tip = $derived.by(() => {
    if (active === null) return null;
    const value = full(values[active]);
    const when = period(points[active].start).long;
    const textWidth = Math.max(value.length * 8, when.length * 6.4) + 20;
    const centre = GUTTER + active * band + band / 2;
    const x = Math.min(Math.max(centre - textWidth / 2, GUTTER), width - textWidth - 2);
    const yAt = Math.max(0, Math.min(y(values[active]) - 52, HEIGHT - 50));
    return { value, when, x, y: yAt, width: textWidth };
  });
</script>

<div class="usage-chart">
  <!-- Measured without the padding, so the plot never runs past the panel. -->
  <div class="usage-plot" bind:clientWidth={width}>
    <svg
      {width}
      height={HEIGHT + AXIS}
      viewBox="0 0 {width} {HEIGHT + AXIS}"
      role="img"
      aria-label={`${label} per period; the table below lists every value`}
    >
      {#each ticks as tick}
        <line class="usage-grid" x1={GUTTER} x2={width - 4} y1={y(tick)} y2={y(tick)} />
        <text class="usage-axis" x={GUTTER - 8} y={y(tick) + 4} text-anchor="end"
          >{compact(tick)}</text
        >
      {/each}
      {#each points as point, index (point.start)}
        <path class="usage-bar" class:active={active === index} d={bar(index)} />
        {#if index % every === 0}
          <text
            class="usage-axis"
            x={GUTTER + index * band + band / 2}
            y={HEIGHT + 16}
            text-anchor="middle">{period(point.start).short}</text
          >
        {/if}
        <!-- The whole slot is the target, not just the painted bar. -->
        <rect
          class="usage-hit"
          x={GUTTER + index * band}
          y={TOP}
          width={band}
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
        <g class="usage-tip" transform="translate({tip.x},{tip.y})" aria-hidden="true">
          <rect width={tip.width} height="44" />
          <text x="10" y="19" class="usage-tip-value">{tip.value}</text>
          <text x="10" y="35" class="usage-tip-label">{tip.when}</text>
        </g>
      {/if}
    </svg>
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
