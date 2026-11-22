<!--
  One card on the wall: a recording's state and links in its head, its body
  (`LiveTileBody`: what was asked and what it did, or, expanded, the whole
  turn), and its lane. Expanded, it covers four slots and follows new calls
  while scrolled to its end. As an outline while the page's reads are on
  their way.
-->
<script lang="ts">
  import { num, providerName, short } from '#lib/api.ts';
  import { turnHref } from '#lib/activity.ts';
  import { Board, LivePlace, type LiveView, type Tile } from '#lib/board.ts';
  import { compactOrDash } from '#lib/display.ts';
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import type { Attachment } from 'svelte/attachments';
  import LiveLane from './LiveLane.svelte';
  import LiveTileBody from './LiveTileBody.svelte';

  type Props =
    /** The page's reads are on their way: the card's outline, in this wall slot. */
    | { kind: 'outline'; slot: number }
    | {
        kind: 'tile';
        tile: Tile;
        board: Board;
        view: LiveView;
        /** Its line in the Now strip, or its band series, is under the pointer. */
        highlighted: boolean;
        /** The time elapsed labels read, every second. */
        labelAt: number;
      };
  let props: Props = $props();

  const focused = $derived(props.kind === 'tile' && props.board.focus === props.tile.recording.id);

  /**
   * The expanded card opens on its newest call and follows new ones while
   * it is scrolled to the end; scrolling up stops the following. Collapsed,
   * it does not follow.
   */
  const follow: Attachment<HTMLElement> = (node) => {
    if (!focused) return;
    let atEnd = true;
    const toEnd = () => (node.scrollTop = node.scrollHeight);
    const onscroll = () => (atEnd = node.scrollHeight - node.scrollTop - node.clientHeight < 24);
    const observer = new MutationObserver(() => {
      if (atEnd) toEnd();
    });
    node.addEventListener('scroll', onscroll);
    observer.observe(node, { childList: true, subtree: true, characterData: true });
    toEnd();
    return () => {
      observer.disconnect();
      node.removeEventListener('scroll', onscroll);
    };
  };
</script>

