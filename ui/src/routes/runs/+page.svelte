<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import { goto } from '$app/navigation';
  import { ManagementApi, clock, short, type CallView } from '#lib/api.ts';
  import { harnessLabel } from '#lib/activity.ts';
  import { callStatus, statusBadge, tokenSummary } from '#lib/display.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import {
    Desk,
    RunCommandError,
    Runs,
    RunsClient,
    Transcript,
    runUpdate,
    type Decision,
    type DeskRun,
    type Entry,
    type StartRequest,
    Setup,
    type RunClient,
    type RunProject,
    type HistoryChange,
    type Read,
    type RecordedRun,
    type RunId,
    type Shelf,
    type RunSnapshot,
    type Waiting,
  } from '#lib/runs.ts';
  import Icon from '#lib/Icon.svelte';
  import Markdown from '#lib/Markdown.svelte';
  import NewRun from './NewRun.svelte';
  import RunList from './RunList.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);

  const client = new RunsClient();
  const api = new ManagementApi();

  /** What the server read for the first render; the feed follows from here. */
  const initial = untrack(() => data.initial);
  const first = <T,>(read: Read<T> | undefined, none: T): T =>
    read?.state === 'read' ? read.value : none;
  let live = $state.raw<RunSnapshot[]>(first(initial?.live, []));
  let recorded = $state.raw<RecordedRun[]>(first(initial?.recorded, []));
  /** Ended runs the operator archived. */
  let archived = $state.raw<RecordedRun[]>(first(initial?.archived, []));
  /** Which runs the list shows. */
  let shelf = $state<Shelf>('current');
  /** The run whose delete waits for a second click. */
  let confirming = $state<RunId | null>(null);
  /** Why runs cannot be listed or driven, when they cannot. */
  let unavailable = $state<string | null>(
    initial?.live.state === 'failed' ? initial.live.reason : null,
  );
  let feed = $state<'connecting' | 'open' | 'closed'>('connecting');
  /** Each run's conversation since this page started following it. */
  const transcripts = new SvelteMap<RunId, Transcript>();
  // The selected run's conversation, as the page was loaded with it.
  untrack(() => {
    if (data.conversation) {
      transcripts.set(data.conversation.run, new Transcript().load(data.conversation.conversation));
    }
  });
  const selected = $derived(data.selected);
  /** The folder scratch runs get theirs in; null when there is none. */
  let scratch = $state<string | null>(first(initial?.places, null)?.scratch ?? null);
  const projects = $derived(
    shelf === 'current'
      ? Desk.projects(live, recorded, new Set(archived.map((run) => run.id)), scratch)
      : Desk.projects([], archived, new Set(), scratch),
  );
  const runs = $derived(projects.flatMap((project) => project.runs));
  const current = $derived<DeskRun | null>(runs.find((run) => run.run === selected) ?? null);
  const runState = $derived(current ? Desk.state(current) : null);
  const controls = $derived(runState ? Runs.controls(runState) : null);
  const mark = $derived(runState ? Runs.mark(runState) : null);
  const transcript = $derived(selected ? (transcripts.get(selected) ?? new Transcript()) : null);
  const waiting = $derived(projects.reduce((sum, project) => sum + project.waiting, 0));
  const liveCount = $derived(live.filter((run) => run.state.phase !== 'ended').length);

  /** The selected run's recorded turns, oldest first, and the session they belong to. */
  /** Why the selected run's conversation could not be read. */
  let readError = $state<string | null>(null);

  /** The projects the daemon offers to start a run in. */
  let offered = $state.raw<RunProject[]>(first(initial?.places, null)?.projects ?? []);
  /** The clients a run can start with, and whether each is ready. */
  let clients = $state.raw<RunClient[]>(first(initial?.clients, []));
  const tabs = [
    ['conversation', 'Conversation'],
    ['activity', 'Activity'],
    ['files', 'Files'],
    ['calls', 'Calls'],
  ] as const;
  /** What the selected run shows. */
  let tab = $state<(typeof tabs)[number][0]>('conversation');
  /** The selected run's model calls, read when its Calls tab opens. */
  let calls = $state.raw<{ session: string; calls: CallView[] } | null>(null);
  /** Now, for how long ago runs started; moves every half minute. */
  let now = $state(Date.now());
  let newRun = $state<ReturnType<typeof NewRun>>();
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

  async function readProjects() {
    if (!data.runsControl) return;
    try {
      ({ projects: offered, scratch } = await client.projects());
    } catch {
      // The list is a convenience; a typed path still starts a run.
    }
  }

  async function readClients() {
    if (!data.runsControl) return;
    try {
      clients = await client.clients();
    } catch {
      // Without the list, both drivable clients are offered.
    }
  }

  async function readCalls(session: string) {
    try {
      const page = await api.sessionCalls(session as never, null);
      calls = { session, calls: page.calls };
    } catch (error) {
      notice = message(error);
    }
  }

  /** What a run was asked first, when its conversation is known here. */
  function title(run: DeskRun): string | null {
    const text = transcripts.get(run.run)?.firstPrompt() ?? null;
    return text ? text.split('\n')[0]!.trim() || null : null;
  }

  async function readRecorded() {
    try {
      [recorded, archived] = await Promise.all([
        client.recorded(100),
        client.recorded(100, 'archived'),
      ]);
    } catch {
      // The recorded list is a convenience here; live runs still work.
    }
  }

  /**
   * Archive, unarchive, or delete the selected run with its session. The
   * daemon applies it after what it is recording, so the lists are read
   * again until the run has moved.
   */
  async function edit(change: HistoryChange) {
    const run = selected;
    if (!run) return;
    confirming = null;
    await act(async () => {
      await client.edit({ kind: 'run', id: run }, change);
      for (const delay of [150, 400, 1000, 2000]) {
        await new Promise((resolve) => setTimeout(resolve, delay));
        await readRecorded();
        const moved =
          change === 'archive'
            ? archived.some((entry) => entry.id === run)
            : change === 'unarchive'
              ? recorded.some((entry) => entry.id === run)
              : ![...recorded, ...archived].some((entry) => entry.id === run);
        if (moved) break;
      }
      if (change !== 'unarchive') select(null);
    });
  }

  /**
   * Read the run's conversation, whole, as the daemon stored it, keeping
   * what streamed in meanwhile; and its record, for its session.
   */
  async function readConversation(run: RunId) {
    try {
      const [stored, conversation] = await Promise.all([
        client.recordedRun(run).catch(() => null),
        client.conversation(run),
      ]);
      if (stored) {
        recorded = recorded.some((entry) => entry.id === run)
          ? recorded.map((entry) => (entry.id === run ? stored : entry))
          : [stored, ...recorded];
      }
      transcripts.set(run, (transcripts.get(run) ?? new Transcript()).load(conversation));
      if (selected === run) readError = null;
    } catch (error) {
      if (selected === run) readError = message(error);
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

  async function start(request: StartRequest) {
    await act(async () => {
      const run = await client.start(request);
      if (request.prompt) transcripts.set(run, new Transcript().prompted(request.prompt));
      await readLive();
      select(run);
    });
  }

  /** The runs in the list's order, for j and k. */
  const order = $derived(
    projects.flatMap((project) => {
      const { open, ended } = Desk.split(project);
      return [...open, ...ended];
    }),
  );

  function typing(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLElement &&
      (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))
    );
  }

  function shortcut(event: KeyboardEvent) {
    if (event.metaKey || event.ctrlKey || event.altKey || typing(event.target)) return;
    switch (event.key) {
      case 'n':
        event.preventDefault();
        select(null);
        void tick().then(() => newRun?.focus());
        break;
      case 'j':
      case 'k': {
        if (!order.length) return;
        event.preventDefault();
        const at = order.findIndex((run) => run.run === selected);
        const next = event.key === 'j' ? Math.min(at + 1, order.length - 1) : Math.max(at - 1, 0);
        select(order[at === -1 ? 0 : next]!.run);
        break;
      }
      case 'y':
      case 'a':
      case 'd': {
        const waiting = controls?.approvals[0];
        if (!waiting) return;
        event.preventDefault();
        answer(
          waiting,
          event.key === 'y' ? 'accept' : event.key === 'a' ? 'accept_for_session' : 'decline',
        );
        break;
      }
    }
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
    if (!following || !conversation) return;
    void tick().then(() => conversation?.scrollTo({ top: conversation.scrollHeight }));
  });

  $effect(() => {
    const run = selected;
    readError = null;
    calls = null;
    confirming = null;
    tab = 'conversation';
    following = true;
    if (!run) return;
    // Loaded with the page when it was opened on this run; read otherwise.
    const given = untrack(() => data.conversation);
    if (given?.run === run) {
      transcripts.set(run, (transcripts.get(run) ?? new Transcript()).load(given.conversation));
    } else {
      void readConversation(run);
    }
  });

  $effect(() => {
    // The Calls tab reads its session's calls when it opens.
    const session = current?.recorded?.session ?? null;
    if (tab === 'calls' && session && calls?.session !== session) void readCalls(session);
  });

  $effect(() => {
    // The composer is where the keyboard goes once a run takes prompts.
    if (controls?.prompt) composer?.focus({ preventScroll: true });
  });

  onMount(() => {
    const ticking = setInterval(() => (now = Date.now()), 30_000);
    // The server read them for the first render; only what it could not
    // read is asked for again. Live runs are read once the feed opens.
    if (initial?.places.state !== 'read') void readProjects();
    if (initial?.clients.state !== 'read') void readClients();
    if (initial?.recorded.state !== 'read' || initial.archived.state !== 'read')
      void readRecorded();
    if (!initial) void readLive();
    if (!data.runsControl) return () => clearInterval(ticking);
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
      transcripts.set(
        update.run,
        (transcripts.get(update.run) ?? new Transcript()).apply(update.event),
      );
    });
    return () => {
      source.close();
      clearInterval(ticking);
    };
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
        >Allow for this run</button
      >
      <button
        class="button small"
        disabled={busy}
        onclick={() => answer(waiting, 'decline')}
        title="Refuse it; the turn goes on">Deny</button
      >
      <button
        class="button small"
        disabled={busy}
        onclick={() => answer(waiting, 'cancel')}
        title="Refuse it and end the turn">Deny and stop</button
      >
    </div>
  </div>
{/snippet}

