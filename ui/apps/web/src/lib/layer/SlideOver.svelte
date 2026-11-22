<!--
  Layer 3: the details of what a page has selected, over the page's right
  side, or pinned beside it as a column. Floating, the page underneath
  keeps its full width and its place; closing
  (the close button or Escape, unless a modal above has it) hands the page
  back as it was. The page that holds it is `position: relative`.
-->
<script lang="ts">
  import type { SlideKind } from './pins.svelte.ts';
  import type { Snippet } from 'svelte';
  import Icon from '#lib/Icon.svelte';
  import { pins } from './pins.svelte.ts';

  let {
    label,
    onclose,
    size = 'narrow',
    pin = null,
    children,
  }: {
    /** What it shows, for assistive technology and its close button. */
    label: string;
    onclose: () => void;
    /** 440px, or 560px for wide content such as bodies. (The CSP allows no
     * inline style, so the widths are classes.) */
    size?: 'narrow' | 'wide';
    /** Its kind, when it can be pinned: pinned, it stands beside the page
     * as a column of its own, and the page makes room for it. */
    pin?: SlideKind | null;
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

<aside class="slide-over {size}" class:pinned={pin !== null && pins.has(pin)} aria-label={label}>
  <span class="corner">
    {#if pin !== null}{@const kind = pin}<button
        class="icon-button"
        aria-pressed={pins.has(kind)}
        aria-label={pins.has(kind) ? `Unpin ${label}` : `Pin ${label} beside the page`}
        title={pins.has(kind) ? 'Unpin: float over the page' : 'Pin beside the page'}
        onclick={() => pins.toggle(kind)}><Icon name="pin" size={13} /></button
      >{/if}
    <button class="icon-button" onclick={onclose} aria-label="Close {label}" title="Close · Esc"
      ><Icon name="close" size={14} /></button
    >
  </span>
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
  .corner {
    position: absolute;
    top: 8px;
    right: 8px;
    z-index: 1;
    display: flex;
    gap: 2px;
  }
  .corner [aria-pressed='true'] {
    color: var(--accent);
  }
  /* Pinned, it is a column of the page, not a layer over it: no shadow, and
     the page that holds it makes room (`:has` below). */
  .slide-over.pinned {
    box-shadow: none;
    border-left-color: var(--line);
  }
  :global(:has(> .slide-over.pinned.narrow)) {
    padding-right: min(440px, 100%);
  }
  :global(:has(> .slide-over.pinned.wide)) {
    padding-right: min(560px, 100%);
  }
</style>
