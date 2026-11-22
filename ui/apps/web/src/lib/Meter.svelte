<!--
  How much of something is used or left: a track, and a fill from its start
  as long as `share` (0–1) of it. `least` keeps anything above nothing in
  sight. Its tone says how it stands; the reading itself is the host's to
  print beside it.
-->
<script lang="ts" module>
  /** The fill's colour: a quantity's own, or how it stands. */
  export type MeterTone = 'chart' | 'cache' | 'live' | 'muted' | 'success' | 'warning' | 'error';
</script>

<script lang="ts">
  let {
    share,
    tone = 'chart',
    size = 'thin',
    least = 0,
    label,
  }: {
    share: number;
    tone?: MeterTone;
    /** Its height: 2, 4, or 6 px. */
    size?: 'hair' | 'thin' | 'bar';
    /** The smallest share drawn for anything above nothing. */
    least?: number;
    /** What it measures, when nothing beside it says. */
    label?: string;
  } = $props();

  const drawn = $derived(share > 0 ? Math.max(least, Math.min(1, share)) : 0);
</script>

<svg
  class="meter {size} {tone}"
  viewBox="0 0 100 1"
  preserveAspectRatio="none"
  role={label ? 'img' : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : true}
  ><rect class="track" width="100" height="1" />{#if drawn}<rect
      class="fill"
      width={drawn * 100}
      height="1"
    />{/if}</svg
>

<style>
  .meter {
    display: block;
    width: 100%;
  }
  .hair {
    height: 2px;
  }
  .thin {
    height: 4px;
  }
  .bar {
    height: 6px;
  }
  .track {
    fill: var(--meter-track, var(--surface-inset));
  }
  .chart .fill {
    fill: var(--chart);
  }
  .cache .fill {
    fill: var(--cache-read);
  }
  .live .fill {
    fill: var(--live);
  }
  .muted .fill {
    fill: var(--muted);
  }
  .success .fill {
    fill: var(--success);
  }
  .warning .fill {
    fill: var(--warning);
  }
  .error .fill {
    fill: var(--error);
  }
</style>