{#snippet entryView(entry: Entry)}
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
          class="chip {entry.status === 'completed' && (entry.exit === 0 || entry.exit === null)
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
{/snippet}

{#snippet activity()}
  {@const steps = Runs.turns(transcript?.entries ?? []).map((turn) => ({
    prompt: turn[0]?.kind === 'prompt' ? turn[0].text : 'Turn',
    steps: turn.filter(
      (entry) => entry.kind === 'command' || entry.kind === 'file_change' || entry.kind === 'tool',
    ),
  }))}
  {#each steps.filter((turn) => turn.steps.length) as turn, index (index)}
    <div class="band">{turn.prompt.split('\n')[0]}</div>
    {#each turn.steps as entry, step (step)}{@render entryView(entry)}{/each}
  {:else}
    <p class="quiet-empty">No steps yet.</p>
  {/each}
{/snippet}

{#snippet files()}
  {@const written = [
    ...new Set(
      (transcript?.entries ?? []).flatMap((entry) =>
        entry.kind === 'file_change' ? entry.paths : [],
      ),
    ),
  ]}
  <div class="band">Written <span class="count">{written.length}</span></div>
  {#each written as path (path)}<div class="entry tool mono">
      <Icon name="pencil" size={11} />
      {path}
    </div>{:else}<p class="quiet-empty">Nothing written yet.</p>{/each}
{/snippet}

{#snippet callList()}
  {#if !current?.recorded?.session}
    <p class="quiet-empty">No calls recorded yet.</p>
  {:else if !calls}
    <p class="quiet-empty">Reading calls…</p>
  {:else}
    <table class="calls">
      <thead
        ><tr><th>Started</th><th>Model</th><th>Ended</th><th>Time</th><th>Tokens</th></tr></thead
      >
      <tbody>
        {#each calls.calls as call (call.id)}
          {@const badge = statusBadge(callStatus(call))}
          {@const tokens = tokenSummary(call.usage)}
          <tr>
            <td class="mono muted">{clock(call.started_at)}</td>
            <td class="mono">{call.model ?? '—'}</td>
            <td><span class="chip {badge.tone}">{badge.label}</span></td>
            <td class="mono muted"
              >{call.ended_at
                ? `${Math.round((call.ended_at - call.started_at) / 100) / 10} s`
                : '…'}</td
            >
            <td class="mono muted" title={tokens?.title}
              >{tokens ? `${tokens.input} in · ${tokens.output} out` : '—'}</td
            >
          </tr>
        {/each}
      </tbody>
    </table>
    <a class="foot-link" href="/sessions/{current.recorded.session}"
      >Open the session <Icon name="arrow" size={11} /></a
    >
  {/if}
{/snippet}

<svelte:window onkeydown={shortcut} />

<div class="runs-page">
  <aside class="runs-sidebar" aria-label="Runs">
    {#if unavailable}<p class="sidebar-note danger">{unavailable}</p>{/if}
    <RunList
      {projects}
      {selected}
      {now}
      {title}
      {shelf}
      archivedCount={archived.length}
      onshelf={(next) => {
        shelf = next;
        select(null);
      }}
      onselect={(run) => select(run)}
      onnew={() => {
        select(null);
        void tick().then(() => newRun?.focus());
      }}
    />
  </aside>

  <section class="run-main" aria-label="Run">
    {#if !current || !runState || !mark || !controls}
      {#if data.runsControl}
        {#if selected}<p class="lost muted">This run is not listed. Start another below.</p>{/if}
        <NewRun
          bind:this={newRun}
          {offered}
          {scratch}
          remembered={data.remembered}
          models={initial?.models ?? null}
          {clients}
          {runs}
          {busy}
          listModels={(harness) => client.models(harness)}
          onstart={(harness, dir, prompt, settings) =>
            void start({ harness, dir, prompt, ...settings })}
        />
        {#if notice}<div class="notice danger" role="alert">
            <Icon name="alert" size={14} /><span>{notice}</span>
          </div>{/if}
      {:else}
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
      {/if}
    {:else}
      <header class="run-head">
        <div class="run-title">
          <strong>{title(current) ?? Runs.folder(current.workdir)}</strong>
          <span class="muted mono" title={current.workdir}>{current.workdir}</span>
        </div>
        <span class="chip phase tone-{mark.tone}"
          ><span class="phase-dot tone-{mark.tone}"></span>{mark.label}</span
        >
        {#if exitText(current)}<span class="muted mono">{exitText(current)}</span>{/if}
        <span class="muted"
          >{[
            harnessLabel(current.harness),
            ...Setup.words(current.settings),
            `run ${short(current.run)}`,
          ].join(' · ')}</span
        >
        <span class="run-actions">
          {#if current.recorded?.session}<a
              class="text-button"
              href="/sessions/{current.recorded.session}">Calls <Icon name="arrow" size={11} /></a
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
          {#if data.runsControl && runState.phase === 'ended'}
            {#if confirming === current.run}
              <span class="confirm-text">Delete this run and its recorded calls for good?</span>
              <button
                class="button small danger confirm"
                disabled={busy}
                onclick={() => edit('delete')}>Delete</button
              >
              <button class="button small" onclick={() => (confirming = null)}>Cancel</button>
            {:else}
              <button
                class="button small"
                disabled={busy}
                onclick={() => edit(current.recorded?.archived_at ? 'unarchive' : 'archive')}
                title={current.recorded?.archived_at
                  ? 'List it with the current runs again'
                  : 'Hide it, with its session, from the current runs'}
                >{current.recorded?.archived_at ? 'Unarchive' : 'Archive'}</button
              >
              <button
                class="button small danger"
                disabled={busy}
                onclick={() => (confirming = current.run)}
                title="Delete it and its session's recorded calls">Delete</button
              >
            {/if}
          {/if}
          {#if data.runsControl && resumable}<button
              class="button small primary"
              disabled={busy}
              onclick={() =>
                resumable &&
                void start({
                  harness: resumable.harness,
                  dir: resumable.workdir,
                  resume: resumable.thread,
                  ...Setup.known(current.settings),
                })}
              title="Start a new run that continues this conversation">Resume</button
            >{/if}
        </span>
      </header>

      <div class="tabs" role="tablist" aria-label="What the run shows">
        {#each tabs as [name, label] (name)}
          <button
            role="tab"
            aria-selected={tab === name}
            class:chosen={tab === name}
            onclick={() => (tab = name)}>{label}</button
          >
        {/each}
      </div>

      {#if tab === 'conversation'}
        <div class="conversation" bind:this={conversation} onscroll={onScroll}>
          {#if readError}<p class="entry notice-entry danger">{readError}</p>{/if}
          {#if transcript?.earlier}<p class="quiet-empty">
              Earlier turns are not shown: the conversation is longer than one read.
            </p>{/if}
          <!-- The whole conversation, every turn open, as one scroll. -->
          {#each Runs.turns(transcript?.entries ?? []) as turn, index (index)}
            <section class="turn">
              {#each turn as entry, at (at)}{@render entryView(entry)}{/each}
            </section>
          {/each}
          {#if runState.phase === 'working' && !controls.approvals.length}<div
              class="entry tool muted working"
            >
              <span class="phase-dot tone-live"></span> Working…
            </div>{/if}
          {#if transcript?.read && !transcript.entries.length && runState.phase === 'idle'}
            <p class="quiet-empty">The conversation is open. Say what to do.</p>
          {/if}
        </div>
      {:else if tab === 'activity'}
        <div class="conversation">
          {@render activity()}
        </div>
      {:else if tab === 'files'}
        <div class="conversation">
          {@render files()}
        </div>
      {:else}
        <div class="conversation">
          {@render callList()}
        </div>
      {/if}

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
  .tabs {
    display: flex;
    border-bottom: 1px solid var(--line);
    padding: 0 12px;
  }
  .tabs button {
    padding: 8px 12px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .tabs button.chosen {
    color: var(--ink);
    border-bottom-color: var(--accent, var(--ink));
  }
  .lost {
    margin: 0;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  /* A heading row: what the rows under it belong to. */
  .band {
    padding: 7px 16px;
    border-bottom: 1px solid var(--line);
    background: var(--sidebar);
    color: var(--muted);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .band .count {
    margin-left: 4px;
    color: var(--ink);
  }
  table.calls {
    font-size: 12px;
  }
  table.calls th:first-child,
  table.calls td:first-child {
    padding-left: 16px;
  }
  table.calls td {
    vertical-align: middle;
  }
  .foot-link {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  .run-main :global(.quiet-empty) {
    margin: 0;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  .run-main > .empty-state {
    margin: auto;
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
  .confirm-text {
    color: var(--error);
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
  }
  /* A turn: its prompt, steps, and reply as rows, ruled off from the next. */
  .turn {
    border-bottom: 1px solid var(--line);
  }
  .entry {
    padding: 0 16px;
    font-size: 13px;
  }
  .entry.prompt p {
    margin: 0;
    padding: 10px 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .entry.prompt {
    background: var(--surface-inset);
    box-shadow: inset 2px 0 var(--role-user);
    border-bottom: 1px solid var(--line);
  }
  .entry.reply {
    padding-top: 10px;
    padding-bottom: 10px;
  }
  .entry.reply.streaming :global(.markdown > :last-child::after) {
    content: '▍';
    color: var(--muted);
  }
  .entry.tool {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 6px;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--line-subtle);
    font-size: 12px;
    color: var(--ink);
  }
  .entry.tool.muted {
    color: var(--muted);
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
  .notice-entry {
    margin: 0;
    padding-top: 8px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--line-subtle);
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
  /* The prompt fills its strip; only the Send control sits inside it. */
  .composer {
    display: flex;
    align-items: flex-end;
    border-top: 1px solid var(--line);
    background: var(--surface);
  }
  .composer:focus-within {
    box-shadow: inset 2px 0 var(--accent);
  }
  .composer textarea {
    flex: 1;
    min-width: 0;
    resize: none;
    min-height: 64px;
    max-height: 40vh;
    padding: 12px 16px;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--ink);
    font: inherit;
    font-size: 13px;
  }
  .composer .button {
    margin: 0 12px 12px 0;
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
