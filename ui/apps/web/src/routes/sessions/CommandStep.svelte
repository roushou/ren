<script lang="ts">
  import Icon from '#lib/Icon.svelte';
  import type { Entry } from '#lib/runs.ts';

  // A command a turn ran, folded to its line and outcome. Its output can run
  // to hundreds of kilobytes, so it is drawn only while the fold is open:
  // a long conversation renders and hydrates without every command's log.
  let { entry }: { entry: Extract<Entry, { kind: 'command' }> } = $props();
  let open = $state(false);

  const tone = $derived(
    entry.status === 'completed' && (entry.exit === 0 || entry.exit === null)
      ? 'success'
      : entry.status === 'in_progress'
        ? 'live'
        : entry.status === 'declined'
          ? 'neutral'
          : 'error',
  );
  const outcome = $derived(
    entry.status === 'in_progress'
      ? 'running'
      : entry.exit !== null
        ? `exit ${entry.exit}`
        : entry.status,
  );
</script>

<details class="entry tool command" bind:open>
  <summary>
    <Icon name="terminal" size={12} />
    <span class="tool-target mono">{entry.command}</span>
    <span class="chip {tone}">{outcome}</span>
  </summary>
  {#if open && entry.output}<pre class="activity-pre">{entry.output}</pre>{/if}
</details>

<style>
  .entry.tool.command {
    padding: 6px 16px;
    border-bottom: 1px solid var(--line-subtle);
    font-size: 12px;
    color: var(--ink);
  }
  summary {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  /* Its outcome sits at the row's end, however long the command. */
  summary > .chip {
    flex: none;
    margin-left: auto;
  }
  .tool-target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
