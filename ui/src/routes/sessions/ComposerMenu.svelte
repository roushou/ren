<script lang="ts" module>
  /** One offer: its name (with the typed characters marked), what it is, and what it does. */
  export interface MenuEntry {
    key: string;
    parts: { text: string; hit: boolean }[];
    /** What follows the name, quieter: a command's arguments. */
    hint: string | null;
    /** What it is: skill, template, folder. */
    tag: string | null;
    detail: string;
  }
</script>

<script lang="ts">
  let {
    id,
    heading,
    entries,
    active,
    note,
    onpick,
    onpoint,
  }: {
    id: string;
    heading: string;
    entries: MenuEntry[];
    active: number;
    /** Said instead of entries: loading, nothing found, or why it failed. */
    note: string | null;
    onpick: (index: number) => void;
    onpoint: (index: number) => void;
  } = $props();

  let list = $state<HTMLUListElement>();
  $effect(() => {
    // Keep the highlighted entry in view as the arrows move it.
    list?.children[active]?.scrollIntoView({ block: 'nearest' });
  });
</script>

<div class="composer-menu">
  <div class="menu-head">
    <span>{heading}</span>
    <span class="keys">↑↓ choose · Enter or Tab insert · Esc close</span>
  </div>
  {#if entries.length > 0}
    <ul {id} role="listbox" aria-label={heading} bind:this={list}>
      {#each entries as entry, index (entry.key)}
        <li
          id="{id}-{index}"
          role="option"
          aria-selected={index === active}
          class:active={index === active}
          onpointermove={() => index !== active && onpoint(index)}
          onpointerdown={(event) => {
            // Keep the focus in the prompt.
            event.preventDefault();
            onpick(index);
          }}
        >
          <span class="name"
            >{#each entry.parts as part, at (at)}{#if part.hit}<mark>{part.text}</mark
                >{:else}{part.text}{/if}{/each}{#if entry.hint}<span class="hint">
                {entry.hint}</span
              >{/if}</span
          >
          {#if entry.tag}<span class="tag">{entry.tag}</span>{/if}
          <span class="detail">{entry.detail}</span>
        </li>
      {/each}
    </ul>
  {:else if note}
    <p class="menu-note">{note}</p>
  {/if}
</div>

<style>
  /* Flush above the prompt, ruled off like the rows around it. */
  .composer-menu {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 100%;
    z-index: 5;
    display: flex;
    flex-direction: column;
    max-height: min(320px, 45vh);
    border-top: 1px solid var(--line);
    background: var(--surface);
    box-shadow: 0 -6px 16px rgb(0 0 0 / 0.06);
  }
  .menu-head {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 16px;
    border-bottom: 1px solid var(--line);
    background: var(--sidebar);
    color: var(--muted);
    font-size: 11px;
  }
  .keys {
    white-space: nowrap;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
  }
  li {
    display: grid;
    grid-template-columns: minmax(0, max-content) auto minmax(0, 1fr);
    align-items: baseline;
    gap: 10px;
    padding: 6px 16px;
    border-bottom: 1px solid var(--line-subtle);
    font-size: 12px;
    cursor: pointer;
  }
  li.active {
    background: var(--row-selected);
    box-shadow: inset 2px 0 var(--accent);
  }
  .name {
    font-family: 'SFMono-Regular', Consolas, 'Liberation Mono', monospace;
    color: var(--ink);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  mark {
    background: none;
    color: var(--accent-text);
    font-weight: 600;
  }
  .hint {
    margin-left: 0.6em;
    color: var(--muted);
  }
  .tag {
    color: var(--muted);
    font-size: 11px;
  }
  .detail {
    min-width: 0;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .menu-note {
    margin: 0;
    padding: 8px 16px;
    color: var(--muted);
    font-size: 12px;
  }
  @media (max-width: 760px) {
    .keys {
      display: none;
    }
    li {
      grid-template-columns: minmax(0, 1fr) auto;
    }
    .detail {
      grid-column: 1 / -1;
    }
  }
</style>
