<!--
  Redline's pane, floating over everything while marking, dragged by its
  header (or moved with the arrow keys there): the marks with their notes,
  and the brief folded until asked for. Where the brief goes is the host's:
  its `footer` fills the pane's foot, and `preview` shows the brief as the
  host renders it (plain text otherwise). Where the pane was left is
  remembered in this browser only, under `keep`.
-->
<script lang="ts">
  import { untrack, type Snippet } from 'svelte';
  import { Redline } from './redline.ts';
  import type { RedlineDesk } from './redline.svelte.ts';
  import './redline.css';

  let {
    desk,
    brief,
    keep = 'redline.pane',
    preview,
    footer,
  }: {
    desk: RedlineDesk;
    /** The brief the noted marks make. */
    brief: string;
    /** Where this browser keeps the pane's place. */
    keep?: string;
    /** The brief, as the host shows it. */
    preview?: Snippet<[string]>;
    /** The pane's foot: where the brief goes, and sending it. */
    footer?: Snippet;
  } = $props();

  /** Where the pane sits: its top-left corner, in viewport pixels. */
  const MARGIN = 8;
  let pane = $state<HTMLElement>();
  let at = $state<{ x: number; y: number }>(placed());
  let dragging = $state<{ dx: number; dy: number } | null>(null);

  /** Where this browser left the pane, else near the top right. */
  function placed(): { x: number; y: number } {
    try {
      const kept = JSON.parse(localStorage.getItem(untrack(() => keep)) ?? 'null') as {
        x?: unknown;
        y?: unknown;
      };
      if (typeof kept?.x === 'number' && typeof kept?.y === 'number')
        return { x: kept.x, y: kept.y };
    } catch {
      // Not kept; the default place.
    }
    return { x: Math.max(MARGIN, window.innerWidth - 356), y: 56 };
  }
  /** `x`, `y` moved back on screen, so the header can always be grabbed. */
  function clamp(x: number, y: number): { x: number; y: number } {
    const width = pane?.offsetWidth ?? 340;
    return {
      x: Math.min(Math.max(MARGIN, x), Math.max(MARGIN, window.innerWidth - width - MARGIN)),
      y: Math.min(Math.max(MARGIN, y), Math.max(MARGIN, window.innerHeight - 48)),
    };
  }
  function leave() {
    try {
      localStorage.setItem(keep, JSON.stringify(at));
    } catch {
      // Not kept; it opens at the default place next time.
    }
  }
  function grab(event: PointerEvent) {
    if (event.button !== 0 || (event.target as Element).closest('button')) return;
    event.preventDefault();
    dragging = { dx: event.clientX - at.x, dy: event.clientY - at.y };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }
  function drag(event: PointerEvent) {
    if (dragging) at = clamp(event.clientX - dragging.dx, event.clientY - dragging.dy);
  }
  function drop() {
    if (!dragging) return;
    dragging = null;
    leave();
  }
  function nudge(event: KeyboardEvent) {
    const step = event.shiftKey ? 64 : 16;
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    const move = moves[event.key];
    if (!move) return;
    event.preventDefault();
    at = clamp(at.x + move[0], at.y + move[1]);
    leave();
  }
  // Kept on screen as the window changes. `at` is read untracked: writing a
  // new position must not run this again.
  $effect(() => {
    const keep = () => untrack(() => (at = clamp(at.x, at.y)));
    keep();
    window.addEventListener('resize', keep);
    return () => window.removeEventListener('resize', keep);
  });

  const noted = $derived(Redline.noted(desk.marks));
</script>

<aside
  bind:this={pane}
  class="redline-panel"
  class:dragging={!!dragging}
  data-redline
  aria-label="Redline"
  style:left="{at.x}px"
  style:top="{at.y}px"
