<!--
  Redline's pane, floating over everything while marking, dragged by its
  header (or moved with the arrow keys there): the marks with their
  notes, the brief exactly as the agent will get it, and where it goes: a
  new session on the client chosen, or a session already open. Either works
  in Ren's own source checkout (`REN_SOURCE_DIR`), since the marks are on
  Ren's pages. A new session gets the brief as its first prompt; an open one
  gets it as its next, queued after its turn while one runs. Sending opens
  the session. The client and the kind of place last sent to, and where the
  pane was left, are remembered in this browser only.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { harnessLabel } from './activity.ts';
  import type { Revision } from './api.ts';
  import { Redline, type Destination, type PageContext } from './redline.ts';
  import type { RedlineDesk } from './redline.svelte.ts';
  import {
    Compose,
    DRIVABLE,
    RunCommandError,
    RunsClient,
    Sessions,
    type DeskSession,
    type Drivable,
    type SessionId,
  } from './runs.ts';

  let {
    desk,
    source,
    revision,
  }: {
    desk: RedlineDesk;
    /** Ren's source checkout, where the agent works; null when unset. */
    source: string | null;
    /** What the daemon serving this page was built from, once read. */
    revision: Revision | null;
  } = $props();

  const client = new RunsClient();
  const KEEP = 'ren.redline';

  /** What this browser last sent to, read before the first frame so the
   * pane opens as it will stay: the client, and whether into an open
   * session. */
  const remembered = recall();
  /** The client picked in this pane; until then, the one last sent to. */
  let chosen = $state<Drivable | null>(null);
  /** Where the operator pointed the brief in this pane: a new session, or
   * an open one; until then, the kind of place last sent to. */
  let picked = $state<'new' | SessionId | null>(null);
  let error = $state<string | null>(null);
  let sending = $state(false);

  /** The clients read; until then the pane keeps its shape, with every
   * client waiting. */
  const read = $derived(desk.destinations.state === 'read' ? desk.destinations.value : null);
  const ready = $derived(read?.ready ?? []);
  const offered = $derived<DeskSession[]>(read?.sessions ?? []);
  /** The clients shown: those ready once read, every one while reading. */
  const shown = $derived(read ? read.ready : DRIVABLE);

  /** Where the pane sits: its top-left corner, in viewport pixels. */
  const PLACE = 'ren.redline.pane';
  const MARGIN = 8;
  let pane = $state<HTMLElement>();
  let at = $state<{ x: number; y: number }>(placed());
  let dragging = $state<{ dx: number; dy: number } | null>(null);

  /** Where this browser left the pane, else near the top right. */
  function placed(): { x: number; y: number } {
    try {
      const kept = JSON.parse(localStorage.getItem(PLACE) ?? 'null') as {
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
      localStorage.setItem(PLACE, JSON.stringify(at));
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

  /** What this browser last sent to: a convenience, so a refusal is
   * ignored. */
  function recall(): { harness: Drivable | null; into: 'new' | 'session' } {
    try {
      const value: unknown = JSON.parse(localStorage.getItem(KEEP) ?? '{}');
      const kept = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
      return {
        harness: DRIVABLE.find((drivable) => drivable === kept.harness) ?? null,
        into: kept.into === 'session' ? 'session' : 'new',
      };
    } catch {
      return { harness: null, into: 'new' };
    }
  }
  function keep() {
    try {
      localStorage.setItem(
        KEEP,
        JSON.stringify({ harness, into: destination.kind === 'new' ? 'new' : 'session' }),
      );
    } catch {
      // Not kept; the pane offers a new session on the first ready client next time.
    }
  }

  /** Where a send goes: the place picked, else, when this browser last sent
   * into an open session, the first one offered (the one waiting for you,
   * else working, else the latest), else a new session. */
  const destination = $derived.by((): Destination => {
    if (picked === 'new') return { kind: 'new' };
    const session =
      picked !== null
        ? offered.find((entry) => entry.session === picked)
        : remembered.into === 'session'
          ? offered[0]
          : undefined;
    return session ? { kind: 'session', session } : { kind: 'new' };
  });
  /** What the send button says. */
  const action = $derived.by(() => {
    if (sending) return 'Sending…';
    if (destination.kind === 'new') return `Send to ${harnessLabel(harness)}`;
    return destination.session.state?.phase === 'working'
      ? 'Queue for after its turn'
      : 'Send to this session';
  });

  /** The client a send goes to: the one picked, else the one last sent to
   * while it is ready here, else the first ready. Settled before the first
   * frame from what this browser kept, so it does not change on reading. */
  const harness = $derived<Drivable>(
    chosen ??
      (read
        ? (read.ready.find((drivable) => drivable === remembered.harness) ??
          read.ready[0] ??
          remembered.harness ??
          DRIVABLE[0])
        : (remembered.harness ?? DRIVABLE[0])),
  );

  // Start reading, if the toggle's hover has not.
  untrack(() => desk.prepare());

  const context = $derived.by((): PageContext => {
    const chosen = document.documentElement.dataset.theme;
    const dark = chosen ? chosen === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches;
    return {
      path: `${page.url.pathname}${page.url.search}`,
      route: page.route.id,
      title: document.title,
      width: window.innerWidth,
      height: window.innerHeight,
      theme: dark ? 'dark' : 'light',
      revision,
    };
  });
  const noted = $derived(desk.marks.filter((mark) => mark.note));
  const brief = $derived(Redline.brief(context, noted));

  /** A new session in `checkout`, with the brief as its first prompt. */
  async function start(checkout: string): Promise<SessionId> {
    const made = await client.createSession(harness, checkout);
    await client.toSession(made, { kind: 'prompt', text: brief, delivery: 'start' });
    return made;
  }

  /** The brief into an open session, as it stands now (read again: it may
   * have moved on since the pane read it); null when it cannot take one. */
  async function into(session: SessionId): Promise<SessionId | null> {
    const [bodies, live] = await Promise.all([client.sessions(), client.live()]);
    const body = bodies.find((entry) => entry.session === session);
    const now = body ? Sessions.desk(body, live) : null;
    if (!now || now.shelf !== 'open') {
      error = 'That session is no longer open.';
      return null;
    }
    const handoff = Redline.handoff(Compose.session(now.state, []), brief);
    switch (handoff.kind) {
      case 'send':
        await client.toSession(session, {
          kind: 'prompt',
          text: handoff.text,
          delivery: handoff.delivery,
        });
        return session;
      case 'wait':
        error = handoff.reason;
        return null;
      default: {
        const never: never = handoff;
        return never;
      }
    }
  }

  async function send() {
    error = null;
    if (!noted.length) {
      error = 'Mark something and write a note first.';
      return;
    }
    if (!source) return;
    sending = true;
    try {
      const target =
        destination.kind === 'new' ? await start(source) : await into(destination.session.session);
      if (target === null) return;
      keep();
      desk.clear();
      desk.toggle();
      await goto(`/sessions?session=${target}`);
    } catch (failure) {
      error = failure instanceof RunCommandError ? failure.message : 'The brief was not sent.';
    } finally {
      sending = false;
    }
  }
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
  <p class="hint">Click anything on the page to mark it. <kbd>Esc</kbd> leaves; your marks stay.</p>

  {#each desk.marks as mark (mark.id)}
    <div class="mark">
      <span class="num">{mark.id}</span>
      <span class="what" title={mark.target.selector}
        >{mark.target.source ?? mark.target.selector}</span
      >
      <span class="text">{mark.note || 'Writing a note…'}</span>
      <button class="text-button remove" onclick={() => desk.drop(mark.id)}>Remove</button>
    </div>
  {/each}

  {#if noted.length}
    <section>
      <h3>Brief the agent gets</h3>
      <pre class="brief">{brief}</pre>
    </section>
  {/if}

  <section>
    <h3>Send to</h3>
    <div class="places" role="radiogroup" aria-label="Session">
      <button
        class="place"
        role="radio"
        aria-checked={destination.kind === 'new'}
        onclick={() => (picked = 'new')}
      >
        <span class="title">New session</span>
      </button>
      <!-- One shape throughout: while reading, one quiet row stands where
           the open sessions arrive. -->
      {#each offered as session (session.session)}
        {@const mark = Sessions.mark(session, harnessLabel(session.harness))}
        <button
          class="place"
          role="radio"
          aria-checked={destination.kind === 'session' &&
            destination.session.session === session.session}
          onclick={() => (picked = session.session)}
        >
          <span class="title">{session.asked ?? 'New session'}</span>
          <span class="state tone-{mark.tone}"
            >{mark.label === harnessLabel(session.harness)
              ? mark.label
              : `${mark.label} · ${harnessLabel(session.harness)}`}</span
          >
        </button>
      {:else}
        <p class="place quiet">
          {read ? 'No session open here' : 'Reading the open sessions…'}
        </p>
      {/each}
    </div>
    {#if destination.kind === 'new'}
      <!-- One shape throughout: while reading, every client waits in place,
           so nothing moves when they arrive. -->
      {#if shown.length}
        <div
          class="segmented"
          class:waiting={!read}
          role="group"
          aria-label="Client"
          aria-busy={!read}
        >
          {#each shown as drivable (drivable)}
            <button
              aria-pressed={harness === drivable}
              disabled={!read}
              onclick={() => (chosen = drivable)}>{harnessLabel(drivable)}</button
            >
          {/each}
        </div>
      {:else}
        <p class="hint one-line">No client can start here. Install one on the daemon's host.</p>
      {/if}
    {/if}
    {#if source}
      <p class="where" title={source}>
        <span>Works in</span> <code><bdi>{source}</bdi></code>
      </p>
      <p class="hint" aria-live="polite">
        {desk.destinations.state === 'failed'
          ? desk.destinations.reason
          : 'Questions are answered there; changes go on a new branch in a git worktree, with nothing installed.'}
      </p>
    {:else}
      <p class="hint unset">
        Redline sends to Ren's own source checkout, which this UI server doesn't know. Set
        <code>REN_SOURCE_DIR</code> in its environment (<code>~/.config/ren/ui.env</code>) to the
        checkout, a folder under the daemon's <code>[runs]</code> roots, and restart it.
      </p>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      <button
        class="button primary redline-send"
        disabled={sending || !source || !read || (destination.kind === 'new' && !ready.length)}
        onclick={() => void send()}>{action}</button
      >
      {#if desk.marks.length}
        <button class="button" onclick={() => desk.clear()}>Clear marks</button>
      {/if}
    </div>
  </section>
</aside>

<style>
  /* Above everything, Redline's own marks and note included. */
  .redline-panel {
    position: fixed;
    z-index: 2147483647;
    width: 340px;
    max-width: calc(100vw - 16px);
    max-height: calc(100vh - 16px);
    overflow-x: hidden;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    background: var(--popover);
    border: 1px solid var(--popover-border);
    border-top: 2px solid var(--redline);
    box-shadow: var(--popover-shadow);
  }
  .redline-panel.dragging {
    user-select: none;
    box-shadow:
      var(--popover-shadow),
      0 0 0 1px var(--redline);
  }
  header {
    position: sticky;
    top: 0;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 8px 8px 10px;
    border-bottom: 1px solid var(--popover-border);
    background: var(--popover);
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
    color: var(--muted);
    font-size: 13px;
    line-height: 1;
  }
  .close {
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 3px;
    background: none;
    color: var(--muted);
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
  }
  .close:hover {
    border-color: var(--control-border);
    background: var(--surface-hover);
    color: var(--ink);
  }
  .count {
    margin-left: auto;
    color: var(--muted);
    font:
      11px 'SFMono-Regular',
      Consolas,
      monospace;
  }
  .hint {
    margin: 0;
    padding: 8px 12px;
    color: var(--muted);
    font-size: 12px;
  }
  section .hint {
    padding: 0;
  }
  .mark {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr);
    gap: 3px 10px;
    padding: 10px 12px;
    border-top: 1px solid var(--popover-border);
  }
  .num {
    width: 18px;
    height: 18px;
    display: grid;
    place-items: center;
    background: var(--redline);
    color: var(--on-redline);
    font:
      700 11px 'SFMono-Regular',
      Consolas,
      monospace;
  }
  .what {
    color: var(--muted);
    font:
      11px 'SFMono-Regular',
      Consolas,
      monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    align-self: center;
  }
  .text,
  .remove {
    grid-column: 2;
  }
  .text {
    font-size: 13px;
    overflow-wrap: anywhere;
  }
  .remove {
    justify-self: start;
    font-size: 11px;
  }
  section {
    display: grid;
    gap: 8px;
    padding: 10px 12px;
    border-top: 1px solid var(--popover-border);
  }
  h3 {
    margin: 0;
    color: var(--muted);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .brief {
    margin: 0;
    max-height: 240px;
    overflow-y: auto;
    padding: 8px 10px;
    border: 1px solid var(--line);
    background: var(--surface-subtle);
    font:
      11px/1.55 'SFMono-Regular',
      Consolas,
      monospace;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .segmented {
    display: flex;
    flex-wrap: wrap;
    width: fit-content;
    border: 1px solid var(--control-border);
    border-radius: 3px;
    overflow: hidden;
  }
  .segmented button {
    padding: 3px 9px;
    border: 0;
    border-right: 1px solid var(--control-border);
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .segmented button:last-child {
    border-right: 0;
  }
  /* Waiting: the same buttons, quiet, until the clients are read. */
  .segmented.waiting button {
    color: var(--muted);
    cursor: progress;
  }
  .one-line {
    min-height: 24px;
  }
  .segmented button[aria-pressed='true'] {
    background: var(--accent-soft);
    color: var(--accent-text);
    font-weight: 600;
  }
  .places {
    display: grid;
    border: 1px solid var(--line);
  }
  .place {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
    margin: 0;
    padding: 5px 8px;
    border: 0;
    border-top: 1px solid var(--line);
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .place:first-child {
    border-top: 0;
  }
  .place:hover:not(.quiet) {
    background: var(--surface-hover);
  }
  .place[aria-checked='true'] {
    background: var(--accent-soft);
    color: var(--accent-text);
    box-shadow: inset 2px 0 0 var(--redline);
  }
  .place.quiet {
    color: var(--muted);
    cursor: default;
  }
  .place .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .place .state {
    flex: none;
    color: var(--muted);
    font-size: 11px;
  }
  .place .state.tone-warn {
    color: var(--warning);
  }
  .place .state.tone-live {
    color: var(--success);
  }
  .place .state.tone-danger {
    color: var(--danger, var(--redline));
  }
  .where {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    margin: 0;
    font-size: 12px;
  }
  .where span {
    flex: none;
    color: var(--muted);
  }
  .where code,
  .unset code {
    font:
      11px 'SFMono-Regular',
      Consolas,
      monospace;
  }
  .where code {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  .error {
    margin: 0;
    color: var(--redline);
    font-size: 12px;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .redline-send,
  .redline-send:hover:not(:disabled) {
    background: var(--redline);
    border-color: var(--redline);
    color: var(--on-redline);
  }
  .redline-send:hover:not(:disabled) {
    background: var(--redline-hover);
    border-color: var(--redline-hover);
  }
  @media (max-width: 760px) {
    .redline-panel {
      max-height: 60vh;
    }
  }
</style>
