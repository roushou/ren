<!--
  Layer 2: a look at what a name stands for, without leaving the page. It
  opens under its trigger after a short rest of the pointer, or on keyboard
  focus, and closes when the pointer or focus leaves both; Escape closes
  it. It lives in the browser's top layer, so a clipping row never cuts it
  off, and is placed by script when it opens: the CSP refuses an inline
  style in the markup, not one set from script.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    label,
    trigger,
    children,
  }: {
    /** What the card is about, for assistive technology. */
    label: string;
    trigger: Snippet;
    children: Snippet;
  } = $props();

  /** How long the pointer rests before the card opens, and how long it
   * may cross the gap to the card before it closes, in ms. */
  const REST = 350;
  const GRACE = 150;
  const id = $props.id();
  let anchor = $state<HTMLElement>();
  let card = $state<HTMLElement>();
  let open = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  // A card that leaves the page takes its pending show or hide with it.
  $effect(() => () => clearTimeout(timer));

  function show() {
    if (!anchor || !card || open) return;
    const rect = anchor.getBoundingClientRect();
    card.style.top = `${Math.round(rect.bottom + 6)}px`;
    card.style.left = `${Math.round(Math.min(rect.left, window.innerWidth - 316))}px`;
    card.showPopover();
    open = true;
  }
  function hide() {
    if (!card || !open) return;
    card.hidePopover();
    open = false;
  }
  function rest() {
    clearTimeout(timer);
    timer = setTimeout(show, REST);
  }
  function leave() {
    clearTimeout(timer);
    timer = setTimeout(hide, GRACE);
  }
  function key(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      clearTimeout(timer);
      hide();
    }
  }
</script>

<span
  class="trigger"
  bind:this={anchor}
  role="presentation"
  aria-describedby={open ? id : undefined}
  onpointerenter={rest}
  onpointerleave={leave}
  onfocusin={() => {
    clearTimeout(timer);
    show();
  }}
  onfocusout={leave}
  onkeydown={key}>{@render trigger()}</span
>
<div
  bind:this={card}
  {id}
  class="card"
  popover="manual"
  role="tooltip"
  aria-label={label}
  onpointerenter={() => clearTimeout(timer)}
  onpointerleave={leave}
>
  {@render children()}
</div>

<style>
  /* A name can be long: the trigger shrinks with its line, and clips. */
  .trigger {
    display: inline-flex;
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
  }
  .card {
    position: fixed;
    inset: auto;
    flex-direction: column;
    gap: 6px;
    width: 300px;
    margin: 0;
    padding: 10px 12px;
    border: 1px solid var(--popover-border);
    background: var(--popover);
    color: var(--ink);
    box-shadow: var(--elev-popover);
    font-size: 12px;
    font-weight: 400;
    text-align: left;
    text-transform: none;
    letter-spacing: normal;
  }
  .card:popover-open {
    display: flex;
  }
</style>
