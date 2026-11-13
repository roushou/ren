<!--
  A "more" button that opens a short menu of actions, least risky first,
  with a block of details between them. Esc or a click outside closes it;
  ↑ and ↓ move between actions.

  <ActionMenu label="Run actions" items={[{ label: 'Archive', onselect }]} details={['Codex']} />
-->
<script lang="ts" module>
  export type ActionItem = {
    label: string;
    onselect: () => void;
    /** A destructive action, set apart and in the danger color. */
    danger?: boolean;
  };
</script>

<script lang="ts">
  import { tick } from 'svelte';
  import Icon from './Icon.svelte';

  let {
    label,
    items,
    details = [],
    disabled = false,
  }: {
    /** The accessible name of the button and menu. */
    label: string;
    items: readonly ActionItem[];
    /** Facts shown in the menu, quietly, between the safe and destructive actions. */
    details?: readonly string[];
    disabled?: boolean;
  } = $props();

  const id = $props.id();
  let isOpen = $state(false);
  let button = $state<HTMLButtonElement>();
  let panel = $state<HTMLElement>();
  const safe = $derived(items.filter((item) => !item.danger));
  const risky = $derived(items.filter((item) => item.danger));

  async function open() {
    isOpen = true;
    await tick();
    panel?.querySelector<HTMLButtonElement>('button')?.focus();
  }

  function close(focusButton = true) {
    isOpen = false;
    if (focusButton) button?.focus();
  }

  function choose(item: ActionItem) {
    close();
    item.onselect();
  }

  function key(event: KeyboardEvent) {
    const buttons = [...(panel?.querySelectorAll<HTMLButtonElement>('button') ?? [])];
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    switch (event.key) {
      case 'Escape':
        event.preventDefault();
        event.stopPropagation();
        close();
        break;
      case 'ArrowDown':
        event.preventDefault();
        buttons[Math.min(at + 1, buttons.length - 1)]?.focus();
        break;
      case 'ArrowUp':
        event.preventDefault();
        buttons[Math.max(at - 1, 0)]?.focus();
        break;
    }
  }

  function outside(event: PointerEvent) {
    if (!isOpen) return;
    const target = event.target as Node;
    if (!panel?.contains(target) && !button?.contains(target)) close(false);
  }
</script>

<svelte:window onpointerdown={outside} />

<span class="action-menu">
  <button
    bind:this={button}
    type="button"
    class="button small more"
    aria-label={label}
    aria-haspopup="menu"
    aria-expanded={isOpen}
    aria-controls="{id}-menu"
    {disabled}
    onclick={() => (isOpen ? close() : void open())}><Icon name="dots" size={14} /></button
  >
  {#if isOpen}
    <div
      bind:this={panel}
      id="{id}-menu"
      class="panel"
      role="menu"
      aria-label={label}
      tabindex="-1"
      onkeydown={key}
    >
      {#each safe as item (item.label)}
        <button type="button" role="menuitem" onclick={() => choose(item)}>{item.label}</button>
      {/each}
      {#if details.length}
        <div class="details">
          {#each details as detail (detail)}<p>{detail}</p>{/each}
        </div>
      {/if}
      {#each risky as item (item.label)}
        <button type="button" role="menuitem" class="danger" onclick={() => choose(item)}
          >{item.label}</button
        >
      {/each}
    </div>
  {/if}
</span>

<style>
  .action-menu {
    position: relative;
    display: inline-flex;
  }
  .more {
    width: 28px;
    padding: 0;
    justify-content: center;
  }
  .panel {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 20;
    min-width: 240px;
    max-width: 380px;
    display: flex;
    flex-direction: column;
    padding: 4px 0;
    border: 1px solid var(--popover-border);
    border-top: 2px solid var(--accent);
    background: var(--popover);
    box-shadow: var(--popover-shadow);
    outline: none;
  }
  .panel button {
    padding: 8px 14px;
    border: 0;
    background: none;
    color: var(--ink);
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    text-align: left;
    cursor: pointer;
  }
  .panel button:hover,
  .panel button:focus-visible {
    background: var(--popover-hover);
    outline: none;
  }
  .panel button:focus-visible {
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .panel button.danger {
    color: var(--error);
  }
  .panel button.danger:hover,
  .panel button.danger:focus-visible {
    background: var(--error-soft);
  }
  .details {
    margin: 4px 0;
    padding: 8px 14px;
    border-top: 1px solid var(--popover-border);
    border-bottom: 1px solid var(--popover-border);
    background: var(--surface-inset);
    color: var(--muted);
    font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', monospace;
    font-size: 11px;
    line-height: 1.5;
  }
  .details p {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .details:first-child {
    margin-top: -4px;
    border-top: 0;
  }
  .details:last-child {
    margin-bottom: -4px;
    border-bottom: 0;
  }
</style>
