<!--
  A page's outline while its first read is on its way and this browser has
  none of it yet: a row of figures, then rows, where the page's own will go.
  A page whose read failed says why in its place.
-->
<script lang="ts">
  import Skeleton from './Skeleton.svelte';
  import type { SnapshotError } from './snapshots.ts';

  let {
    figures = 4,
    rows = 6,
    error = null,
  }: {
    figures?: number;
    rows?: number;
    /** Why the read failed; the outline gives way to it. */
    error?: SnapshotError | null;
  } = $props();

  /** Text widths, varied so the rows read as text rather than stripes. */
  const widths = ['62%', '48%', '71%', '39%', '55%', '66%'];
</script>

{#if error}
  <p class="error" role="alert">{error.message}</p>
{:else}
  <div class="outline" aria-busy="true" aria-label="Loading">
    {#if figures}
      <div class="figures">
        {#each { length: figures }, index (index)}
          <div><Skeleton width="6em" /><Skeleton size="figure" width="3em" /></div>
        {/each}
      </div>
    {/if}
    <div class="rows">
      {#each { length: rows }, index (index)}
        <div class="row"><Skeleton width={widths[index % widths.length]} /></div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .outline {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 1px;
    min-height: 0;
    background: var(--line);
  }
  .figures {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
    gap: 1px;
  }
  .figures > div {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 16px;
    background: var(--surface);
  }
  .rows {
    flex: 1;
    background: var(--surface);
  }
  .row {
    display: flex;
    align-items: center;
    min-height: 32px;
    padding: 0 16px;
    border-top: 1px solid var(--line-subtle);
  }
  .error {
    margin: 0;
    padding: 16px;
    background: var(--surface);
    color: var(--error);
  }
</style>
