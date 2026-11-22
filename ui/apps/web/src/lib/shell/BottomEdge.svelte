<!--
  The bottom edge of every page: silent while everything is well; one strip
  when something is not, the worst first, the rest behind "+N". Which strips
  to say is `BottomEdge.strips` (edge.ts); this draws them.
-->
<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import { Clock } from '#lib/time.ts';
  import { BottomEdge, type EdgeReading } from '#lib/edge.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';

  let {
    reading,
    onretry,
    ondismiss,
  }: {
    /** What the layout read, as the edge needs it. */
    reading: EdgeReading;
    /** Read Ren again, now. */
    onretry: () => void;
    /** The operator dismissed a recorder notice, by its words. */
    ondismiss: (said: string) => void;
  } = $props();

  const workbench = getWorkbench();
  const strips = $derived(BottomEdge.strips(reading));
  /** Whether every strip shows, not just the worst. */
  let open = $state(false);
</script>

<div class="edge">
  {#each open ? strips : strips.slice(0, 1) as strip, place (strip.kind)}
    {@const tone = BottomEdge.tone(strip)}
    <div class="strip {tone}" role={tone === 'bad' ? 'alert' : 'status'}>
      {#if strip.kind === 'unreachable'}<Glyph mark="failed" /><span
          ><b>Ren isn't answering.</b>
          {strip.readAt
            ? `Showing what was read at ${Clock.second(strip.readAt)}.`
            : 'Nothing has been read yet.'}</span
        ><span class="strip-acts"
          >{#if workbench.loading}<span class="muted mono">retrying…</span>{/if}<button
            class="button small"
            disabled={workbench.loading}
            onclick={onretry}>Retry now</button
          ></span
        >
      {:else if strip.kind === 'incompatible'}<Glyph mark="failed" /><span
          ><b>This UI and Ren speak different API versions.</b> One was updated without the other.</span
        ><span class="strip-acts"
          ><button class="button small" onclick={() => location.reload()}>Reload</button></span
        >
      {:else if strip.kind === 'recorder'}<Glyph mark="warning" /><span
          ><b>Recording:</b> {strip.said}</span
        ><span class="strip-acts"
          ><button class="button small" onclick={() => ondismiss(strip.said)}>Dismiss</button><a
            class="button small"
            href="/system/recorder">Recorder →</a
          ></span
        >
      {:else}<Glyph mark="idle" /><span><b>Read only</b></span>
      {/if}
      {#if place === 0 && strips.length > 1}<button
          class="strip-more mono"
          aria-expanded={open}
          onclick={() => (open = !open)}>{open ? 'fewer' : `+${strips.length - 1}`}</button
        >{/if}
    </div>
  {/each}
</div>

<style>
  .edge {
    flex-shrink: 0;
  }
  .strip {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 34px;
    padding: 4px 16px;
    border-top: 1px solid var(--line);
    background: var(--surface);
    color: var(--ink);
    font-size: 12.5px;
  }
  .strip b {
    font-weight: 600;
  }
  .strip.bad {
    border-top-color: color-mix(in srgb, var(--error) 55%, var(--line));
    background: color-mix(in srgb, var(--error) 12%, var(--canvas));
  }
  .strip.warning {
    border-top-color: color-mix(in srgb, var(--warning) 55%, var(--line));
    background: color-mix(in srgb, var(--warning) 12%, var(--canvas));
  }
  .strip-acts {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;
    font-size: 11.5px;
  }
  .strip-more {
    border: 0;
    background: none;
    color: var(--muted);
    font-size: 11px;
    cursor: pointer;
  }
  .strip-more:hover {
    color: var(--ink);
  }
</style>
