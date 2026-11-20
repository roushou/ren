<!--
  Layer 3: the details of what a page has selected, over the page's right
  side. The page underneath keeps its full width and its place; closing
  (the close button or Escape, unless a modal above has it) hands the page
  back as it was. The page that holds it is `position: relative`.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '#lib/Icon.svelte';

  let {
    label,
    onclose,
    size = 'narrow',
    children,
  }: {
    /** What it shows, for assistive technology and its close button. */
    label: string;
    onclose: () => void;
    /** 440px, or 560px for wide content such as bodies. (The CSP allows no
     * inline style, so the widths are classes.) */
    size?: 'narrow' | 'wide';
    children: Snippet;
  } = $props();

  function key(event: KeyboardEvent) {
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    // A modal above takes Escape for itself.
    if (document.querySelector('dialog[open]')) return;
    onclose();
  }
</script>

<svelte:window onkeydown={key} />

<aside class="slide-over {size}" aria-label={label}>
  <button class="icon-button close" onclick={onclose} aria-label="Close {label}" title="Close · Esc"
    ><Icon name="close" size={14} /></button
  >
  {@render children()}
</aside>

<style>
  .slide-over {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 20;
    display: flex;
    flex-direction: column;
    width: min(440px, 100%);
    min-height: 0;
    overflow: auto;
    border-left: 1px solid var(--popover-border);
    background: var(--surface);
    box-shadow: var(--elev-slide);
  }
  .wide {
    width: min(560px, 100%);
  }
  .close {
    position: absolute;
    top: 8px;
    right: 8px;
    z-index: 1;
  }
</style>
