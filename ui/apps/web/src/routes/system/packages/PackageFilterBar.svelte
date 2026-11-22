<!--
  What the packages table shows: packages whose name, program, or path
  holds the query, narrowed to one way they stand; and the clients signed
  out, which opens the first one's details.
-->
<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import { ONLY_MARKS, type Only, type PackageFilter, type Tally } from '#lib/packages.ts';

  let {
    filter = $bindable(),
    tally,
    total,
    signedOut,
    onsignedout,
  }: {
    filter: PackageFilter;
    tally: Tally;
    /** How many packages the listing holds. */
    total: number;
    /** How many client packages are signed out. */
    signedOut: number;
    onsignedout: () => void;
  } = $props();
</script>

{#snippet narrowing(only: Only | null, count: number, word: string, tone: string)}
  <button
    class="chip filter {tone}"
    class:active={filter.only === only}
    aria-pressed={filter.only === only}
    onclick={() => (filter = { ...filter, only })}
    >{#if only}<Glyph mark={ONLY_MARKS[only]} />{/if}{count}
    {word}</button
  >
{/snippet}

<div class="filters">
  <label class="search-input search">
    <Icon name="search" size={12} />
    <input
      type="search"
      placeholder="Filter"
      aria-label="Filter packages"
      value={filter.query}
      oninput={(event) => (filter = { ...filter, query: event.currentTarget.value })}
    />
  </label>
  {@render narrowing(null, total, 'all', '')}
  {@render narrowing('managed', tally.managed, 'managed', 'success')}
  {@render narrowing('system', tally.system, 'system', '')}
  {#if tally.missing > 0}{@render narrowing('missing', tally.missing, 'missing', 'error')}{/if}
  {#if tally.updates > 0}{@render narrowing(
      'updates',
      tally.updates,
      tally.updates === 1 ? 'update' : 'updates',
      'live',
    )}{/if}
  {#if signedOut > 0}
    <button class="chip filter warning" onclick={onsignedout}
      ><Icon name="lock" size={10} />{signedOut} signed out</button
    >
  {/if}
</div>

<style>
  .filters {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line);
  }
  .filters .search {
    width: 220px;
  }
  .search {
    width: 160px;
    height: 24px;
    border: 1px solid var(--control-border);
    border-radius: 2px;
  }
  .chip.filter {
    cursor: pointer;
    font: inherit;
  }
  .chip.filter.active {
    box-shadow: inset 0 0 0 1px currentColor;
  }
</style>
