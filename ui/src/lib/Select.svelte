<!--
  A single-select dropdown: a button that opens a square listbox in the top
  layer, so no scrolling or clipped ancestor cuts it off. The keyboard
  behaviour is the pure machine in `listbox.ts`; this component turns DOM
  events into its events and carries out its actions.

  <Select label="Provider" options={providers} bind:value={provider} onchange={apply} />
-->
<script lang="ts" module>
  export type SelectOption<T extends string> = {
    value: T;
    label: string;
    /** Secondary text, shown muted after the label. */
    hint?: string;
  };
  export type SelectSize = 'regular' | 'compact';
</script>

<script lang="ts" generics="T extends string">
  import type { Snippet } from 'svelte';
  import type { Attachment } from 'svelte/attachments';
  import { on } from 'svelte/events';
  import Icon from './Icon.svelte';
  import { Listbox, type ListboxEvent, type ListboxState } from './listbox.ts';

  let {
    options,
    value = $bindable(),
    label,
    placeholder = '',
    size = 'regular',
    disabled = false,
    name,
    class: className = '',
    onchange,
    item,
  }: {
    options: readonly SelectOption<T>[];
    value: T;
    /** The accessible name; the control shows the chosen option, not this. */
    label: string;
    /** Shown when `value` is none of the options. */
    placeholder?: string;
    size?: SelectSize;
    disabled?: boolean;
    /** Submits the value with a native form under this name. */
    name?: string;
    class?: string;
    /** Called after a choice changes `value`; not called for the current value. */
    onchange?: (value: T) => void;
    /** Renders an option (and the chosen one in the button) instead of its label and hint. */
    item?: Snippet<[SelectOption<T>]>;
  } = $props();

  const id = $props.id();
  const listId = `${id}-list`;
  const optionId = (index: number) => `${id}-option-${index}`;

  let machine = $state<ListboxState>(Listbox.initial);
  let trigger = $state<HTMLButtonElement>();
  const selected = $derived(options.findIndex((option) => option.value === value));
  const current = $derived(selected >= 0 ? options[selected] : undefined);
  const labels = $derived(options.map((option) => option.label));
  const active = $derived(machine.kind === 'open' ? machine.active : -1);

  /** Feed the machine; true when the event changed something. */
  function send(event: ListboxEvent): boolean {
    const step = Listbox.step(machine, event, { labels, selected });
    machine = step.state;
    if (step.kind === 'ignored') return false;
    for (const action of step.actions) {
      switch (action.kind) {
        case 'commit': {
          const chosen = options[action.index].value;
          value = chosen;
          onchange?.(chosen);
          break;
        }
      }
    }
    return true;
  }

  // Disabling the control, or emptying its options, closes it.
  $effect(() => {
    if ((disabled || options.length === 0) && machine.kind === 'open') machine = Listbox.initial;
  });

  function keydown(event: KeyboardEvent) {
    const parsed = Listbox.key(event);
    if (!parsed || !send(parsed)) return;
    // Tab still moves focus; every other handled key is the listbox's alone,
    // so page shortcuts (Escape, `j`, `/`) do not see it.
    if (parsed.kind !== 'leave') event.preventDefault();
    event.stopPropagation();
  }

  /**
   * The open list: shown in the top layer, placed under the button (or above
   * it when there is more room there), and kept there while the page scrolls.
   * Styles go through CSSOM, which the page's CSP allows; a style attribute
   * would not be.
   */
  const popup: Attachment<HTMLElement> = (list) => {
    const place = () => {
      if (!trigger) return;
      const box = trigger.getBoundingClientRect();
      const gap = 2;
      const below = window.innerHeight - box.bottom - gap;
      const above = box.top - gap;
      const up = below < 160 && above > below;
      const style = list.style;
      style.setProperty('min-width', `${box.width}px`);
      style.setProperty('max-width', `${Math.max(box.width, window.innerWidth - box.left - 8)}px`);
      style.setProperty('max-height', `${Math.min(320, up ? above : below)}px`);
      style.setProperty('left', `${box.left}px`);
      if (up) {
        style.setProperty('bottom', `${window.innerHeight - box.top + gap}px`);
        style.removeProperty('top');
      } else {
        style.setProperty('top', `${box.bottom + gap}px`);
        style.removeProperty('bottom');
      }
    };
    if (typeof list.showPopover === 'function') list.showPopover();
    place();
    const outside = (event: Event) =>
      event.target instanceof Node &&
      !list.contains(event.target) &&
      !trigger?.contains(event.target);
    const stops = [
      on(window, 'resize', place),
      // Scrolling the list itself does not move it.
      on(window, 'scroll', (event) => outside(event) && place(), { capture: true, passive: true }),
      on(document, 'pointerdown', (event) => outside(event) && send({ kind: 'dismiss' }), {
        capture: true,
      }),
    ];
    return () => stops.forEach((stop) => stop());
  };

  // Keep the highlighted option in view as the keyboard moves it.
  $effect(() => {
    if (active < 0) return;
    document.getElementById(optionId(active))?.scrollIntoView({ block: 'nearest' });
  });
</script>

{#snippet content(option: SelectOption<T>)}
  {#if item}{@render item(option)}{:else}<span class="select-label">{option.label}</span
    >{#if option.hint}<span class="select-hint">{option.hint}</span>{/if}{/if}
{/snippet}

<button
  bind:this={trigger}
  type="button"
  class="select-trigger {size} {className}"
  role="combobox"
  aria-label={label}
  aria-haspopup="listbox"
  aria-expanded={machine.kind === 'open'}
  aria-controls={listId}
  aria-activedescendant={active >= 0 ? optionId(active) : undefined}
  {disabled}
  onclick={() => send({ kind: 'toggle' })}
  onkeydown={keydown}
  onblur={() => send({ kind: 'dismiss' })}
  ><span class="select-value"
    >{#if current}{@render content(current)}{:else}<span class="select-placeholder"
        >{placeholder}</span
      >{/if}</span
  ><Icon name="chevron" size={12} /></button
>{#if name}<input type="hidden" {name} {value} />{/if}
{#if machine.kind === 'open'}
  <!-- The button keeps focus throughout; options are announced through
       aria-activedescendant, and a press on an option must not blur it. -->
  <ul
    {@attach popup}
    id={listId}
    class="select-list {size}"
    role="listbox"
    aria-label={label}
    popover="manual"
    tabindex="-1"
    onmousedown={(event) => event.preventDefault()}
  >
    {#each options as option, index}
      <!-- Options never take focus: the button's keydown drives them. -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <li
        id={optionId(index)}
        role="option"
        class:active={index === active}
        aria-selected={index === selected}
        onpointermove={() => index !== active && send({ kind: 'point', index })}
        onclick={() => send({ kind: 'choose', index })}
      >
        {@render content(option)}{#if index === selected}<Icon name="check" size={12} />{/if}
      </li>
    {/each}
  </ul>
{/if}
