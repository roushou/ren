<!--
  The wall: a fixed 3 × 2 grid of cards that fills the rest of the page,
  cards touching, split by 1px rules, with its empty slots drawn as canvas.
  It narrows to two columns, then one, as the page does.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';

  let {
    children,
    empty = 0,
    ...rest
  }: {
    /** The cards. */
    children: Snippet;
    /** Slots left without a card. */
    empty?: number;
  } & HTMLAttributes<HTMLDivElement> = $props();
</script>

<div class="live-wall" {...rest}>
  {@render children()}
  {#each { length: empty }, slot (slot)}
    <div class="live-slot" aria-hidden="true"></div>
  {/each}
</div>

<style>
  .live-wall {
    display: grid;
    flex: 1;
    gap: 1px;
    min-height: 0;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    grid-template-rows: repeat(2, minmax(0, 1fr));
    background: var(--line);
  }
  .live-slot {
    background: var(--canvas);
  }
  @media (max-width: 1100px) {
    .live-wall {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      grid-template-rows: repeat(4, minmax(0, 1fr));
    }
  }
  @media (max-width: 760px) {
    .live-wall {
      flex: none;
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: none;
      grid-auto-rows: 260px;
    }
    .live-slot {
      display: none;
    }
  }
</style>
