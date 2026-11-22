<!--
  Layer 4: do or find anything by typing (⌘K or Ctrl+K, or the top bar's
  box). Its commands come in groups: actions, work items, recorded calls
  that say what was typed (read as you type), places, and a full search
  last; the matched letters stand out. Inside a project it keeps to that
  project, and Tab widens it to all. ↑ ↓ move, Enter goes, ⌘/Ctrl+Enter
  opens beside, Escape closes.
-->
<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import Icon, { type IconName } from '#lib/Icon.svelte';
  import { ManagementApi } from '#lib/api.ts';
  import { query as searchQuery } from '#lib/query.ts';
  import { searchFieldLabel, searchHitHref, snippetParts } from '#lib/activity.ts';
  import { Commands, type Command, type CommandKind, type CommandSources } from '#lib/commands.ts';
  import type { RawSearchQuery } from '#lib/wire/RawSearchQuery.ts';

  let {
    projects,
    sources,
    here,
    onclose,
  }: {
    projects: readonly { name: string }[];
    /** What it offers besides places: canaries to promote, work items. */
    sources: CommandSources;
    /** The project the operator is in, which the palette keeps to. */
    here: string | null;
    onclose: () => void;
  } = $props();

  /** Each kind's mark in the list. */
  const ICONS: Record<CommandKind, IconName> = {
    act: 'spark',
    work: 'file',
    call: 'logs',
    go: 'chevron',
    search: 'search',
  };
  /** Recorded calls shown for a query: the first few. */
  const CALLS = 3;
  /** How long typing rests before calls are searched, in ms. */
  const REST = 200;

  let dialog = $state<HTMLDialogElement>();
  let input = $state<HTMLInputElement>();
  let query = $state('');
  let chosen = $state(0);
  /** Whether Tab widened it from the project to every project. */
  let everywhere = $state(false);
  const scope = $derived(everywhere ? null : here);
  /** Recorded calls that say what was typed. */
  let calls = $state<Command[]>([]);

  const all = $derived(Commands.all(projects, sources));
  const groups = $derived(Commands.groups([...Commands.match(all, query, scope), ...calls]));
  /** The commands in the order the list shows them, for ↑ ↓ and Enter. */
  const shown = $derived(groups.flatMap((group) => group.commands));

  $effect(() => {
    untrack(() => {
      dialog?.showModal();
      void tick().then(() => input?.focus());
    });
  });
  $effect(() => {
    // A new query or scope starts again at the top.
    void query;
    void scope;
    chosen = 0;
  });
  $effect(() => {
    const words = query.trim();
    const project = scope;
    calls = [];
    if (words.length < 3) return;
    const controller = new AbortController();
    const timer = setTimeout(() => {
      const params: RawSearchQuery = { q: words, limit: CALLS };
      if (project) params.project = project;
      new ManagementApi()
        .search(searchQuery<RawSearchQuery>(params), controller.signal)
        .then((page) => {
          calls = page.hits.slice(0, CALLS).map((hit): Command => ({
            kind: 'call',
            label: snippetParts(hit.snippet)
              .map((part) => part.text)
              .join('')
              .trim(),
            detail: searchFieldLabel(hit.field),
            href: searchHitHref(hit),
            project,
          }));
        })
        .catch(() => {
          // A search that fails leaves the rest of the palette as it is.
        });
    }, REST);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  });

  function go(command: Command | undefined, beside = false) {
    if (!command) return;
    if (beside) {
      window.open(command.href, '_blank', 'noopener');
      return;
    }
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
      go(shown[chosen], event.metaKey || event.ctrlKey);
    } else if (event.key === 'Tab' && here !== null) {
      event.preventDefault();
      everywhere = !everywhere;
    }
  }
</script>

<dialog
  bind:this={dialog}
  class="palette"
  aria-label="Go to anything or run a command"
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
      placeholder="Do something, or find a page, a work item, a call"
      aria-label="Go to anything or run a command"
      aria-controls="palette-list"
      aria-activedescendant={shown.length ? `palette-${chosen}` : undefined}
      autocomplete="off"
      spellcheck="false"
    />
    {#if here}<span class="scope muted"
        >{scope ? `in ${scope}` : 'all projects'} · <kbd>tab</kbd>
        {scope ? 'all projects' : `only ${here}`}</span
      >{/if}
  </label>
  <div id="palette-list" role="listbox" aria-label="Commands">
    {#each groups as group (group.kind)}
      <div class="group" role="group" aria-label={group.label}>
        <span class="cap">{group.label}</span>
        {#each group.commands as command, place (`${command.href}#${place}`)}
          {@const index = shown.indexOf(command)}
          <div
            id="palette-{index}"
            class="row"
            class:chosen={index === chosen}
            class:act={command.kind === 'act'}
            role="option"
            tabindex="-1"
            aria-selected={index === chosen}
            onmousemove={() => (chosen = index)}
            onclick={(event) => go(command, event.metaKey || event.ctrlKey)}
            onkeydown={() => {}}
          >
            <Icon name={ICONS[command.kind]} size={12} />
            <span class="label" class:mono={command.kind === 'call'}
              >{#each Commands.marks(command.label, query) as part, at (at)}{#if part.match}<b
                    >{part.text}</b
                  >{:else}{part.text}{/if}{/each}</span
            >
            <span class="detail">{command.detail}</span>
          </div>
        {/each}
      </div>
    {/each}
  </div>
  <footer>
    <span><kbd>↑</kbd> <kbd>↓</kbd> move</span><span><kbd>↵</kbd> go</span><span
      ><kbd>⌘↵</kbd> open beside</span
    ><span><kbd>esc</kbd> close</span>
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
  .scope {
    flex: none;
    font-size: 11px;
  }
  #palette-list {
    max-height: 56vh;
    padding: 4px 0;
    overflow: auto;
  }
  .group + .group {
    margin-top: 4px;
  }
  .cap {
    display: block;
    padding: 6px 14px 2px;
  }
  .row {
    display: grid;
    grid-template-columns: 14px minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 10px;
    height: 30px;
    padding: 0 14px;
    color: var(--muted);
    cursor: pointer;
  }
  .row.chosen {
    background: var(--surface-hover);
    color: var(--ink);
  }
  .label {
    overflow: hidden;
    color: var(--ink);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .label.mono {
    font-size: 12px;
  }
  .row.act .label {
    font-weight: 600;
  }
  b {
    color: var(--accent);
    font-weight: 700;
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
    font-family: var(--mono);
    font-size: 10px;
  }
</style>
