<!--
  How long the recorder's commits take: the last and the slowest on one
  scale, with its lag and the calls it recovered at startup.
-->
<script lang="ts">
  import { num, type RecorderMetrics } from '#lib/api.ts';
  import Skeleton from '#lib/Skeleton.svelte';
  import RecorderBlock from './RecorderBlock.svelte';
  import RecorderRows from './RecorderRows.svelte';

  let { busy, recorder }: { busy: boolean; recorder: RecorderMetrics | undefined } = $props();

  /** The scale's end: the slowest commit, and never under 250 ms. */
  const scale = $derived(Math.max(250, recorder?.max_commit_ms ?? 0));
</script>

<RecorderBlock
  id="commits-title"
  title="Commits{recorder ? ` · ${num(recorder.commits)}` : ''} · commit time"
  {busy}
>
  {#if recorder}
    <svg class="commit-scale" viewBox="0 0 100 10" preserveAspectRatio="none" aria-hidden="true"
      ><rect class="track" y="4" width="100" height="2" /><rect
        class="tick last"
        x={(recorder.last_commit_ms / scale) * 100 - 0.4}
        width="0.8"
        height="10"
      /><rect
        class="tick slowest"
        x={Math.min(99.2, (recorder.max_commit_ms / scale) * 100 - 0.4)}
        width="0.8"
        height="10"
      /></svg
    >
    <div class="scale-axis meta">
      <span>0</span><span>{Math.round(scale / 2)} ms</span><span>{scale} ms</span>
    </div>
    <RecorderRows
      rows={[
        { label: 'Last commit', value: `${num(recorder.last_commit_ms)} ms` },
        { label: 'Slowest commit', value: `${num(recorder.max_commit_ms)} ms` },
        { label: 'Recorder lag', value: `${num(recorder.lag_ms)} ms` },
        { label: 'Calls recovered at startup', value: num(recorder.recovered_calls) },
      ]}
    />
  {:else}<Skeleton width="100%" />{/if}
</RecorderBlock>

<style>
  .commit-scale {
    width: 100%;
    height: 14px;
  }
  .commit-scale .track {
    fill: var(--line);
  }
  .commit-scale .tick.last {
    fill: var(--ink);
  }
  .commit-scale .tick.slowest {
    fill: var(--warning);
  }
  .scale-axis {
    display: flex;
    justify-content: space-between;
    margin-bottom: 8px;
  }
</style>
