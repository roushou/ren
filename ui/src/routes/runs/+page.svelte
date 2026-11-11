<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import { goto } from '$app/navigation';
  import { ManagementApi, clock, short, type TurnActivity } from '#lib/api.ts';
  import { effectLabel, harnessLabel, resultBadge } from '#lib/activity.ts';
  import { Board } from '#lib/board.ts';
  import { date } from '#lib/display.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import {
    DRIVABLE,
    Desk,
    RunCommandError,
    Runs,
    RunsClient,
    Transcript,
    runUpdate,
    type Decision,
    type DeskRun,
    type Drivable,
    type RunFolder,
    type RecordedRun,
    type RunId,
    type RunSnapshot,
    type Waiting,
  } from '#lib/runs.ts';
  import Icon from '#lib/Icon.svelte';
  import Markdown from '#lib/Markdown.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);

  const client = new RunsClient();
  const api = new ManagementApi();

  let live = $state.raw<RunSnapshot[]>([]);
  let recorded = $state.raw<RecordedRun[]>([]);
  /** Why runs cannot be listed or driven, when they cannot. */
  let unavailable = $state<string | null>(null);
  let feed = $state<'connecting' | 'open' | 'closed'>('connecting');
  /** Each run's conversation since this page started following it. */
  const transcripts = new SvelteMap<RunId, Transcript>();
  const selected = $derived(data.selected);
  const projects = $derived(Desk.projects(live, recorded));
  const runs = $derived(projects.flatMap((project) => project.runs));
  const current = $derived<DeskRun | null>(runs.find((run) => run.run === selected) ?? null);
  const runState = $derived(current ? Desk.state(current) : null);
  const controls = $derived(runState ? Runs.controls(runState) : null);
  const mark = $derived(runState ? Runs.mark(runState) : null);
  const transcript = $derived(selected ? (transcripts.get(selected) ?? new Transcript()) : null);
  const waiting = $derived(projects.reduce((sum, project) => sum + project.waiting, 0));
  const liveCount = $derived(live.filter((run) => run.state.phase !== 'ended').length);
  /** Folders runs were started in, offered when starting another. */
  const folders = $derived([...new Set(runs.map((run) => run.workdir))]);

  /** The selected run's recorded turns, oldest first, and the session they belong to. */
  let history = $state.raw<{ run: RunId; session: string | null; turns: TurnActivity[] } | null>(
    null,
  );
  let historyError = $state<string | null>(null);

  let folder = $state('');
  /** The folders the daemon offers to start a run in. */
  let offered = $state.raw<RunFolder[]>([]);
  const choices = $derived(Runs.choose(offered, folder));
  let starting = $state<Drivable>('codex');
  let prompt = $state('');
  let busy = $state(false);
  let notice = $state<string | null>(null);
  let conversation = $state<HTMLElement>();
  let composer = $state<HTMLTextAreaElement>();
  /** Follow the newest entry unless the operator scrolled up. */
  let following = true;

  function message(error: unknown): string {
    if (error instanceof RunCommandError) return error.message;
    if (error instanceof TypeError) return 'Cannot reach the Ren UI server.';
    return error instanceof Error ? error.message : 'Something went wrong.';
  }

  async function readLive() {
    if (!data.runsControl) return;
    try {
      live = await client.live();
      unavailable = null;
    } catch (error) {
      unavailable = message(error);
    }
  }

  async function readFolders() {
    if (!data.runsControl) return;
    try {
      offered = await client.folders();
    } catch {
      // The list is a convenience; a typed path still starts a run.
    }
  }

  async function readRecorded() {
    try {
      recorded = await client.recorded(100);
    } catch {
      // The recorded list is a convenience here; live runs still work.
    }
  }

  /**
   * Read the selected run's session turns, newest ten, oldest first. Returns
   * whether the run's live turn, once ended, is now among them.
   */
  async function readHistory(run: RunId): Promise<boolean> {
    try {
      const stored = await client.recordedRun(run).catch(() => null);
      const session = stored?.session ?? null;
      if (stored) {
        recorded = recorded.some((entry) => entry.id === run)
          ? recorded.map((entry) => (entry.id === run ? stored : entry))
          : [stored, ...recorded];
      }
      if (!session) {
        if (selected === run) history = { run, session: null, turns: [] };
        return false;
      }
      const listed = await api.turns(session as never, 10);
      const turns = await Promise.all(listed.turns.map((turn) => api.turn(turn.turn)));
      turns.reverse();
      const live = transcripts.get(run);
      const landed = live?.recordedIn(turns) ?? false;
      if (selected !== run) return landed;
      history = { run, session, turns };
      historyError = null;
      // A turn now recorded no longer needs its live copy.
      if (landed) transcripts.set(run, new Transcript());
      return landed;
    } catch (error) {
      if (selected === run) historyError = message(error);
      return false;
    }
  }

  /**
   * A turn's record lands shortly after its calls finish: read the history
   * again until it holds `turn`, for up to about fifteen seconds.
   */
  async function awaitRecorded(run: RunId) {
    for (const delay of [500, 1000, 2000, 4000, 8000]) {
      await new Promise((resolve) => setTimeout(resolve, delay));
      if (selected !== run) return;
      if (await readHistory(run)) return;
    }
  }

  async function act(action: () => Promise<unknown>) {
    busy = true;
    notice = null;
    try {
      await action();
    } catch (error) {
      notice = message(error);
    } finally {
      busy = false;
    }
  }

  function select(run: RunId | null) {
    void goto(run ? `/runs?run=${run}` : '/runs');
  }

  async function start(
    resume: { harness: Drivable; workdir: string; thread: string } | null = null,
  ) {
    const dir = resume?.workdir ?? folder.trim();
    if (!dir) return;
    await act(async () => {
      const run = await client.start(
        resume?.harness ?? starting,
        dir,
        (resume?.thread ?? null) as never,
      );
      folder = '';
      await readLive();
      select(run);
    });
  }

  async function send() {
    const text = prompt.trim();
    if (!selected || !text || !controls?.prompt) return;
    const run = selected;
    await act(async () => {
      await client.prompt(run, text);
      transcripts.set(run, (transcripts.get(run) ?? new Transcript()).prompted(text));
      prompt = '';
      following = true;
    });
  }

  function answer(waiting: Waiting, decision: Decision) {
    const run = selected;
    if (!run) return;
    void act(() => client.answer(run, waiting.approval, decision));
  }

  function composerKey(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      void send();
    }
  }

  function onScroll() {
    if (!conversation) return;
    following = conversation.scrollHeight - conversation.scrollTop - conversation.clientHeight < 48;
  }

  $effect(() => {
    // Keep the newest entry in view as the conversation grows.
    void transcript?.entries;
    void history;
    if (!following || !conversation) return;
    void tick().then(() => conversation?.scrollTo({ top: conversation.scrollHeight }));
  });

  $effect(() => {
    const run = selected;
    history = null;
    historyError = null;
    following = true;
    if (run) void readHistory(run);
  });

  $effect(() => {
    // The composer is where the keyboard goes once a run takes prompts.
    if (controls?.prompt) composer?.focus({ preventScroll: true });
  });

  onMount(() => {
    void readLive();
    void readFolders();
    void readRecorded();
    if (!data.runsControl) return;
    const source = new EventSource('/control/runs/events');
    source.addEventListener('open', () => {
      feed = 'open';
      // Whatever changed while the feed was closed.
      void readLive();
    });
    source.addEventListener('error', () => {
      feed = source.readyState === EventSource.CLOSED ? 'closed' : 'connecting';
    });
    source.addEventListener('lagged', () => void readLive());
    source.addEventListener('run', (event) => {
      let update;
      try {
        update = runUpdate(JSON.parse((event as MessageEvent<string>).data), '');
      } catch {
        return;
      }
      if (update.kind === 'state') {
        if (!live.some((run) => run.run === update.run)) {
          void readLive();
        } else {
          live = Runs.update(live, update);
        }
        if (update.state.phase === 'ended' || update.state.phase === 'idle') void readRecorded();
        return;
      }
      // What was recorded before this turn, to tell its record apart.
      const read =
        history?.run === update.run ? history.turns.map((turn) => turn.native_id as string) : null;
      const before = transcripts.get(update.run) ?? new Transcript([], null, read);
      let after = before.apply(update.event);
      if (update.event.kind === 'turn_started' && read !== null) after = after.knowing(read);
      transcripts.set(update.run, after);
      if (update.event.kind === 'turn_ended' && update.run === selected) {
        void awaitRecorded(update.run);
      }
    });
    return () => source.close();
  });

  function exitText(run: DeskRun): string | null {
    const ended = Desk.state(run);
    if (ended.phase !== 'ended' || !ended.exit) return null;
    switch (ended.exit.kind) {
      case 'code':
        return `exit ${ended.exit.value}`;
      case 'signal':
        return `signal ${ended.exit.value}`;
      case 'unknown':
        return null;
    }
  }

  const resumable = $derived(
    current &&
      runState?.phase === 'ended' &&
      current.recorded?.thread &&
      Runs.drivable(current.harness) &&
      !live.some(
        (run) =>
          run.workdir === current.workdir &&
          run.state.phase !== 'ended' &&
          Runs.thread(run.state) === current.recorded?.thread,
      )
      ? {
          harness: Runs.drivable(current.harness) ?? 'codex',
          workdir: current.workdir,
          thread: current.recorded.thread,
        }
      : null,
  );
