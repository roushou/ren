<!--
  Layer 5: the toast stack, bottom right over everything but a modal's
  scrim. The layout draws it once.
-->
<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import { toasts } from './toasts.svelte.ts';
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each toasts.items as toast (toast.id)}
    <div class="toast">
      <Glyph mark={toast.mark} />
      <span class="text">
        <span>{toast.text}</span>
        {#if toast.detail}<span class="detail">{toast.detail}</span>{/if}
      </span>
      <button class="icon-button" aria-label="Dismiss" onclick={() => toasts.dismiss(toast.id)}
        ><Icon name="close" size={12} /></button
      >
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 16px;
    bottom: 48px;
    z-index: 40;
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: min(380px, calc(100vw - 32px));
    pointer-events: none;
  }
  .toast {
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 10px;
    padding: 10px 8px 10px 12px;
    border: 1px solid var(--popover-border);
    background: var(--popover);
    color: var(--ink);
    box-shadow: var(--elev-toast);
    font-size: 12.5px;
    pointer-events: auto;
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    line-height: 1.35;
  }
  .detail {
    overflow: hidden;
    color: var(--muted);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
