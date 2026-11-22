<!--
  What a body becomes on disk: the bytes calls reference, the distinct
  chunks they dedupe to, those chunks compressed, and the ratio overall.
-->
<script lang="ts">
  import { num, bytes, type StorageView } from '#lib/api.ts';
  import { RecorderReport } from '#lib/recorder.ts';
  import Skeleton from '#lib/Skeleton.svelte';
  import RecorderBlock from './RecorderBlock.svelte';

  let { busy, usage }: { busy: boolean; usage: StorageView | undefined } = $props();
</script>

<RecorderBlock id="body-title" title="What a body becomes on disk" {busy}>
  {#snippet aside()}
    {#if usage}<span class="meta">{num(usage.chunks)} chunks</span>{/if}
  {/snippet}
  {#if usage}
    <div class="funnel">
      {#each RecorderReport.bodySteps(usage) as step (step.label)}
        <span class="funnel-label">{step.label}</span>
        <svg viewBox="0 0 100 10" preserveAspectRatio="none" aria-hidden="true"
          ><rect class="funnel-track" width="100" height="10" /><rect
            class="funnel-bar {step.tone}"
            width={(step.bytes / Math.max(1, usage.referenced_bytes)) * 100}
            height="10"
          /></svg
        >
        <span class="mono value"
          >{#if step.note}<span class="chip mono ratio" title={step.note.label}
              >{step.note.ratio}</span
            >{/if}{bytes(step.bytes)}</span
        >
      {/each}
      <span class="funnel-label">Overall</span><span></span><span
        class="mono value ok"
        title="What clients sent, against what is on disk"
        >{RecorderReport.ratio(usage.referenced_bytes, usage.compressed_bytes)}</span
      >
    </div>
  {:else}<Skeleton width="100%" />{/if}
</RecorderBlock>

<style>
  .funnel {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr) 160px;
    column-gap: 14px;
    row-gap: 4px;
    align-items: center;
  }
  .funnel svg {
    width: 100%;
    height: 22px;
  }
  .funnel-label {
    font-size: 12.5px;
  }
  .funnel .value {
    display: flex;
    gap: 8px;
    align-items: center;
    justify-content: flex-end;
  }
  .funnel-track {
    fill: var(--surface-inset);
  }
  .funnel-bar.referenced {
    fill: var(--input-fresh);
  }
  .funnel-bar.unique {
    fill: var(--cache-read);
  }
  .funnel-bar.compressed {
    fill: var(--accent);
  }
  @media (max-width: 1100px) {
    .funnel {
      grid-template-columns: 140px minmax(0, 1fr) 80px;
    }
  }
</style>
