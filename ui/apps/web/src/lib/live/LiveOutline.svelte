<!--
  The Live page while its reads are on their way: the band, the Now strip,
  and the wall or the rows, each drawn as its own outline, so the page keeps
  its shape when the reads arrive.
-->
<script lang="ts">
  import { Board, type BandBy, type Density } from '#lib/board.ts';
  import LiveBand from './LiveBand.svelte';
  import LiveRows from './LiveRows.svelte';
  import LiveTile from './LiveTile.svelte';
  import LiveWall from './LiveWall.svelte';
  import NowStrip from './NowStrip.svelte';

  let { by, density }: { by: BandBy; density: Density } = $props();
</script>

<LiveBand kind="outline" {by} />
<NowStrip kind="outline" />
{#if density === 'compact'}
  <LiveRows kind="outline" />
{:else}
  <LiveWall aria-busy="true">
    {#each { length: Board.maxTiles }, slot (slot)}
      <LiveTile kind="outline" {slot} />
    {/each}
  </LiveWall>
{/if}
