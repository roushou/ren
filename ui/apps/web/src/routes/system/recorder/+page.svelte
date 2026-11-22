<script lang="ts">
  import { Now } from '#lib/now.svelte.ts';
  import Toolbar from '#lib/Toolbar.svelte';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { buildLabel } from '#lib/display.ts';
  import { RecorderReport } from '#lib/recorder.ts';
  import RecorderVerdict from './RecorderVerdict.svelte';
  import RetentionGauges from './RetentionGauges.svelte';
  import BodyFunnel from './BodyFunnel.svelte';
  import DiskUsage from './DiskUsage.svelte';
  import RetentionPolicy from './RetentionPolicy.svelte';
  import RecorderQueue from './RecorderQueue.svelte';
  import CommitTimes from './CommitTimes.svelte';
  import LossAndErrors from './LossAndErrors.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  const gateway = createArrival(() => data.gateway);
  const arrival = createArrival(() => data.recorder);
  const shown = $derived(arrival.current);
  /** The counters are on their way: the page draws its outline. */
  const pending = $derived(shown.state === 'pending');
  const snapshot = $derived(shown.state === 'ready' ? shown.snapshot : null);
  const policy = $derived(gateway.snapshot?.value?.storage.policy);
  // `/api/metrics` is fresher; `/health` keeps counters visible if storage reads fail.
  // While the metrics are on their way, their places are outlined instead.
  const recorder = $derived(
    pending
      ? undefined
      : (snapshot?.value?.metrics.recorder ?? gateway.snapshot?.value?.health.recorder),
  );
  const usage = $derived(
    pending
      ? undefined
      : (snapshot?.value?.metrics.storage ?? gateway.snapshot?.value?.storage.usage),
  );
  /** History's age moves with the clock. */
  const clock = new Now(60_000);
  const report = $derived(
    recorder
      ? new RecorderReport(
          recorder,
          usage ?? null,
          policy ?? null,
          snapshot?.value?.oldest ?? null,
          clock.current,
        )
      : null,
  );
  /** The limits' gauges, labelled, for their outline. */
  const gaugeOutline = $derived(RecorderReport.gauges(null, policy ?? null, null, clock.current));
</script>

{#snippet scope()}
  <div class="scope-group">
    <span class="meta"
      >daemon build {buildLabel(gateway.snapshot?.value?.health.build ?? null)}</span
    >
  </div>
{/snippet}

<Toolbar {scope} />
<div class="recorder-console">
  <RecorderVerdict busy={pending} {report} {usage} />
  <RetentionGauges busy={pending} {report} outline={gaugeOutline} />
  <div class="storage-grid">
    <div class="rec-column">
      <BodyFunnel busy={pending} {usage} />
      <DiskUsage busy={pending} {usage} />
      <RetentionPolicy {policy} {recorder} {usage} />
    </div>

    <div class="rec-column side">
      <RecorderQueue busy={pending} {recorder} />
      <CommitTimes busy={pending} {recorder} />
      <LossAndErrors busy={pending} {report} />
    </div>
  </div>
</div>

<style>
  .recorder-console {
    width: 100%;
    border-bottom: 1px solid var(--control-border);
    flex-shrink: 0;
  }
  .storage-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 420px;
    gap: 1px;
    background: var(--line);
  }
  .rec-column {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  @media (max-width: 1100px) {
    .storage-grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
