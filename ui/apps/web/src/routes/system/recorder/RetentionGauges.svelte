<!--
  The retention limits: how close each is to pruning, the nearest marked.
  While the counters are on their way, the gauges are outlined.
-->
<script lang="ts">
  import { Shares } from '#lib/display.ts';
  import { RecorderReport, type Gauge } from '#lib/recorder.ts';
  import Meter from '#lib/Meter.svelte';
  import Skeleton from '#lib/Skeleton.svelte';

  let {
    busy,
    report,
    outline,
  }: {
    /** The counters are on their way: it draws `outline`. */
    busy: boolean;
    report: RecorderReport | null;
    /** The limits' gauges, labelled, for their outline. */
    outline: Gauge[];
  } = $props();
</script>

{#if busy}
  <section class="rec-gauges" aria-label="Retention limits" aria-busy="true">
    {#each outline as gauge (gauge.label)}
      <div class="rec-gauge">
        <div class="rec-gauge-head"><span>{gauge.label}</span></div>
        <div class="mono rec-gauge-value"><Skeleton size="figure" width="9em" /></div>
        <Meter share={0} size="bar" />
      </div>
    {/each}
  </section>
{:else if report}
  <section class="rec-gauges" aria-label="Retention limits">
    {#each report.gauges as gauge (gauge.label)}
      <div class="rec-gauge" class:binding={gauge.binding}>
        <div class="rec-gauge-head">
          <span>{gauge.label}</span>
          {#if gauge.binding}<span class="rec-binding" title="Retention prunes for this limit first"
              >prunes first</span
            >{/if}
          <span class="mono end">{gauge.share === null ? '' : Shares.percent(gauge.share)}</span>
        </div>
        <div class="mono rec-gauge-value">
          <strong>{gauge.used}</strong> <span class="muted">of {gauge.limit}</span>
        </div>
        <Meter
          share={gauge.share ?? 0}
          size="bar"
          tone={RecorderReport.meterTone(gauge.share ?? 0)}
        />
      </div>
    {/each}
  </section>
{/if}

<style>
  .rec-gauges {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 1px;
    border-bottom: 1px solid var(--control-border);
    background: var(--control-border);
  }
  .rec-gauge {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    padding: 10px 16px 12px;
    background: var(--surface);
  }
  .rec-gauge-head {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--muted);
    font-size: 11.5px;
  }
  .rec-gauge-head .end {
    margin-left: auto;
  }
  .rec-binding {
    padding: 0 5px;
    border: 1px solid currentColor;
    border-radius: 2px;
    color: var(--ink);
    font-size: 10.5px;
  }
  .rec-gauge-value {
    overflow: hidden;
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rec-gauge-value strong {
    font-size: 18px;
    font-weight: 500;
  }
</style>
