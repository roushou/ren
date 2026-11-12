<!--
  A chip that opens a searchable list: type to narrow it, ↑ and ↓ to move,
  Enter to choose, Esc to close. Options come in groups, may say why they
  cannot be chosen, and carry a muted hint, with an icon that says what it
  is (a folder's branch). An option with a detail (a model's description)
  takes two lines, and a badge marks one apart (the default). A note says
  why the list is short (still loading, or why nothing is listed).

  <Picker label="Project" options={groups} value={path} onchange={(path) => (chosen = path)} />
-->
<script lang="ts" module>
  export type PickerOption = {
    value: string;
    label: string;
    hint?: string;
    /** What the hint is, shown before it. */
    hintIcon?: IconName;
    /** Why it cannot be chosen; it is shown, but not chosen. */
    unavailable?: string | null;
    /** A second line under the label. */
    detail?: string | null;
    /** A short mark beside the label. */
    badge?: string | null;
  };
  export type PickerGroup = { label: string | null; options: PickerOption[] };
</script>

<script lang="ts">
  import { tick } from 'svelte';
  import Icon, { type IconName } from './Icon.svelte';

  let {
    label,
    icon,
    groups,
    value,
    placeholder,
    search = true,
    free = null,
    note = null,
    onchange,
  }: {
    /** The accessible name. */
    label: string;
    icon: IconName;
    groups: readonly PickerGroup[];
    value: string | null;
    /** Shown on the chip when nothing is chosen. */
    placeholder: string;
    /** Whether the list can be narrowed by typing. */
    search?: boolean;
    /** Typed text that is a value of its own when it passes this test (a full path). */
    free?: ((typed: string) => boolean) | null;
    /** Said at the foot of the list: why it is short, or still loading. */
    note?: string | null;
    onchange: (value: string) => void;
  } = $props();

  const id = $props.id();
  let isOpen = $state(false);
  let typed = $state('');
  let active = $state(0);
  let field = $state<HTMLInputElement>();
  let panel = $state<HTMLElement>();
  let chip = $state<HTMLButtonElement>();

  const all = $derived(groups.flatMap((group) => group.options));
  const current = $derived(all.find((option) => option.value === value));
  const shown = $derived.by(() => {
    const text = typed.trim().toLowerCase();
    return groups
      .map((group) => ({
        label: group.label,
        options: group.options.filter(
          (option) =>
            !text ||
            option.label.toLowerCase().includes(text) ||
            option.value.toLowerCase().includes(text) ||
            !!option.detail?.toLowerCase().includes(text),
        ),
      }))
      .filter((group) => group.options.length);
  });
  const typedValue = $derived(free && free(typed.trim()) ? typed.trim() : null);
  const choices = $derived([
    ...(typedValue ? [{ value: typedValue, label: typedValue, hint: 'this folder' }] : []),
    ...shown.flatMap((group) => group.options),
  ]);

  /** Open the list, as a click on the chip does. */
  export function open() {
    if (!isOpen) void show();
  }

  async function show() {
    isOpen = true;
    typed = '';
    active = Math.max(
      0,
      choices.findIndex((option) => option.value === value),
    );
    await tick();
    (search ? field : panel)?.focus();
  }

  function close(focusChip = true) {
    isOpen = false;
    if (focusChip) chip?.focus();
  }

  function choose(option: PickerOption) {
    if (option.unavailable) return;
    onchange(option.value);
    close();
  }

  function key(event: KeyboardEvent) {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        active = Math.min(active + 1, choices.length - 1);
        break;
      case 'ArrowUp':
        event.preventDefault();
        active = Math.max(active - 1, 0);
        break;
      case 'Enter': {
        event.preventDefault();
        const option = choices[active];
        if (option) choose(option);
        break;
      }
      case 'Escape':
        event.preventDefault();
        event.stopPropagation();
        close();
        break;
    }
  }

  function outside(event: PointerEvent) {
    if (!isOpen) return;
    const target = event.target as Node;
    if (!panel?.contains(target) && !chip?.contains(target)) close(false);
  }
</script>

<svelte:window onpointerdown={outside} />

