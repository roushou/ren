<!--
  What the recorder lost or failed since startup, worst first, and the loss
  and error counters still at zero.
-->
<script lang="ts">
  import type { RecorderReport } from '#lib/recorder.ts';
  import Glyph from '#lib/Glyph.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import RecorderBlock from './RecorderBlock.svelte';
  import RecorderRows from './RecorderRows.svelte';

  let { busy, report }: { busy: boolean; report: RecorderReport | null } = $props();
</script>

<RecorderBlock id="loss-title" title="Loss and errors" {busy} size="grow">
  {#snippet aside()}
    {#if report && !report.notes.some((note) => note.severity !== 'neutral')}<Glyph
        mark="passed"
        label="Nothing lost or failed"
      />{/if}
  {/snippet}
  {#if report}
    {#each report.notes as note (note.label)}
      <div class="rec-note {note.severity}">
        <div>
          <strong>{note.label}</strong>
          <small>{note.detail}</small>
        </div>
        <span class="mono">{note.value}</span>
      </div>
    {/each}
    <RecorderRows rows={report.quiet.map((label) => ({ label, value: '0' }))} />
  {:else}<Skeleton width="100%" />{/if}
</RecorderBlock>

<style>
  .rec-note {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 8px;
    padding: 8px 10px;
    border: 1px solid var(--line);
  }
  .rec-note > div {
    display: flex;
    flex-direction: column;
  }
  .rec-note strong {
    font-weight: 500;
  }
  .rec-note.error strong,
  .rec-note small {
    color: var(--muted);
    font-size: 11px;
  }
</style>
