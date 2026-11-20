<!--
  Attached images at full size, over the page, one at a time: ← and → (or
  the arrows in its bar) step through the images it was opened on. Closes on
  Escape, on the close button, or on a click outside the image; the original
  opens in a tab.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import Icon from '#lib/Icon.svelte';
  import { Gallery, type Viewable } from '#lib/attachments.ts';

  let {
    images,
    start,
    onclose,
  }: {
    /** The images to step through, in order; at least one. */
    images: readonly Viewable[];
    /** The one shown first. */
    start: number;
    onclose: () => void;
  } = $props();

  let dialog = $state<HTMLDialogElement>();
  let index = $state(untrack(() => Math.min(Math.max(start, 0), images.length - 1)));
  const shown = $derived(images[index]);

  $effect(() => {
    untrack(() => dialog?.showModal());
  });

  function close() {
    dialog?.close();
    onclose();
  }

  function step(by: -1 | 1) {
    index = Math.min(Math.max(index + by, 0), images.length - 1);
  }
</script>

<dialog
  bind:this={dialog}
  class="viewer"
  aria-label={shown?.read.name}
  oncancel={(event) => {
    event.preventDefault();
    close();
  }}
  onclick={(event) => {
    if (event.target === dialog) close();
  }}
  onkeydown={(event) => {
    if (event.key === 'ArrowLeft') step(-1);
    else if (event.key === 'ArrowRight') step(1);
  }}
>
  {#if shown}
    <header>
      {#if images.length > 1}
        <div class="steps">
          <button
            class="icon-button"
            aria-label="Previous image"
            disabled={index === 0}
            onclick={() => step(-1)}
            ><span class="previous"><Icon name="chevron" size={12} /></span></button
          >
          <span class="count">{index + 1} / {images.length}</span>
          <button
            class="icon-button"
            aria-label="Next image"
            disabled={index === images.length - 1}
            onclick={() => step(1)}><Icon name="chevron" size={12} /></button
          >
        </div>
      {/if}
      <span class="name" title={shown.read.name}>{shown.read.name}</span>
      <span class="muted">{Gallery.describe(shown.read)}</span>
      <a class="button small" href={shown.src} target="_blank" rel="noopener">Open original</a>
      <button class="icon-button" aria-label="Close" onclick={close}
        ><Icon name="close" size={12} /></button
      >
    </header>
    <img src={shown.src} alt={shown.read.name} />
  {/if}
</dialog>

<style>
  .viewer {
    max-width: calc(100vw - 32px);
    max-height: calc(100vh - 32px);
    padding: 0;
    border: 1px solid var(--popover-border);
    border-radius: 0;
    background: var(--popover);
    color: var(--ink);
    box-shadow: var(--popover-shadow);
    font-size: 13px;
  }
  .viewer[open] {
    display: flex;
    flex-direction: column;
  }
  .viewer::backdrop {
    background: rgb(0 0 0 / 0.6);
  }
  header {
    display: flex;
    gap: 12px;
    align-items: center;
    padding: 6px 6px 6px 12px;
    border-bottom: 1px solid var(--line);
  }
  .steps {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-left: -6px;
  }
  .previous {
    display: inline-flex;
    transform: rotate(180deg);
  }
  .count {
    min-width: 3.5em;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  img {
    display: block;
    min-height: 0;
    max-width: calc(100vw - 34px);
    max-height: calc(100vh - 80px);
    margin: auto;
    object-fit: contain;
  }
</style>
