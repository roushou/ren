<!--
  A watched repository's open pull requests as one bar, split by where they
  stand (`GitHubWatches.groups`), in their order.
-->
<script lang="ts">
  import { GitHubWatches, type WatchRow } from './github.ts';

  let { row }: { row: Pick<WatchRow, 'pulls' | 'groups'> } = $props();
  const parts = $derived.by(() => {
    let before = 0;
    return GitHubWatches.groups.map((group) => {
      const part = { group, x: before, width: row.groups[group] };
      before += row.groups[group];
      return part;
    });
  });
  const total = $derived(Math.max(1, row.pulls.length));
</script>

<svg class="pull-bar" viewBox="0 0 100 6" preserveAspectRatio="none" aria-hidden="true">
  <rect class="track" width="100" height="6" />
  {#each parts as part (part.group)}
    {#if part.width}<rect
        class="part {part.group}"
        x={(part.x / total) * 100}
        width={(part.width / total) * 100}
        height="6"
      />{/if}
  {/each}
</svg>

<style>
  .pull-bar {
    display: block;
    width: 100%;
    height: 8px;
  }
  .track {
    fill: var(--surface-inset);
  }
  .ready {
    fill: var(--success);
  }
  .review {
    fill: var(--warning);
  }
  .failing {
    fill: var(--error);
  }
  .running {
    fill: var(--live);
  }
  .held {
    fill: color-mix(in srgb, var(--warning) 50%, transparent);
  }
  .draft,
  .unknown {
    fill: var(--muted);
  }
</style>
