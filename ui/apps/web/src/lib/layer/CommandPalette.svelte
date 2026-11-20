<!--
  Layer 4: go anywhere by typing (⌘K or Ctrl+K, or the top bar's box). It
  lists every page and each project's own, narrowed as the operator types,
  with a search of recorded history last. ↑ ↓ move, Enter goes, Escape
  closes.
-->
<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import Icon from '#lib/Icon.svelte';
  import { Commands, type Command } from '#lib/commands.ts';

  let {
    projects,
    onclose,
  }: {
    projects: readonly { name: string }[];
    onclose: () => void;
  } = $props();

  let dialog = $state<HTMLDialogElement>();
  let input = $state<HTMLInputElement>();
  let query = $state('');
  let chosen = $state(0);
  const all = $derived(Commands.all(projects));
  const shown = $derived(Commands.match(all, query));

  $effect(() => {
    untrack(() => {
      dialog?.showModal();
      void tick().then(() => input?.focus());
    });
  });
  $effect(() => {
    // A new query starts again at the top.
    void query;
    chosen = 0;
  });

  function go(command: Command | undefined) {
    if (!command) return;
    dialog?.close();
    onclose();
    void goto(command.href);
  }

  function key(event: KeyboardEvent) {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const step = event.key === 'ArrowDown' ? 1 : -1;
      chosen = (chosen + step + shown.length) % Math.max(shown.length, 1);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      go(shown[chosen]);
    }
  }
</script>

<dialog
  bind:this={dialog}
  class="palette"
  aria-label="Go to anything"
  oncancel={(event) => {
    event.preventDefault();
    onclose();
  }}
  onclick={(event) => {
    if (event.target === dialog) onclose();
  }}
>
  <label class="field">
    <Icon name="search" size={14} />
    <input
      bind:this={input}
      bind:value={query}
      onkeydown={key}
      placeholder="Go to a page, a project, or search recorded history"
      aria-label="Go to anything"
      aria-controls="palette-list"
      aria-activedescendant={shown.length ? `palette-${chosen}` : undefined}
      autocomplete="off"
      spellcheck="false"
    />
    <kbd>esc</kbd>
  </label>
  <ul id="palette-list" role="listbox" aria-label="Places">
    {#each shown as command, index (command.href)}
      <li
        id="palette-{index}"
        role="option"
        aria-selected={index === chosen}
        class:chosen={index === chosen}
        onmousemove={() => (chosen = index)}
        onclick={() => go(command)}
        onkeydown={() => {}}
      >
        <Icon name={command.kind === 'search' ? 'search' : 'chevron'} size={12} />
        <span class="label">{command.label}</span>
        <span class="detail">{command.detail}</span>
      </li>
    {/each}
  </ul>
  <footer>
    <span><kbd>↑</kbd> <kbd>↓</kbd> move</span><span><kbd>↵</kbd> go</span>
  </footer>
</dialog>

<style>
  .palette {
    width: min(640px, calc(100vw - 32px));
    margin-top: 12vh;
    padding: 0;
    border: 1px solid var(--popover-border);
    border-radius: 0;
    background: var(--popover);
    color: var(--ink);
    box-shadow: var(--elev-modal);
    font-size: 13px;
  }
  .palette::backdrop {
    background: var(--scrim);
  }
  .field {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 46px;
    padding: 0 14px;
    border-bottom: 1px solid var(--line);
    color: var(--muted);
  }
  input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: none;
    color: var(--ink);
    font: inherit;
    font-size: 14px;
  }
  ul {
    max-height: 50vh;
    margin: 0;
    padding: 4px 0;
    overflow: auto;
    list-style: none;
  }
  li {
    display: grid;
    grid-template-columns: 14px minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 10px;
    height: 32px;
    padding: 0 14px;
    color: var(--muted);
    cursor: pointer;
  }
  li.chosen {
    background: var(--surface-hover);
    color: var(--ink);
  }
  .label {
    overflow: hidden;
    color: var(--ink);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .detail {
    font-size: 11px;
  }
  footer {
    display: flex;
    gap: 16px;
    padding: 8px 14px;
    border-top: 1px solid var(--line);
    color: var(--muted);
    font-size: 11px;
  }
  kbd {
    padding: 0 4px;
    border: 1px solid var(--control-border);
    border-bottom-width: 2px;
    border-radius: 2px;
    font-family: var(--mono, monospace);
    font-size: 10px;
  }
</style>