</script>

{#snippet scope()}
  <div class="scope-group scope-identity">
    <span class="scope-title">Runs</span>
    <span class="muted"
      >{liveCount} running{waiting ? ` · ${waiting} waiting for you` : ''}{data.runsControl &&
      feed !== 'open'
        ? ' · updates reconnecting'
        : ''}</span
    >
  </div>
{/snippet}

{#snippet ask(waiting: Waiting, index: number)}
  {@const line = Runs.ask(waiting.ask)}
  <div class="approval" role="group" aria-label="Approval {index + 1}">
    <div class="approval-text">
      <strong><Icon name="shield" size={13} /> {line.action}</strong>
      {#if line.detail}<pre class="approval-detail mono">{line.detail}</pre>{/if}
      {#if waiting.ask.kind === 'command' && waiting.ask.cwd}<span class="muted mono"
          >in {waiting.ask.cwd}</span
        >{/if}
      {#if line.reason}<span class="muted">{line.reason}</span>{/if}
    </div>
    <div class="approval-actions">
      <button class="button primary small" disabled={busy} onclick={() => answer(waiting, 'accept')}
        >Allow</button
      >
      <button
        class="button small"
        disabled={busy}
        onclick={() => answer(waiting, 'accept_for_session')}
        title="Allow this, and the same kind of action for the rest of the session"
        >Always allow</button
      >
      <button
        class="button small"
        disabled={busy}
        onclick={() => answer(waiting, 'decline')}
        title="Refuse it; the turn goes on">Decline</button
      >
      <button
        class="button small"
        disabled={busy}
        onclick={() => answer(waiting, 'cancel')}
        title="Refuse it and end the turn">Cancel turn</button
      >
    </div>
  </div>
{/snippet}

{#snippet recordedTurn(turn: TurnActivity)}
  {@const workdir = Board.workdir(turn)}
  {#if turn.prompt}<div class="entry prompt"><p>{turn.prompt.text}</p></div>{/if}
  {#each turn.calls as call (call.call)}
    {#each call.tool_calls as tool, index (tool.id ?? index)}
      {@const badge = resultBadge(tool.result)}
      <div class="entry tool">
        <Icon name="tool" size={12} />
        <span class="tool-name">{tool.name}</span>
        <span class="tool-target mono"
          >{#if tool.effects.length}{#each tool.effects as effect, effectIndex (effectIndex)}<span
                ><span class="muted">{effectLabel(effect.kind)}</span>
                {Board.shorten(effect.target, workdir)}</span
              >{/each}{:else}{tool.input.text.split('\n')[0]}{/if}</span
        >
        <span class="chip {badge.tone}">{badge.label}</span>
      </div>
    {/each}
    {#if call.reply?.text}<div class="entry reply"><Markdown text={call.reply.text} /></div>{/if}
  {/each}
  <div class="turn-rule">
    <span>{turn.started_at ? `${date(turn.started_at)} ${clock(turn.started_at)}` : 'Turn'}</span>
    <a href="/turns/{turn.turn}">Turn details <Icon name="arrow" size={11} /></a>
  </div>
{/snippet}

<div class="runs-page">
  <aside class="runs-sidebar" aria-label="Runs">
    {#if data.runsControl}
      <form
        class="new-run"
        onsubmit={(event) => {
          event.preventDefault();
          void start();
        }}
      >
        <label for="run-folder">Start a run in a folder</label>
        <div class="segmented" role="radiogroup" aria-label="Client">
          {#each DRIVABLE as harness (harness)}<button
              type="button"
              role="radio"
              aria-checked={starting === harness}
              class:chosen={starting === harness}
              onclick={() => (starting = harness)}>{harnessLabel(harness)}</button
            >{/each}
        </div>
        <div class="new-run-row">
          <input
            id="run-folder"
            class="mono"
            list="run-folders"
            placeholder="/home/you/dev/project"
            autocomplete="off"
            spellcheck="false"
            bind:value={folder}
          />
          <button class="button primary" disabled={busy || !folder.trim()}>Start</button>
        </div>
        <datalist id="run-folders">
          {#each folders as known (known)}<option value={known}></option>{/each}
        </datalist>
        {#if choices.length}
          <ul class="folder-choices" aria-label="Folders on the daemon's host">
            {#each choices as choice (choice.path)}
              <li>
                <button
                  type="button"
                  class:chosen={folder === choice.path}
                  title={choice.path}
                  onclick={() => (folder = choice.path)}
                  ><span>{choice.name}</span><span class="muted mono">{choice.path}</span></button
                >
              </li>
            {/each}
          </ul>
        {/if}
      </form>
    {/if}
    {#if unavailable}<p class="sidebar-note danger">{unavailable}</p>{/if}
    <nav class="projects" aria-label="Runs by folder">
      {#each projects as project (project.workdir)}
        <section class="project">
          <header title={project.workdir}>
            <Icon name="file" size={12} /><strong>{project.name}</strong>
            {#if project.waiting}<span class="chip warning">{project.waiting}</span>{/if}
          </header>
          {#each project.runs as run (run.run)}
            {@const runMark = Runs.mark(Desk.state(run))}
            <button
              class="run-row"
              class:selected={run.run === selected}
              aria-current={run.run === selected ? 'true' : undefined}
              onclick={() => select(run.run)}
            >
              <span class="phase-dot tone-{runMark.tone}"></span>
              <span class="run-label">{runMark.label}</span>
              <span class="muted mono run-when"
                >{run.started_at ? clock(run.started_at) : short(run.run)}</span
              >
            </button>
          {/each}
        </section>
      {:else}
        <p class="sidebar-note">
          {data.runsControl
            ? 'No runs yet. Start one in a folder under the daemon’s run roots.'
            : 'No recorded runs.'}
        </p>
      {/each}
    </nav>
  </aside>

  <section class="run-main" aria-label="Conversation">
    {#if !data.runsControl && !current}
      <div class="empty-state">
        <div class="empty-icon"><Icon name="lock" size={26} /></div>
        <h2>This UI is read-only</h2>
        <p>
          To start and drive agent runs here, give the UI server the daemon’s control token file
          with <code>REN_CONTROL_TOKEN_FILE</code> (and, when published with Tailscale Serve, the
          allowed logins in <code>REN_CONTROL_LOGINS</code>), and add folders to the daemon’s
          <code>[runs] roots</code>.
        </p>
      </div>
    {:else if !current || !runState || !mark || !controls}
      <div class="empty-state">
        <div class="empty-icon"><Icon name="spark" size={26} /></div>
        <h2>{selected ? 'This run is not listed' : 'Pick a run, or start one'}</h2>
        <p>
          A run is Codex or Claude Code working in one of your folders, driven from here. Each keeps
          its own sign-in; its model traffic is recorded like any session’s.
        </p>
      </div>
    {:else}
      <header class="run-head">
        <div class="run-title">
          <strong>{Runs.folder(current.workdir)}</strong>
          <span class="muted mono" title={current.workdir}>{current.workdir}</span>
        </div>
        <span class="chip phase tone-{mark.tone}"
          ><span class="phase-dot tone-{mark.tone}"></span>{mark.label}</span
        >
        {#if exitText(current)}<span class="muted mono">{exitText(current)}</span>{/if}
        <span class="muted">{harnessLabel(current.harness)} · run {short(current.run)}</span>
        <span class="run-actions">
          {#if history?.session}<a class="text-button" href="/sessions/{history.session}"
              >Calls <Icon name="arrow" size={11} /></a
            >{/if}
          {#if data.runsControl && controls.interrupt}<button
              class="button small"
              disabled={busy}
              onclick={() => selected && void act(() => client.interrupt(selected))}
              >Interrupt</button
            >{/if}
          {#if data.runsControl && controls.stop}<button
              class="button small"
              disabled={busy}
              onclick={() => selected && void act(() => client.stop(selected))}>Stop</button
            >{/if}
          {#if data.runsControl && resumable}<button
              class="button small primary"
              disabled={busy}
              onclick={() => void start(resumable)}
              title="Start a new run that continues this conversation">Resume</button
            >{/if}
        </span>
      </header>

      <div class="conversation" bind:this={conversation} onscroll={onScroll}>
        {#if historyError}<p class="entry notice-entry danger">{historyError}</p>{/if}
        {#each history?.turns ?? [] as turn (turn.turn)}{@render recordedTurn(turn)}{/each}
        {#each transcript?.entries ?? [] as entry, index (index)}
          {#if entry.kind === 'prompt'}
            <div class="entry prompt"><p>{entry.text}</p></div>
          {:else if entry.kind === 'message'}
            <div class="entry reply" class:streaming={!entry.done}>
              <Markdown text={entry.text} />
            </div>
          {:else if entry.kind === 'reasoning'}
            {#if !entry.done}<div class="entry tool muted">
                <Icon name="thought" size={12} /> Thinking…
              </div>{/if}
          {:else if entry.kind === 'command'}
            <details class="entry tool command">
              <summary>
                <Icon name="terminal" size={12} />
                <span class="tool-target mono">{entry.command}</span>
                <span
                  class="chip {entry.status === 'completed' &&
                  (entry.exit === 0 || entry.exit === null)
                    ? 'success'
                    : entry.status === 'in_progress'
                      ? 'live'
                      : entry.status === 'declined'
                        ? 'neutral'
                        : 'error'}"
                  >{entry.status === 'in_progress'
                    ? 'running'
                    : entry.exit !== null
                      ? `exit ${entry.exit}`
                      : entry.status}</span
                >
              </summary>
              {#if entry.output}<pre class="activity-pre">{entry.output}</pre>{/if}
            </details>
          {:else if entry.kind === 'file_change'}
            <div class="entry tool">
              <Icon name="pencil" size={12} />
              <span class="tool-target mono">{entry.paths.join(', ') || 'files'}</span>
              <span class="chip {entry.status === 'completed' ? 'success' : 'neutral'}"
                >{entry.status === 'in_progress' ? 'editing' : entry.status}</span
              >
            </div>
          {:else if entry.kind === 'tool'}
            <div class="entry tool">
              <Icon name="tool" size={12} /><span class="tool-name">{entry.name}</span>
              {#if !entry.done}<span class="chip live">running</span>{/if}
            </div>
          {:else if entry.kind === 'turn_end'}
            {#if entry.end.kind !== 'completed'}<p
                class="entry notice-entry {entry.end.kind === 'interrupted' ? 'warn' : 'danger'}"
              >
                Turn {entry.end.kind === 'interrupted'
                  ? 'interrupted'
                  : entry.end.kind === 'failed'
                    ? `failed${entry.end.message ? `: ${entry.end.message}` : ''}`
                    : `ended: ${entry.end.status}`}
              </p>{/if}
          {:else if entry.kind === 'notice'}
            <p class="entry notice-entry {entry.tone}">{entry.text}</p>
          {/if}
        {/each}
        {#if runState.phase === 'working' && !controls.approvals.length}<div
            class="entry tool muted working"
          >
            <span class="phase-dot tone-live"></span> Working…
          </div>{/if}
        {#if !history?.turns.length && !transcript?.entries.length && runState.phase === 'idle'}
          <p class="quiet-empty">The conversation is open. Say what to do.</p>
        {/if}
      </div>

      {#if controls.approvals.length}
        <div class="approvals" aria-label="Waiting for your answer">
          {#each controls.approvals as waiting, index (waiting.approval)}{@render ask(
              waiting,
              index,
            )}{/each}
        </div>
      {/if}
      {#if notice}<div class="notice danger" role="alert">
          <Icon name="alert" size={14} /><span>{notice}</span>
        </div>{/if}
      {#if data.runsControl && runState.phase !== 'ended'}
        <form
          class="composer"
          onsubmit={(event) => {
            event.preventDefault();
            void send();
          }}
        >
          <textarea
            bind:this={composer}
            bind:value={prompt}
            onkeydown={composerKey}
            rows="3"
            placeholder={controls.prompt
              ? `Ask ${harnessLabel(current.harness)} to do something. Enter sends, Shift+Enter adds a line.`
              : runState.phase === 'working'
                ? `${harnessLabel(current.harness)} is working. You can type the next prompt now.`
                : 'Waiting for the run to open its conversation…'}
            aria-label="Prompt"></textarea>
          <button class="button primary" disabled={busy || !controls.prompt || !prompt.trim()}
            >Send</button
          >
        </form>
      {/if}
    {/if}
  </section>
</div>

<style>
  .runs-page {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(220px, 280px) 1fr;
    background: var(--canvas);
  }
  .runs-sidebar {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--line);
    background: var(--sidebar);
  }
  .new-run {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px;
    border-bottom: 1px solid var(--line);
  }
  .new-run label {
    font-size: 11px;
    color: var(--muted);
  }
  .folder-choices {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 180px;
    overflow-y: auto;
    border: 1px solid var(--line);
  }
  .folder-choices li + li {
    border-top: 1px solid var(--line);
  }
  .folder-choices button {
    display: flex;
    width: 100%;
    gap: 8px;
    align-items: baseline;
    padding: 4px 8px;
    border-radius: 0;
    font-size: 12px;
    text-align: left;
    color: var(--ink);
    background: none;
  }
  .folder-choices button span:first-child {
    flex-shrink: 0;
    white-space: nowrap;
  }
  .folder-choices button span:last-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
  }
  .folder-choices button:hover,
  .folder-choices button.chosen {
    background: var(--surface-hover);
  }
  .new-run-row {
    display: flex;
    gap: 6px;
  }
  .new-run input {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    padding: 4px 8px;
    border: 1px solid var(--control-border);
    border-radius: 3px;
    background: var(--surface);
    color: var(--ink);
  }
  .projects {
    overflow: auto;
    min-height: 0;
  }
  .project {
    border-bottom: 1px solid var(--line);
  }
  .project header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px 4px;
    font-size: 12px;
  }
  .project header .chip {
    margin-left: auto;
  }
  .run-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 12px 6px 30px;
    border: 0;
    background: none;
    color: var(--ink);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .run-row:hover {
    background: var(--row-hover);
  }
  .run-row.selected {
    background: var(--row-selected);
  }
  .run-when {
    margin-left: auto;
    font-size: 11px;
  }
  .sidebar-note {
    margin: 0;
    padding: 12px;
    font-size: 12px;
    color: var(--muted);
  }
  .sidebar-note.danger {
    color: var(--error);
    border-bottom: 1px solid var(--line);
  }
  .phase-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--muted);
  }
  .tone-live.phase-dot {
    background: var(--live);
  }
  .tone-warn.phase-dot {
    background: var(--warning);
  }
  .tone-ok.phase-dot {
    background: var(--success);
  }
  .tone-danger.phase-dot {
    background: var(--error);
  }
  .run-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--surface);
  }
  .run-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  .run-title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .run-title strong {
    font-size: 14px;
  }
  .run-title .mono {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip.phase {
    gap: 6px;
  }
  .run-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
  }
  .conversation {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 16px 0 24px;
  }
  .entry {
    max-width: 860px;
    margin: 0 auto;
    padding: 0 24px;
    font-size: 13px;
  }
  .entry.prompt {
    margin-top: 16px;
    margin-bottom: 10px;
  }
  .entry.prompt p {
    margin: 0;
    padding: 10px 14px;
    background: var(--surface-inset);
    border-left: 2px solid var(--role-user);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .entry.reply {
    padding-top: 6px;
    padding-bottom: 6px;
  }
  .entry.reply.streaming :global(.markdown > :last-child::after) {
    content: '▍';
    color: var(--muted);
  }
  .entry.tool {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 3px;
    padding-bottom: 3px;
    font-size: 12px;
    color: var(--ink);
  }
  .entry.tool.command {
    display: block;
  }
  .entry.tool.command summary {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  .tool-name {
    color: var(--role-tool);
  }
  .tool-target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tool-target > span + span::before {
    content: ' · ';
    color: var(--muted);
  }
  .turn-rule {
    display: flex;
    justify-content: space-between;
    max-width: 812px;
    margin: 10px auto 0;
    padding: 4px 0 0;
    border-top: 1px solid var(--line-subtle);
    font-size: 11px;
    color: var(--muted);
  }
  .turn-rule a {
    color: var(--muted);
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .notice-entry {
    margin-top: 6px;
    margin-bottom: 6px;
    font-size: 12px;
  }
  .notice-entry.warn {
    color: var(--warning);
  }
  .notice-entry.danger {
    color: var(--error);
  }
  .working {
    color: var(--muted);
  }
  .approvals {
    border-top: 1px solid var(--warning-border);
    background: var(--warning-soft);
  }
  .approval {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 10px 16px;
    font-size: 12px;
  }
  .approval + .approval {
    border-top: 1px solid var(--warning-border);
  }
  .approval-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }
  .approval-text strong {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--warning);
  }
  .approval-detail {
    margin: 0;
    padding: 6px 8px;
    background: var(--surface);
    border: 1px solid var(--warning-border);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 160px;
    overflow: auto;
  }
  .approval-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .composer {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    padding: 10px 16px;
    border-top: 1px solid var(--line);
    background: var(--surface);
  }
  .composer textarea {
    flex: 1;
    min-width: 0;
    resize: vertical;
    min-height: 44px;
    max-height: 40vh;
    padding: 8px 10px;
    border: 1px solid var(--control-border);
    border-radius: 3px;
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: 13px;
  }
  @media (max-width: 760px) {
    .runs-page {
      grid-template-columns: 1fr;
      grid-template-rows: auto 1fr;
    }
    .runs-sidebar {
      border-right: 0;
      border-bottom: 1px solid var(--line);
      max-height: 40vh;
    }
    .approval {
      flex-direction: column;
    }
  }
</style>
