<!--
  The rows view: the same board as the wall, one line per recording, with
  its state, its lane, and the newest thing it did. As an outline while the
  page's reads are on their way, drawn from the same columns.
-->
<script lang="ts">
  import { providerName, short } from '#lib/api.ts';
  import { Board, type Tile } from '#lib/board.ts';
  import type { Column } from '#lib/columns.ts';
  import { compactOrDash } from '#lib/display.ts';
  import Glyph from '#lib/Glyph.svelte';
  import SkeletonRows from '#lib/SkeletonRows.svelte';
  import { Elapsed } from '#lib/time.ts';
  import LiveLane from './LiveLane.svelte';

  type Props =
    /** The page's reads are on their way. */
    { kind: 'outline' } | { kind: 'rows'; board: Board };
  let props: Props = $props();

  /** The columns: the header and the outline rows are drawn from them. */
  const columns: Column[] = [
    { label: 'Session' },
    { label: 'Now' },
    { label: `Last ${Elapsed.span(Board.laneMs)}`, class: 'lane-column' },
    { label: 'Latest', class: 'live-latest' },
    { label: 'Context', numeric: true },
  ];
</script>

{#snippet head()}
  <thead>
    <tr>
      {#each columns as column (column.label)}<th
          class={column.class}
          class:numeric={column.numeric}>{column.label}</th
        >{/each}
    </tr>
  </thead>
{/snippet}

{#snippet identity(tile: Tile)}
  <span class="harness-mark {tile.harnessClass}"></span><strong
    >{Board.clientName(tile.harness)} → {providerName(tile.recording.provider)}</strong
  >
  <span class="mono muted" title="The session ID your client reported"
    >{short(tile.recording.native_id)}</span
  >
{/snippet}

{#if props.kind === 'outline'}
  <div class="live-rows-wrap" aria-busy="true">
    <table class="live-rows">
      {@render head()}
      <tbody><SkeletonRows {columns} rows={Board.maxTiles} /></tbody>
    </table>
  </div>
{:else}
  {@const board = props.board}
  <div class="live-rows-wrap">
    <table class="live-rows">
      {@render head()}
      <tbody>
        {#each board.tiles as tile (tile.recording.id)}
          {@const chip = Board.phaseChip(tile.phase, board.now)}
          <tr class:quiet={tile.quiet}>
            <td
              ><a class="live-open" href={tile.href} title="Open the session"
                >{@render identity(tile)}</a
              ></td
            >
            <td
              ><span class="mono live-state {chip.tone}" title={chip.title}
                ><Glyph mark={chip.mark} label={chip.label} /></span
              ></td
            >
            <td class="lane-column"><LiveLane {tile} /></td>
            <td class="live-latest">{tile.lastLine}</td>
            <td class="numeric mono">{compactOrDash(tile.context)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .live-rows-wrap {
    flex: 1;
    min-height: 0;
    background: var(--surface);
  }
  .live-rows {
    width: 100%;
    font-size: 12.5px;
    border-collapse: collapse;
  }
  .live-rows th {
    padding: 5px 12px;
    font-size: 11px;
    border-bottom: 1px solid var(--line);
    color: var(--muted);
    font-weight: 500;
    text-align: left;
    white-space: nowrap;
  }
  .live-rows td {
    padding: 4px 12px;
    border-bottom: 1px solid var(--line-subtle);
    vertical-align: middle;
  }
  .live-rows .numeric {
    text-align: right;
  }
  .live-rows .lane-column {
    width: 28%;
    min-width: 160px;
  }
  .live-rows .live-latest {
    max-width: 0;
    width: 40%;
  }
  .live-latest {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /*
   * Quiet recordings fade to dim once, as they go quiet; hovering brings one
   * back at once, with no transition either way.
   */
  .live-rows tr.quiet {
    opacity: 0.4;
    animation: live-dim 600ms ease-out;
  }
  @keyframes live-dim {
    from {
      opacity: 1;
    }
  }
  .live-rows tr.quiet:hover {
    opacity: 1;
  }
  @media (prefers-reduced-motion: reduce) {
    .live-rows tr.quiet {
      animation: none;
    }
  }
  .live-open {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    text-align: left;
  }
  .live-open strong,
  .live-open .mono {
    white-space: nowrap;
  }
  .live-open strong {
    font-weight: 600;
  }
  .live-open .mono {
    overflow: hidden;
    font-size: 11px;
    text-overflow: ellipsis;
  }
  .live-open:hover strong {
    text-decoration: underline;
  }
  .live-state {
    overflow: hidden;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .live-state.live {
    color: var(--ink);
  }
  .live-state.success {
    color: var(--success);
  }
  .live-state.warning {
    color: var(--warning);
  }
  .live-state.error {
    color: var(--error);
  }
  .live-state.neutral {
    color: var(--muted);
  }
  @media (max-width: 760px) {
    .live-rows .lane-column,
    .live-rows .live-latest {
      display: none;
    }
  }
</style>