<div class="picker">
  <button
    bind:this={chip}
    type="button"
    class="chip-button"
    aria-haspopup="listbox"
    aria-expanded={isOpen}
    aria-label={label}
    onclick={() => (isOpen ? close() : void show())}
  >
    <Icon name={icon} size={12} />
    <span class:placeholder={!current && !value}>{current?.label ?? value ?? placeholder}</span>
    {#if current?.hint}<span class="hint muted"
        >{#if current.hintIcon}<Icon name={current.hintIcon} size={11} />{/if}<span class="mono"
          >{current.hint}</span
        ></span
      >{/if}
    <Icon name="chevron" size={10} />
  </button>
  {#if isOpen}
    <div class="panel" bind:this={panel} tabindex="-1" role="presentation" onkeydown={key}>
      {#if search}
        <input
          bind:this={field}
          bind:value={typed}
          class="search mono"
          placeholder="Search{free ? ', or a full path' : ''}"
          autocomplete="off"
          spellcheck="false"
          aria-controls="{id}-list"
          oninput={() => (active = 0)}
        />
      {/if}
      <ul id="{id}-list" role="listbox" aria-label={label}>
        {#if typedValue}
          <li
            role="option"
            aria-selected={active === 0}
            class:active={active === 0}
            onpointerdown={(event) => {
              event.preventDefault();
              choose({ value: typedValue, label: typedValue });
            }}
          >
            <span class="mono">{typedValue}</span><span class="muted">this folder</span>
          </li>
        {/if}
        {#each shown as group, groupIndex (group.label ?? groupIndex)}
          {#if group.label}<li class="group" role="presentation">{group.label}</li>{/if}
          {#each group.options as option (option.value)}
            {@const index = choices.indexOf(option)}
            <li
              role="option"
              aria-selected={option.value === value}
              aria-disabled={option.unavailable ? 'true' : undefined}
              class:active={index === active}
              class:chosen={option.value === value}
              class:unavailable={!!option.unavailable}
              title={option.unavailable ?? option.value}
              onpointerenter={() => (active = index)}
              onpointerdown={(event) => {
                event.preventDefault();
                choose(option);
              }}
            >
              {#if option.detail}
                <span class="rich">
                  <span class="name"
                    >{option.label}{#if option.badge}<span class="badge">{option.badge}</span
                      >{/if}</span
                  >
                  <span class="detail muted">{option.detail}</span>
                </span>
                {#if option.hint}<span class="hint muted mono">{option.hint}</span>{/if}
              {:else}
                <span class="name"
                  >{option.label}{#if option.badge}<span class="badge">{option.badge}</span
                    >{/if}</span
                >
                {#if option.unavailable}<span class="muted">{option.unavailable}</span>{:else}<span
                    class="hint muted"
                    >{#if option.hint && option.hintIcon}<Icon
                        name={option.hintIcon}
                        size={11}
                      />{/if}<span class="mono">{option.hint ?? ''}</span></span
                  >{/if}
              {/if}
            </li>
          {/each}
        {:else}
          {#if !typedValue && !note}<li class="group" role="presentation">Nothing matches</li>{/if}
        {/each}
      </ul>
      {#if note}<p class="note muted">{note}</p>{/if}
    </div>
  {/if}
</div>

<style>
  .picker {
    position: relative;
  }
  .chip-button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 8px;
    border: 1px solid var(--control-border);
    border-radius: 3px;
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .chip-button:hover,
  .chip-button[aria-expanded='true'] {
    background: var(--surface-hover);
  }
  /* A hint stands apart from its label: a rule, then its icon and text. */
  .chip-button .hint {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-left: 2px;
    padding-left: 8px;
    border-left: 1px solid var(--line);
  }
  li .hint {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }
  .chip-button .placeholder {
    color: var(--muted);
  }
  .panel {
    position: absolute;
    bottom: calc(100% + 4px);
    left: 0;
    z-index: 20;
    width: min(420px, 80vw);
    max-height: 340px;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--line);
    background: var(--surface);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.25);
    outline: none;
  }
  .search {
    margin: 0;
    padding: 8px 10px;
    border: 0;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
    color: var(--ink);
    font-size: 12px;
    outline: none;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
  }
  li[role='option'] {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  li[role='option'] > span:last-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  li.active {
    background: var(--row-hover);
  }
  li.chosen .name {
    font-weight: 600;
  }
  /* A label over its detail: the name, then a quieter line. */
  .rich {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .rich .detail {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
  }
  li[role='option']:has(.rich) {
    align-items: center;
    padding: 7px 10px;
  }
  .name {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .badge {
    padding: 0 5px;
    border: 1px solid var(--line);
    color: var(--muted);
    font-size: 10px;
    font-weight: 400;
    line-height: 15px;
    text-transform: lowercase;
  }
  .note {
    margin: 0;
    padding: 8px 10px;
    border-top: 1px solid var(--line);
    font-size: 11px;
  }
  li.unavailable {
    cursor: not-allowed;
    color: var(--muted);
  }
  li.group {
    padding: 8px 10px 4px;
    font-size: 11px;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
</style>
