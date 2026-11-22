<!--
  The recorder queue against its record and byte limits.
-->
<script lang="ts">
  import type { RecorderMetrics } from '#lib/api.ts';
  import { RecorderReport } from '#lib/recorder.ts';
  import Meter from '#lib/Meter.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import RecorderBlock from './RecorderBlock.svelte';

  let { busy, recorder }: { busy: boolean; recorder: RecorderMetrics | undefined } = $props();
</script>

<RecorderBlock id="queue-title" title="Recorder health · queue" {busy}>
  {#if recorder}
    {#each RecorderReport.queueMeters(recorder) as meter (meter.label)}
      <div class="queue">
        <span>{meter.label}</span><span class="mono">{meter.text}</span>
        <Meter
          share={meter.share}
          size="bar"
          least={0.005}
          tone={RecorderReport.meterTone(meter.share)}
        />
      </div>
    {/each}
  {:else}<Skeleton width="100%" />{/if}
</RecorderBlock>

<style>
  .queue {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    row-gap: 4px;
    margin-bottom: 10px;
    font-size: 12.5px;
  }
  .queue :global(.meter) {
    grid-column: 1 / -1;
  }
</style>
