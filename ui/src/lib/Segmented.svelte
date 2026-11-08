<!--
  One choice out of a few, all visible: a group of toggle buttons, or of links
  when each choice is its own page (`href`).

  <Segmented label="Measure" options={measures} bind:value={measure} />
  <Segmented label="Search in" options={fields} value={field} href={fieldHref} />
-->
<script lang="ts" module>
  import type { IconName } from './Icon.svelte';

  export type SegmentedOption<T extends string> = {
    value: T;
    label: string;
    /** A tooltip, for a label that is an abbreviation. */
    title?: string;
    icon?: IconName;
  };
  /** `buttons`: compact chips, the default. `tabs`: an underlined row of tabs. */
  export type SegmentedLook = 'buttons' | 'tabs';
</script>

<script lang="ts" generics="T extends string">
  import Icon from './Icon.svelte';

  let {
    options,
    value = $bindable(),
    label,
    look = 'buttons',
    href,
    class: className = '',
    onchange,
  }: {
    options: readonly SegmentedOption<T>[];
    /** The chosen option. A value that is none of the options chooses none. */
    value: T;
    /** The group's accessible name. */
    label: string;
    look?: SegmentedLook;
    /** Each option's page: the choices become links, and the chosen one is the current page. */
    href?: (value: T) => string;
    class?: string;
    /** Called after a choice changes `value`; not called for the current value. */
    onchange?: (value: T) => void;
  } = $props();

  function choose(next: T) {
    if (next === value) return;
    value = next;
    onchange?.(next);
  }
</script>

{#snippet content(option: SegmentedOption<string>)}
  {#if option.icon}<Icon name={option.icon} size={13} />{/if}{option.label}
{/snippet}

{#if href}
  <nav class="segmented {look} {className}" aria-label={label}>
    {#each options as option (option.value)}<a
        href={href(option.value)}
        title={option.title}
        class:chosen={option.value === value}
        aria-current={option.value === value ? 'page' : undefined}>{@render content(option)}</a
      >{/each}
  </nav>
{:else}
  <div class="segmented {look} {className}" role="group" aria-label={label}>
    {#each options as option (option.value)}<button
        type="button"
        title={option.title}
        class:chosen={option.value === value}
        aria-pressed={option.value === value}
        onclick={() => choose(option.value)}>{@render content(option)}</button
      >{/each}
  </div>
{/if}
