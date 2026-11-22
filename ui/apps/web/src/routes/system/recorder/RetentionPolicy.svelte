<!--
  The retention policy's limits, and what retention has pruned so far.
-->
<script lang="ts">
  import {
    num,
    bytes,
    type RecorderMetrics,
    type RetentionPolicy,
    type StorageView,
  } from '#lib/api.ts';
  import Glyph from '#lib/Glyph.svelte';
  import RecorderBlock from './RecorderBlock.svelte';

  let {
    policy,
    recorder,
    usage,
  }: {
    policy: RetentionPolicy | undefined;
    recorder: RecorderMetrics | undefined;
    usage: StorageView | undefined;
  } = $props();
</script>

<RecorderBlock id="retention-title" title="Retention policy" size="grow">
  {#snippet aside()}
    {#if policy}<Glyph
        mark={policy.enabled ? 'passed' : 'idle'}
        label={policy.enabled ? 'Enabled' : 'Disabled'}
      />{/if}
  {/snippet}
  <div class="policy">
    <div>
      <span class="meta">History window</span>
      <span class="mono">{policy ? `${num(policy.max_age_seconds / 86400)} days` : '—'}</span>
    </div>
    <div>
      <span class="meta">Call limit</span>
      <span class="mono">{num(policy?.max_calls)}</span>
    </div>
    <div>
      <span class="meta">Storage limit</span>
      <span class="mono">{policy ? bytes(policy.max_storage_bytes) : '—'}</span>
    </div>
  </div>
  <div class="pruned">
    <div>
      <span class="meta">Pruned calls</span><span class="mono">{num(recorder?.pruned_calls)}</span>
    </div>
    <div>
      <span class="meta">Pruned chunks</span><span class="mono">{num(recorder?.pruned_chunks)}</span
      >
    </div>
    <div>
      <span class="meta">Calls with derived activity</span><span class="mono"
        >{num(usage?.digested_calls ?? undefined)}{#if usage}{' '}<span class="meta"
            >of {num(usage.calls)}</span
          >{/if}</span
      >
    </div>
  </div>
</RecorderBlock>

<style>
  .policy {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    border: 1px solid var(--line);
  }
  .policy > div {
    display: flex;
    flex-direction: column;
    padding: 8px 12px;
  }
  .policy > div + div {
    border-left: 1px solid var(--line);
  }
  .policy .mono,
  .pruned .mono {
    font-size: 14px;
  }
  .pruned {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px;
    margin-top: 14px;
  }
  .pruned > div {
    display: flex;
    flex-direction: column;
  }
</style>
