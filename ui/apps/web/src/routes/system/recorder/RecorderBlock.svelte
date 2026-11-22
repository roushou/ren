<!--
  One block of the Recorder page: a titled section, with what its title
  leaves to say on the header's right.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    id,
    title,
    busy,
    size = 'fit',
    aside,
    children,
  }: {
    /** Its title's id, which labels the section. */
    id: string;
    title: string;
    /** Whether what it shows is on its way; unset where nothing it shows waits. */
    busy?: boolean;
    /** Its own height, or the rest of its column's. */
    size?: 'fit' | 'grow';
    aside?: Snippet;
    children: Snippet;
  } = $props();
</script>

<section class="rec-block" class:grow={size === 'grow'} aria-labelledby={id} aria-busy={busy}>
  <header class="rec-head">
    <h2 {id} class="cap">{title}</h2>
    {@render aside?.()}
  </header>
  {@render children()}
</section>

<style>
  .rec-block {
    padding: 12px 16px 14px;
    background: var(--surface);
  }
  .rec-block.grow {
    flex: 1;
  }
  .rec-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 10px;
  }
</style>
