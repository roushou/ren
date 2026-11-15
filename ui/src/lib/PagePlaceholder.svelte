<!--
  A view's outline while its data loads: the shape of the page about to
  show (tiles over panels, rows, or a sidebar beside a pane), in flat bars,
  square and flush like the views themselves. It holds no data and says
  nothing; the layout shows it only once a wait is long enough to notice.
-->
<script lang="ts">
  import type { Shape } from './view.ts';

  let { shape }: { shape: Shape } = $props();

  /** Row widths, varied so the outline reads as text rather than stripes. */
  const widths = [72, 54, 88, 63, 46, 80, 58, 69, 51, 84, 60, 75];
</script>

{#snippet rows(count: number)}
  {#each widths.slice(0, count) as width, index (index)}
    <div class="row">
      <span class="bar short"></span>
      <span class="bar" style:width="{width}%"></span>
    </div>
  {/each}
{/snippet}

<div class="placeholder shape-{shape}" aria-hidden="true">
  {#if shape === 'dashboard'}
    <div class="tiles">
      {#each [0, 1, 2, 3] as tile (tile)}
        <div class="tile"><span class="bar short"></span><span class="bar figure"></span></div>
      {/each}
    </div>
    <div class="panels">
      <div class="panel">
        <div class="heading"><span class="bar short"></span></div>
        {@render rows(6)}
      </div>
      <div class="panel">
        <div class="heading"><span class="bar short"></span></div>
        {@render rows(6)}
      </div>
    </div>
  {:else if shape === 'list'}
    <div class="panel wide">
      <div class="heading"><span class="bar short"></span></div>
      {@render rows(12)}
    </div>
  {:else}
    <div class="split">
      <div class="side">{@render rows(8)}</div>
      <div class="panel">
        <div class="heading"><span class="bar title"></span></div>
        {@render rows(5)}
      </div>
    </div>
  {/if}
</div>

<style>
  .placeholder {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--canvas);
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    border-bottom: 1px solid var(--line);
    background: var(--surface);
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 12px;
    border-right: 1px solid var(--line);
  }
  .tile:last-child {
    border-right: 0;
  }
  .panels {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .panels .panel:first-child {
    border-right: 1px solid var(--line);
  }
  .panel {
    min-width: 0;
    background: var(--surface);
  }
  .wide {
    flex: 1;
  }
  .split {
    flex: 1;
    display: grid;
    grid-template-columns: minmax(220px, 280px) 1fr;
  }
  .side {
    border-right: 1px solid var(--line);
    background: var(--sidebar);
  }
  .heading {
    padding: 12px;
    border-bottom: 1px solid var(--line);
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding: 11px 12px;
    border-bottom: 1px solid var(--line-subtle, var(--line));
  }
  .bar {
    display: block;
    height: 9px;
    background: var(--surface-inset);
    animation: breathe 1.4s ease-in-out infinite;
  }
  .bar.short {
    width: 22%;
    max-width: 140px;
  }
  .bar.figure {
    width: 40%;
    height: 20px;
  }
  .bar.title {
    width: 35%;
    height: 13px;
  }
  @keyframes breathe {
    50% {
      opacity: 0.55;
    }
  }
  @media (max-width: 760px) {
    .tiles {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .tile:nth-child(2) {
      border-right: 0;
    }
    .panels,
    .split {
      grid-template-columns: 1fr;
    }
    .panels .panel:first-child,
    .side {
      border-right: 0;
    }
    .split .side {
      display: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .bar {
      animation: none;
    }
  }
</style>
