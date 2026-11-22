<!--
  Layer 2: a look without leaving, anchored under the control that opens it
  (a button with `popovertarget={id}`). The browser's popover keeps it in
  the top layer and closes it on Escape or a click elsewhere. It lines an
  edge up with its trigger's (`align`), and is placed by script as it opens:
  the CSP refuses an inline style in the markup, not one set from script.
  It is placed on `beforetoggle`, which runs before the browser shows it;
  `toggle` runs after, so a first open would paint one frame unplaced.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    id,
    label,
    width = 'narrow',
    align = 'right',
    children,
  }: {
    id: string;
    label: string;
    /** 360px, or 460px for rows with actions. */
    width?: 'narrow' | 'wide';
    /** Which edge it shares with its trigger: the right for a trigger on
     * the bar's right, the left for one on its left. */
    align?: 'left' | 'right';
    children: Snippet;
  } = $props();

  let element = $state<HTMLElement>();

  function place(event: ToggleEvent) {
    if (event.newState !== 'open' || !element) return;
    const trigger = document.querySelector(`[popovertarget="${id}"]`);
    if (!trigger) return;
    const anchor = trigger.getBoundingClientRect();
    element.style.top = `${Math.round(anchor.bottom + 6)}px`;
    if (align === 'left') element.style.left = `${Math.max(8, Math.round(anchor.left))}px`;
    else element.style.right = `${Math.max(8, Math.round(window.innerWidth - anchor.right))}px`;
  }
</script>

<div
  bind:this={element}
  {id}
  class="popover {width}"
  popover="auto"
  role="dialog"
  aria-label={label}
  onbeforetoggle={place}
>
  {@render children()}
</div>

<style>
  .popover {
    position: fixed;
    inset: auto;
    width: min(360px, calc(100vw - 16px));
    max-height: calc(100vh - 96px);
    margin: 0;
    padding: 0;
    overflow: auto;
    border: 1px solid var(--popover-border);
    background: var(--popover);
    color: var(--ink);
    box-shadow: var(--elev-popover);
    font-size: 12.5px;
  }
  .wide {
    width: min(460px, calc(100vw - 16px));
  }
</style>
