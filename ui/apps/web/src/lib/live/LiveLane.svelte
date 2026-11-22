<!--
  A recording's lane: its calls over the last `Board.laneMs`, each a mark in
  its ending's tone, open ones running to now. Drawn in a card's foot and in
  a row of the rows view.
-->
<script lang="ts">
  import { Board, type Tile } from '#lib/board.ts';
  import { Clock, Elapsed } from '#lib/time.ts';

  let { tile }: { tile: Tile } = $props();
</script>

<svg
  class="live-lane"
  viewBox="0 0 1000 10"
  preserveAspectRatio="none"
  role="img"
  aria-label="Calls in the last {Elapsed.span(Board.laneMs)}"
>
  <line class="live-lane-base" x1="0" y1="5" x2="1000" y2="5" />
  {#each tile.lane as mark (mark.call.id)}
    <rect
      class="live-mark {mark.tone}"
      class:open={mark.open}
      x={mark.start * 1000}
      y="1"
      width={Math.max(3, (mark.end - mark.start) * 1000)}
      height="8"
      ><title
        >{mark.call.model ?? mark.call.endpoint} · {Clock.second(mark.call.started_at)}{mark.open
          ? ' · open'
          : ''}</title
      ></rect
    >
  {/each}
</svg>

<style>
  .live-lane {
    display: block;
    width: 100%;
    height: 10px;
  }
  .live-lane-base {
    stroke: var(--line);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  .live-mark {
    fill: var(--muted);
    opacity: 0.55;
  }
  .live-mark.success {
    fill: var(--success);
  }
  .live-mark.warning {
    fill: var(--warning);
    opacity: 0.9;
  }
  .live-mark.error {
    fill: var(--error);
    opacity: 0.9;
  }
  .live-mark.live,
  .live-mark.open {
    fill: var(--live);
    opacity: 1;
    animation: live-breathe 1.6s ease-in-out infinite;
  }
  @keyframes live-breathe {
    50% {
      opacity: 0.45;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .live-mark.open,
    .live-mark.live {
      animation: none;
    }
  }
</style>
