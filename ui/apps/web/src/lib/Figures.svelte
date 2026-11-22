<!--
  A row of figures: each a caption over a value (`Figure`), side by side on
  hairlines, wrapping as the width allows. `size` sets the values' size: a
  page's strip is large, a panel's small. Actions, when given, sit in a last
  cell of their own. Cells take the host's `--figure-surface`, the surface
  by default.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    label,
    size = 'large',
    children,
    actions,
  }: {
    /** What the row is, for assistive technology. */
    label: string;
    size?: 'large' | 'small';
    children: Snippet;
    actions?: Snippet;
  } = $props();
</script>

<div class="figures {size}" role="group" aria-label={label}>
  {@render children()}
  {#if actions}<div class="figures-actions">{@render actions()}</div>{/if}
</div>

<style>
  .figures {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(88px, 1fr));
    gap: 1px;
    background: var(--line);
  }
  .figures-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    justify-content: flex-end;
    padding: 10px 16px;
    background: var(--figure-surface, var(--surface));
  }
  .small :global(.figure) {
    padding: 8px 12px;
  }
  .small :global(.figure-value) {
    font-size: 15px;
  }
</style>