{#if props.kind === 'outline'}
  {@const slot = props.slot}
  <div class="live-tile state-idle" aria-hidden="true">
    <header class="live-tile-head">
      <Skeleton width="{9 + (slot % 3) * 2}em" /><span class="mono muted end"
        >ctx <Skeleton width="2.5em" /></span
      >
    </header>
    <LiveTileBody content={{ kind: 'outline', slot }} />
    <footer class="live-tile-foot mono">
      <div class="live-foot-state"><Skeleton width="5em" /></div>
      <div class="live-foot-facts"><Skeleton width="8em" /></div>
    </footer>
  </div>
{:else}
  {@const { tile, board, view, highlighted, labelAt } = props}
  {@const attention = board.attention(tile.recording.id)}
  {@const chip = Board.phaseChip(tile.phase, board.now)}
  {@const toCall = tile.callHref}
  <article
    class="live-tile state-{chip.mark} {tile.harnessClass} {attention
      ? `attention-${attention}`
      : ''}"
    class:quiet={tile.quiet && !focused}
    class:focused
    class:highlighted
  >
    <header class="live-tile-head">
      <Glyph mark={chip.mark} label={chip.label} />
      <a
        class="live-open"
        href={tile.href}
        title="{Board.clientName(tile.harness)} → {providerName(tile.recording.provider)} · {short(
          tile.recording.native_id,
        )}"><span class="live-title">{tile.title}</span></a
      >
      {#if tile.open > 1}<span
          class="mono live-open-count"
          title="Model calls in flight at once: a subagent or a side request beside the main one"
          ><span class="muted">·</span> {tile.open} in flight</span
        >{/if}
      {#if tile.pinned}<span
          class="live-pinned"
          title="Pinned: stays on the wall while the session is active"
          ><Icon name="pin" size={12} /></span
        >{/if}
      <span class="mono muted end live-state-word" title={chip.title}>{chip.label}</span>
      <div class="live-actions">
        {#if tile.turnId}<a
            class="icon-button"
            href={turnHref(tile.recording.id, tile.turnId)}
            aria-label="Open the turn"
            title="Open the turn: its prompt, tools, files, and findings"
            ><Icon name="logs" size={13} /></a
          >{/if}
        {#if toCall}<a
            class="icon-button"
            href={toCall}
            aria-label="Open the call"
            title="Open this call in its session, with the inspector"
            ><Icon name="pulse" size={13} /></a
          >{/if}
        <a
          class="icon-button live-pin"
          class:on={tile.pinned}
          href={LivePlace.href(LivePlace.togglePin(view, tile.recording.id))}
          aria-label={tile.pinned ? 'Unpin this card' : 'Pin this card'}
          title={tile.pinned
            ? 'Unpin: busier sessions may take its place'
            : 'Pin: keep it on the wall while the recording is active'}
          ><Icon name="pin" size={13} /></a
        >
        <a
          class="icon-button live-expand"
          href={LivePlace.href({ ...view, focus: focused ? null : tile.recording.id })}
          aria-label={focused ? 'Collapse this card' : 'Expand this card'}
          title={focused ? 'Collapse (Esc)' : 'Expand: the whole turn, with tool input and output'}
          ><Icon name={focused ? 'shrink' : 'expand'} size={13} /></a
        >
      </div>
    </header>
    <LiveTileBody
      content={{ kind: 'tile', tile, extent: focused ? 'turn' : 'newest', labelAt }}
      {@attach follow}
    />
    <footer class="live-tile-foot mono">
      <LiveLane {tile} />
      <span class="live-foot-facts" title="Provider-reported tokens of the newest call"
        >{tile.context === null ? '' : `${compactOrDash(tile.context)} ctx · `}{num(
          tile.recording.call_count,
        )} calls</span
      >
    </footer>
  </article>
{/if}

<style>
  .live-tile {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--surface);
  }
  /*
   * Quiet recordings fade to dim once, as they go quiet; hovering brings one
   * back at once, with no transition either way.
   */
  .live-tile.quiet {
    opacity: 0.4;
    animation: live-dim 600ms ease-out;
  }
  @keyframes live-dim {
    from {
      opacity: 1;
    }
  }
  .live-tile.quiet:hover,
  .live-tile.quiet:focus-within,
  .live-tile.quiet.highlighted {
    opacity: 1;
  }
  .live-tile.attention-warning {
    box-shadow: inset 0 2px 0 var(--warning);
  }
  .live-tile.attention-error {
    box-shadow: inset 0 2px 0 var(--error);
  }
  /* Hovering a tile, or its line in the Now strip, outlines it in its client's colour. */
  .live-tile:hover::after,
  .live-tile.highlighted::after {
    position: absolute;
    inset: 0;
    box-shadow: inset 0 0 0 2px var(--harness);
    content: '';
    pointer-events: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .live-tile.quiet {
      animation: none;
    }
  }
  .live-tile-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    min-width: 0;
    font-size: 12.5px;
  }
  /*
   * The card's state is a tab beside its title and a faint wash behind it,
   * not an edge down the whole card: stacked cards in one state would join
   * into a single bar and read as a column.
   */
  .live-tile-head {
    border-bottom: 1px solid var(--line);
    background: color-mix(in srgb, var(--state, transparent) 9%, var(--surface));
  }
  .live-tile-head::before {
    position: absolute;
    top: 0;
    bottom: -1px;
    left: 0;
    width: 3px;
    background: var(--state, transparent);
    content: '';
    pointer-events: none;
  }
  .live-tile.state-idle .live-tile-head {
    background: var(--surface);
  }
  .live-tile.state-idle .live-tile-head::before {
    background: transparent;
  }
  .live-tile.state-responding .live-tile-head::before {
    animation: state-breathe 1.6s ease-in-out infinite;
  }
  /* Unknown is dashed grey, as everywhere: never an ending, good or bad. */
  .live-tile.state-unknown .live-tile-head::before {
    background: repeating-linear-gradient(to bottom, var(--muted) 0 6px, transparent 6px 10px);
  }
  @media (prefers-reduced-motion: reduce) {
    .live-tile.state-responding .live-tile-head::before {
      animation: none;
    }
  }
  .live-tile-head > .mono {
    flex: none;
    font-size: 11px;
    white-space: nowrap;
  }
  .live-open-count {
    color: var(--live);
  }
  .end {
    margin-left: auto;
  }
  .live-open {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    text-align: left;
  }
  .live-tile-foot :global(.live-lane) {
    padding: 4px 12px 2px;
    box-sizing: content-box;
    width: calc(100% - 24px);
  }
  /*
   * A card's actions sit over the right of its header, shown while the card
   * is hovered or holds focus; always shown when expanded, or without hover.
   */
  .live-tile-head {
    position: relative;
  }
  .live-actions {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 6px 0 10px;
    background: inherit;
    opacity: 0;
    pointer-events: none;
  }
  .live-tile:hover .live-actions,
  .live-tile:focus-within .live-actions {
    opacity: 1;
    pointer-events: auto;
  }
  .live-tile.focused .live-actions {
    position: static;
    margin: -3px -6px -3px 0;
    padding: 0;
    opacity: 1;
    pointer-events: auto;
  }
  @media (hover: none) and (pointer: coarse) {
    .live-actions {
      position: static;
      margin: -3px -6px -3px 0;
      padding: 0;
      opacity: 1;
      pointer-events: auto;
    }
  }
  .live-pin.on,
  .live-pinned {
    color: var(--accent);
  }
  .live-pinned {
    display: inline-flex;
    flex: none;
  }
  /* The expanded card covers four slots and reads top down, scrolling. */
  .live-tile.focused {
    grid-column: span 2;
    grid-row: span 2;
  }
  .live-tile-foot {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    column-gap: 10px;
    align-items: center;
    padding: 6px 12px 8px;
    border-top: 1px solid var(--line);
    color: var(--muted);
    font-size: 11px;
  }
  .live-foot-facts {
    white-space: nowrap;
  }
  .live-title {
    overflow: hidden;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .live-state-word {
    flex: none;
    font-size: 11px;
    white-space: nowrap;
  }
  @media (max-width: 760px) {
    .live-tile.focused {
      grid-column: auto;
      grid-row: span 2;
    }
  }
</style>
