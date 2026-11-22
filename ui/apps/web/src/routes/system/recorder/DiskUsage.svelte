<!--
  The database files' bytes on disk: each file's live and free pages, as
  one bar and its key.
-->
<script lang="ts">
  import { bytes, type StorageView } from '#lib/api.ts';
  import { RecorderReport } from '#lib/recorder.ts';
  import Skeleton from '#lib/Skeleton.svelte';
  import RecorderBlock from './RecorderBlock.svelte';

  let { busy, usage }: { busy: boolean; usage: StorageView | undefined } = $props();

  const parts = $derived(usage ? RecorderReport.diskParts(usage) : []);
  const whole = $derived(
    Math.max(
      1,
      parts.reduce((sum, part) => sum + part.bytes, 0),
    ),
  );
</script>

<RecorderBlock id="disk-title" title={usage ? `Disk usage · ${bytes(whole)}` : 'Disk usage'} {busy}>
  {#if usage}
    <svg class="disk-bar" viewBox="0 0 100 10" preserveAspectRatio="none" aria-hidden="true"
      >{#each parts as part, index (part.key)}<rect
          class="disk-{part.key}"
          x={(parts.slice(0, index).reduce((sum, one) => sum + one.bytes, 0) / whole) * 100}
          width={(part.bytes / whole) * 100}
          height="10"><title>{part.label}: {bytes(part.bytes)}</title></rect
        >{/each}</svg
    >
    <div class="disk-key">
      {#each parts as part (part.key)}
        <div>
          <span class="meta"><span class="swatch disk-{part.key}"></span>{part.label}</span>
          <span class="mono">{bytes(part.bytes)}</span>
        </div>
      {/each}
    </div>
  {:else}
    <Skeleton width="100%" />
  {/if}
</RecorderBlock>

<style>
  .disk-bar {
    display: block;
    width: calc(100% - 24px);
    height: 8px;
    margin: 4px 12px 6px;
    background: var(--surface-inset);
  }
  .disk-meta-live {
    fill: var(--chart);
    background: var(--chart);
  }
  .disk-meta-free {
    fill: color-mix(in srgb, var(--chart) 35%, transparent);
    background: color-mix(in srgb, var(--chart) 35%, transparent);
  }
  .disk-blob-live {
    fill: var(--accent);
    background: var(--accent);
  }
  .disk-blob-free {
    fill: color-mix(in srgb, var(--accent) 35%, transparent);
    background: color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .swatch {
    width: 8px;
    height: 8px;
  }
  .disk-key {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 12px;
    margin-top: 8px;
  }
  .disk-key > div {
    display: flex;
    flex-direction: column;
  }
  .disk-key .mono {
    font-size: 14px;
  }
</style>
