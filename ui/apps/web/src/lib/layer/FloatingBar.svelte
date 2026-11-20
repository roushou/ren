<!--
  Layer 1: what to do with what a page has gathered (a selection, staged
  changes), floating over the bottom of the content it acts on. The content
  that holds it is `position: relative`.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  let { label, children }: { label: string; children: Snippet } = $props();
</script>

<div class="floating-bar" role="toolbar" aria-label={label}>{@render children()}</div>

<style>
  .floating-bar {
    position: absolute;
    bottom: 16px;
    left: 50%;
    z-index: 15;
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: calc(100% - 32px);
    padding: 6px 6px 6px 12px;
    transform: translateX(-50%);
    border: 1px solid var(--popover-border);
    background: var(--popover);
    box-shadow: var(--elev-toolbar);
    font-size: 12px;
    white-space: nowrap;
  }
</style>
