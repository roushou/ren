<!--
  One attached image at full size, over the page. Closes on Escape, on the
  close button, or on a click outside the image; the original opens in a tab.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import Icon from '#lib/Icon.svelte';

  let {
    src,
    name,
    detail,
    onclose,
  }: { src: string; name: string; detail: string; onclose: () => void } = $props();

  let dialog = $state<HTMLDialogElement>();

  $effect(() => {
    untrack(() => dialog?.showModal());
  });

  function close() {
    dialog?.close();
    onclose();
  }
</script>

<dialog
  bind:this={dialog}
  class="viewer"
  aria-label={name}
  oncancel={(event) => {
    event.preventDefault();
    close();
  }}
  onclick={(event) => {
    if (event.target === dialog) close();
  }}
>
  <header>
    <span class="name" title={name}>{name}</span>
    <span class="muted">{detail}</span>
    <a class="button small" href={src} target="_blank" rel="noopener">Open original</a>
    <button class="icon-button" aria-label="Close" onclick={close}
      ><Icon name="close" size={12} /></button
    >
  </header>
  <img {src} alt={name} />
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
