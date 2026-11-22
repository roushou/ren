<!--
  How recording stands, in one row: the verdict, the figures behind it, each
  toned by its own limits, and the calls still open.
-->
<script lang="ts">
  import { num, type StorageView } from '#lib/api.ts';
  import { Marks } from '#lib/marks.ts';
  import type { RecorderReport } from '#lib/recorder.ts';
  import Glyph from '#lib/Glyph.svelte';
  import Meter from '#lib/Meter.svelte';
  import Skeleton from '#lib/Skeleton.svelte';

  let {
    busy,
    report,
    usage,
  }: {
    /** The counters are on their way: it draws its outline. */
    busy: boolean;
    report: RecorderReport | null;
    usage: StorageView | undefined;
  } = $props();

  const verdict = $derived(report?.verdict ?? null);
</script>

{#if busy}
  <section class="rec-verdict state-edge" aria-label="How recording stands" aria-busy="true">
    <Skeleton width="22em" />
  </section>
{:else if verdict}
  <section class="rec-verdict state-edge {verdict.tone}" aria-label="How recording stands">
    <div class="rec-state">
      <Glyph mark={Marks.ofTone(verdict.tone)} />
      <strong>{verdict.headline}</strong>
    </div>
    {#each report?.indicators ?? [] as one (one.key)}
      <div class="rec-indicator {one.tone}">
        <span class="cap">{one.label}</span>
        <span class="value mono"><Glyph mark={Marks.ofTone(one.tone)} />{one.value}</span>
        {#if one.share !== null}<Meter share={one.share} tone={one.tone} least={0.02} />{/if}
      </div>
    {/each}
    {#if usage}
      <div class="rec-indicator neutral">
        <span class="cap">Open</span>
        <span class="value mono">{num(usage.open_calls)}</span>
      </div>
    {/if}
  </section>
{/if}

<style>
  .rec-verdict {
    display: grid;
    grid-template-columns: minmax(220px, 1.6fr) repeat(auto-fit, minmax(92px, 1fr));
    gap: 1px;
    border-bottom: 1px solid var(--control-border);
    background: var(--line);
    font-size: 13.5px;
  }
  .rec-verdict > div {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 5px;
    min-width: 0;
    padding: 10px 16px;
    background: var(--surface);
  }
  .rec-verdict > .rec-state {
    flex-direction: row;
    align-items: center;
    justify-content: flex-start;
    gap: 10px;
    padding-left: 18px;
  }
  .rec-indicator .value {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 15px;
  }
  .rec-indicator.success {
    --tone: var(--success);
  }
  .rec-indicator.warning {
    --tone: var(--warning);
    background: color-mix(in srgb, var(--warning) 8%, var(--surface));
  }
  .rec-indicator.error {
    --tone: var(--error);
    background: color-mix(in srgb, var(--error) 10%, var(--surface));
  }
  .rec-indicator.warning .value,
  .rec-indicator.error .value {
    color: var(--tone);
  }
  .rec-verdict.error {
    --state: var(--error);
  }
  .rec-verdict.warning {
    --state: var(--warning);
  }
  .rec-verdict.success {
    --state: var(--success);
  }
  .rec-verdict strong {
    font-weight: 600;
  }
</style>