>
  <header
    role="toolbar"
    tabindex="0"
    title="Drag to move; arrow keys move it too"
    aria-label="Redline pane: drag, or use the arrow keys, to move it"
    onpointerdown={grab}
    onpointermove={drag}
    onpointerup={drop}
    onpointercancel={drop}
    onkeydown={nudge}
  >
    <span class="grip" aria-hidden="true">⠿</span>
    <strong>Redline</strong>
    <span class="count">{desk.marks.length} mark{desk.marks.length === 1 ? '' : 's'}</span>
    <button
      class="close"
      aria-label="Leave Redline"
      title="Leave Redline (Esc)"
      onclick={() => desk.toggle()}>×</button
    >
  </header>
  <div class="body">
    {#if !desk.marks.length}
      <p class="hint">
        Click anything on the page to mark it. <kbd>Esc</kbd> leaves; your marks stay.
      </p>
    {/if}

    {#each desk.marks as mark, index (mark.id)}
      <div
        class="mark"
        title="{mark.target.selector}{mark.target.source ? `\n${mark.target.source}` : ''}"
      >
        <span class="num">{index + 1}</span>
        <span class="what">{Redline.name(mark.target)}</span>
        <button
          class="remove"
          aria-label="Remove mark {index + 1}"
          title="Remove"
          onclick={() => desk.drop(mark.id)}>×</button
        >
        <span class="text" class:writing={!mark.note}>{mark.note || 'Writing a note…'}</span>
      </div>
    {/each}

    {#if noted.length}
      <details class="brief-fold">
        <summary>Brief the agent gets</summary>
        <div class="brief">
          {#if preview}{@render preview(brief)}{:else}<pre>{brief}</pre>{/if}
        </div>
      </details>
    {/if}
  </div>

  {#if footer}
    <footer>{@render footer()}</footer>
  {/if}
</aside>

<style>
  /* Above everything, Redline's own marks and note included. */
  .redline-panel {
    position: fixed;
    z-index: 2147483647;
    width: 340px;
    max-width: calc(100vw - 16px);
    max-height: calc(100vh - 16px);
    /* The body scrolls, so a menu in the foot can open past the pane. */
    overflow: visible;
    display: flex;
    flex-direction: column;
    background: var(--redline-surface);
    color: var(--redline-ink);
    border: 1px solid var(--redline-border);
    border-top: 2px solid var(--redline);
    box-shadow: var(--redline-shadow);
  }
  .redline-panel.dragging {
    user-select: none;
    box-shadow:
      var(--redline-shadow),
      0 0 0 1px var(--redline);
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 8px 8px 10px;
    border-bottom: 1px solid var(--redline-border);
    background: var(--redline-surface);
    cursor: grab;
    touch-action: none;
  }
  .dragging header {
    cursor: grabbing;
  }
  header:focus-visible {
    outline: 2px solid var(--redline);
    outline-offset: -2px;
  }
  .grip {
    color: var(--redline-muted);
    font-size: 13px;
    line-height: 1;
  }
  .close,
  .remove {
    display: grid;
    place-items: center;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 3px;
    background: none;
    color: var(--redline-muted);
    font: inherit;
    line-height: 1;
    cursor: pointer;
  }
  .close {
    width: 22px;
    height: 22px;
    font-size: 16px;
  }
  .remove {
    width: 20px;
    height: 20px;
    font-size: 14px;
  }
  .close:hover,
  .remove:hover {
    border-color: var(--redline-control);
    background: var(--redline-control-hover);
    color: var(--redline-ink);
  }
  .count {
    margin-left: auto;
    color: var(--redline-muted);
    font: 11px var(--redline-mono);
  }
  .hint {
    margin: 0;
    padding: 8px 12px;
    color: var(--redline-muted);
    font-size: 12px;
  }
  kbd {
    padding: 0 3px;
    border: 1px solid var(--redline-control);
    border-radius: 2px;
    color: var(--redline-muted);
    font: 10px var(--redline-mono);
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-x: hidden;
    overflow-y: auto;
  }
  .mark {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto;
    align-items: center;
    gap: 3px 10px;
    padding: 9px 8px 9px 12px;
    border-top: 1px solid var(--redline-border);
  }
  .hint + .mark,
  .mark:first-child {
    border-top: 0;
  }
  .num {
    width: 18px;
    height: 18px;
    display: grid;
    place-items: center;
    background: var(--redline);
    color: var(--on-redline);
    font: 700 11px var(--redline-mono);
  }
  .what {
    color: var(--redline-muted);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .text {
    grid-column: 2 / -1;
    font-size: 13px;
    overflow-wrap: anywhere;
  }
  .text.writing {
    color: var(--redline-muted);
    font-style: italic;
  }
  .brief-fold {
    border-top: 1px solid var(--redline-border);
  }
  .brief-fold summary {
    padding: 8px 12px;
    color: var(--redline-muted);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    cursor: pointer;
  }
  .brief-fold summary:hover {
    color: var(--redline-ink);
  }
  .brief {
    margin: 0 12px 10px;
    max-height: 280px;
    overflow-y: auto;
    padding: 8px 10px;
    border: 1px solid var(--redline-well-border);
    background: var(--redline-well);
  }
  pre {
    margin: 0;
    font: 12px/1.5 var(--redline-mono);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  footer {
    position: relative;
    display: grid;
    gap: 8px;
    padding: 10px 12px;
    border-top: 1px solid var(--redline-border);
    background: var(--redline-surface);
  }
  @media (max-width: 760px) {
    .redline-panel {
      max-height: 60vh;
    }
  }
</style>
