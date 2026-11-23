<script lang="ts">
  import { Now } from '#lib/now.svelte.ts';
  import Toolbar from '#lib/Toolbar.svelte';
  import { onMount, untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import type { Shelf } from '#lib/api.ts';
  import { harnessLabel } from '#lib/activity.ts';
  import {
    Compose,
    Desk,
    Runs,
    RunsClient,
    Sessions,
    UNSTARTED,
    type DeskRun,
    type DeskSession,
    type Drivable,
    type SessionId,
    type SessionScope,
    type Waiting,
  } from '#lib/runs.ts';
  import { SignIns } from '#lib/signIn.ts';
  import type { Viewable } from '#lib/attachments.ts';
  import QuestionCard from './QuestionCard.svelte';
  import SessionInspector from './SessionInspector.svelte';
  import { Steps, type Block } from '#lib/steps.ts';
  import Icon from '#lib/Icon.svelte';
  import ImageViewer from './ImageViewer.svelte';
  import SignInSheet from '#lib/SignInSheet.svelte';
  import SessionList from './SessionList.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import type { DeskData } from './load.ts';
  import { Acting } from './acting.svelte.ts';
  import { SessionActions } from './actions.svelte.ts';
  import { DeskFeed } from './feed.svelte.ts';
  import { ComposerState } from './composer.svelte.ts';
  import ApprovalCard from './ApprovalCard.svelte';
  import ConversationOutline from './ConversationOutline.svelte';
  import PromptComposer from './PromptComposer.svelte';
  import SessionHeader from './SessionHeader.svelte';
  import SessionConversation from './SessionConversation.svelte';

  let {
    data,
    scope,
  }: {
    data: DeskData;
    /** Which sessions this desk holds: one project's, or Scratch. */
    scope: SessionScope;
  } = $props();

  const client = new RunsClient();
  /** One action at a time, and why the last one failed. */
  const acting = new Acting();
  const feed = new DeskFeed({
    client,
    data: () => data,
    scope: () => scope,
    selected: () => selected,
  });

  /** The project whose desk this is; null for Scratch. */
  const deskProject = $derived(scope.kind === 'project' ? scope.project.name : null);
  /** The session the page shows, from the URL: the one it was loaded
   * with, or the one chosen since without loading it again (`select`). */
  const chosen = $derived(
    page.shallow
      ? Sessions.openOn(page.shallow.url.pathname, Sessions.deskHref(deskProject))
      : data.session,
  );
  /** Which sessions the list shows. */
  let shelf = $state<Shelf>('current');
  /** On a phone the page shows one pane at a time: the list, a session, or
   * the new-session form that New session opened. */
  let starting = $state(false);
  const pane = $derived<'list' | 'session' | 'new'>(chosen ? 'session' : starting ? 'new' : 'list');
  const folders = $derived(
    // A project's desk has no Scratch group: a session started there would
    // leave the project.
    Sessions.folders(
      shelf === 'current' ? feed.sessions : feed.archived,
      feed.live,
      scope.kind === 'project' ? null : feed.scratch,
      chosen,
    ),
  );
  const session = $derived.by((): DeskSession | null => {
    const body = [...feed.sessions, ...feed.archived].find((entry) => entry.session === chosen);
    return body ? Sessions.desk(body, feed.live) : null;
  });
  /** The run the page shows: the session's newest process. */
  const selected = $derived(session?.run ?? null);
  const current = $derived.by((): DeskRun | null => {
    if (!session?.run) return null;
    const run = session.run;
    const record = session.runs.find((entry) => entry.id === run) ?? null;
    return {
      run,
      harness: session.harness,
      workdir: session.workdir,
      started_at: record?.started_at ?? null,
      state: feed.live.find((entry) => entry.run === run)?.state ?? null,
      recorded: record,
      settings: session.settings,
    };
  });
  const runState = $derived(current ? Desk.state(current) : null);
  /** What can be asked of it: of its client, or, with none running, a prompt. */
  const controls = $derived(runState ? Runs.controls(runState) : session ? UNSTARTED : null);
  const transcript = $derived(feed.transcript);

  /** The client whose sign-in sheet is open. */
  let signingIn = $state<Drivable | null>(null);
  /** Now, for how long ago runs started; moves every half minute. */
  const clock = new Now(30_000);
  const now = $derived(clock.current);
  /** The open session's conversation, once drawn. */
  let conversation = $state<SessionConversation>();

  /** What the selected run's client takes mid-run. */
  const sessionClient = $derived(feed.clientOf(session?.harness));
  /** The session's client is signed out here: a prompt would only fail, so none is sent. */
  const signedOut = $derived(sessionClient !== null && SignIns.signedOut(sessionClient.sign_in));
  const compose = $derived(
    session ? Compose.session(current ? runState : null, sessionClient?.midway ?? []) : null,
  );

  const composer = new ComposerState({
    client,
    feed,
    acting,
    data: () => data,
    chosen: () => chosen,
    session: () => session,
    controls: () => controls,
    compose: () => compose,
    signedOut: () => signedOut,
    onsent: () => conversation?.follow(),
  });

  /**
   * Show `next`. Shallow: the address changes (so back, forward, and a
   * reload show it), but the page's load does not run again. The desk keeps
   * every conversation it has read and the feed keeps them current, so a
   * session seen before shows at once, and a new one reads its conversation
   * itself (`DeskFeed.readConversation`).
   */
  function select(next: SessionId | null, how: 'push' | 'replace' = 'push') {
    void goto(Sessions.deskHref(deskProject, next), {
      shallow: true,
      replaceState: how === 'replace',
    });
  }

  const actions = new SessionActions({
    client,
    feed,
    acting,
    data: () => data,
    scope: () => scope,
    chosen: () => chosen,
    session: () => session,
    select: (next) => select(next),
    focus: () => composer.field?.focus(),
  });

  /** The sessions in the list's order, for j and k. */
  const order = $derived(folders.flatMap((folder) => folder.sessions));

  function typing(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLElement &&
      (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))
    );
  }

  function shortcut(event: KeyboardEvent) {
    if (event.metaKey || event.ctrlKey || event.altKey || typing(event.target)) return;
    switch (event.key) {
      case 'ArrowUp':
      case 'ArrowDown':
        conversation?.stepKey(event);
        break;
      case 'j':
      case 'k': {
        if (!order.length) return;
        event.preventDefault();
        const at = order.findIndex((entry) => entry.session === chosen);
        const next = event.key === 'j' ? Math.min(at + 1, order.length - 1) : Math.max(at - 1, 0);
        select(order[at === -1 ? 0 : next]!.session);
        break;
      }
      case 'y':
      case 'a':
      case 'd': {
        const waiting = controls?.approvals[0];
        if (!waiting) return;
        event.preventDefault();
        actions.answer(
          waiting,
          event.key === 'y' ? 'accept' : event.key === 'a' ? 'accept_for_session' : 'decline',
        );
        break;
      }
    }
  }

  /** The conversation as it shows: each turn's blocks, with each stored
   * step's time. */
  const turnBlocks = $derived(
    Runs.turns(transcript?.entries ?? []).map((turn): Block[] =>
      Steps.blocks(turn, session?.workdir ?? null, transcript?.spans),
    ),
  );
  /** Every step of the conversation, in order: ↑ and ↓ move along it. */
  const allSteps = $derived(Steps.all(turnBlocks));

  /** Quote `quote` in the composer, to ask about it. */
  function askAbout(quote: string) {
    if (!chosen) return;
    composer.quote(quote);
    composer.focusSoon();
  }

  /** The images open full size over the page, and the one shown first, if any are. */
  let viewing = $state.raw<{ images: readonly Viewable[]; start: number } | null>(null);
  function view(images: readonly Viewable[], start: number) {
    viewing = { images, start };
  }

  $effect(() => {
    const run = selected;
    // A question asked in the header is about the session it showed.
    untrack(() => actions.dropAsked());
    // Loaded with the page when it was opened on this run; read otherwise.
    // Untracked: the effect follows the selection, not the transcripts it
    // writes, or writing one would run it again.
    untrack(() => feed.show(run));
  });

  /** Turns ended while the desk is open, so the inspector reads again. */
  let settledAt = $state(0);
  $effect(() => {
    // Counted as a session opens, and again once a turn ends: not while one
    // runs. The header reads the session's tokens on the same beat.
    const settled = runState?.phase !== 'starting' && runState?.phase !== 'working';
    if (chosen && settled) untrack(() => (settledAt += 1));
  });

  $effect(() => {
    // The composer is where the keyboard goes once a run takes prompts.
    if (controls?.prompt) composer.field?.focus({ preventScroll: true });
  });

  onMount(() => {
    // Arriving from ⌘K's New session, or the desk's prompt: open a new
    // session at once, with the desk's prompt waiting in its composer.
    if (data.runsControl && page.url.searchParams.get('new') === '1') {
      starting = true;
      const prompt = page.url.searchParams.get('prompt');
      void actions.newDraft().then(() => {
        if (prompt && chosen) composer.prefill(chosen, prompt);
      });
    }
    return feed.connect();
  });
</script>

{#snippet toolbarScope()}
  <!-- The list beside says how many and what they are doing; the bar keeps
       only what concerns the feed. -->
  <div class="scope-group scope-identity">
    <span class="muted">{feed.note}</span>
  </div>
{/snippet}

{#snippet ask(waiting: Waiting, index: number)}
  {#if waiting.ask.kind === 'questions'}
    <QuestionCard
      title={waiting.ask.title}
      questions={waiting.ask.questions}
      {index}
      busy={acting.busy}
      onreply={(request) => actions.reply(waiting, request)}
      ondecide={(decision) => actions.answer(waiting, decision)}
    />
  {:else}
    <ApprovalCard
      {waiting}
      {index}
      busy={acting.busy}
      onanswer={(decision) => actions.answer(waiting, decision)}
    />
  {/if}
{/snippet}

{#snippet sessionOutline()}
  <!-- A session on its way, before the lists say what it is: its header
       and its conversation's outline. -->
  <header class="run-head" aria-busy="true">
    <div class="run-heading">
      <h2 class="run-title"><Skeleton width="42%" /></h2>
      <p class="run-facts"><Skeleton width="16em" /></p>
    </div>
  </header>
  <div class="conversation" aria-busy="true"><ConversationOutline /></div>
{/snippet}

<svelte:window onkeydown={shortcut} />

<Toolbar scope={data.runsControl && feed.note ? toolbarScope : undefined} />
<div class="runs-page" data-pane={pane}>
  <aside class="runs-sidebar" aria-label="Runs">
    {#if feed.unavailable}<p class="sidebar-note danger">{feed.unavailable}</p>{/if}
    <SessionList
      {folders}
      listing={feed.listing}
      selected={chosen}
      {now}
      title={(entry) => feed.title(entry)}
      place={(workdir) => feed.place(workdir)}
      scratch={scope.kind === 'project' ? null : feed.scratch}
      {shelf}
      archivedCount={feed.archived.length}
      busy={acting.busy}
      deletable={data.runsControl}
      deletion={actions.deletion}
      ondelete={actions.deleting}
      onshelf={(next) => {
        shelf = next;
        select(null);
      }}
      onselect={(next) => select(next)}
      oncreate={(dir) => void actions.newDraft(dir)}
      onnew={() => {
        starting = true;
        void actions.newDraft();
      }}
    />
  </aside>

  <section class="run-main" aria-label="Run">
    <button
      class="pane-back"
      type="button"
      onclick={() => {
        starting = false;
        select(null);
      }}><span class="pane-back-icon"><Icon name="chevron" size={14} /></span>Sessions</button
    >
    {#if !session || !controls}
      {#if data.runsControl && chosen && !session && feed.listing === 'pending'}
        {@render sessionOutline()}
      {:else if data.runsControl}
        <div class="empty-state">
          <h2>{chosen ? 'This session is not listed' : 'No session open'}</h2>
          <button
            class="button primary"
            type="button"
            disabled={acting.busy}
            onclick={() => void actions.newDraft()}
            ><Icon name="spark" size={12} />New session</button
          >
        </div>
        {#if acting.notice}<div class="notice danger" role="alert">
            <Icon name="alert" size={14} /><span>{acting.notice}</span>
          </div>{/if}
      {:else}
        <div class="empty-state">
          <div class="empty-icon"><Icon name="lock" size={26} /></div>
          <h2>Read-only</h2>
        </div>
      {/if}
    {:else}
      <SessionHeader
        {session}
        {current}
        {runState}
        {controls}
        title={feed.title({ run: current?.run ?? null, asked: session.asked }) ??
          `New ${harnessLabel(session.harness)} session`}
        firstPrompt={current && feed.firstPrompt(current)}
        place={feed.place(session.workdir)}
        {now}
        runsControl={data.runsControl}
        busy={acting.busy}
        asking={actions.asking}
        ondelete={actions.deleting}
        onstop={() => actions.stop()}
        onshelve={(change) => void actions.edit(change)}
      />

      <SessionConversation
        bind:this={conversation}
        {session}
        run={selected}
        {transcript}
        blocks={turnBlocks}
        steps={allSteps}
        pending={feed.conversationPending}
        readError={feed.readError}
        working={runState?.phase === 'working' && !controls.approvals.length}
        recording={current?.recorded?.recording ?? null}
        {composer}
        askable={data.runsControl && compose !== null}
        onask={askAbout}
        onview={view}
      >
        {#snippet foot()}
          {#if controls.approvals.length}
            <div class="approvals" aria-label="Waiting for your answer">
              {#each controls.approvals as waiting, index (waiting.approval)}{@render ask(
                  waiting,
                  index,
                )}{/each}
            </div>
          {/if}
          {#if acting.notice}<div class="notice danger" role="alert">
              <Icon name="alert" size={14} /><span>{acting.notice}</span>
            </div>{/if}
          {#if data.runsControl && compose && !(compose.kind === 'closed' && runState?.phase === 'ended')}
            <PromptComposer
              {composer}
              {feed}
              {client}
              {session}
              {compose}
              {controls}
              {sessionClient}
              {signedOut}
              busy={acting.busy}
              {folders}
              onmove={actions.moveDraft}
              onsettings={actions.changeSettings}
              onclient={actions.changeClient}
              onsignin={(harness) => (signingIn = harness)}
              onview={view}
            />
          {/if}
        {/snippet}
      </SessionConversation>
    {/if}
  </section>
  {#if chosen && session}
    <SessionInspector
      session={chosen}
      recording={current?.recorded?.recording ?? null}
      runs={session.runs.map((run) => run.id)}
      steps={allSteps}
      project={scope.kind === 'project' ? scope.project.name : null}
      version={settledAt}
    />
  {/if}
</div>
{#if viewing}
  <ImageViewer images={viewing.images} start={viewing.start} onclose={() => (viewing = null)} />
{/if}
{#if signingIn}
  {@const harness = signingIn}
  <SignInSheet
    {harness}
    client={harnessLabel(harness)}
    methods={feed.clientOf(harness)?.sign_in_methods ?? []}
    start={(method) => client.startSignIn(harness, method)}
    answer={(answer) => client.signInAnswer(harness, answer)}
    wait={() => client.waitSignIn(harness)}
    cancel={() => client.cancelSignIn(harness)}
    onclose={() => {
      signingIn = null;
      void feed.readClients();
    }}
  />
{/if}

<style>
  .runs-page {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(220px, 280px) 1fr;
    background: var(--canvas);
  }
  /* An open session has its inspector beside it, where there is room. */
  .runs-page[data-pane='session'] {
    grid-template-columns: minmax(220px, 280px) minmax(0, 1fr) minmax(280px, 320px);
  }
  @media (max-width: 1180px) {
    .runs-page[data-pane='session'] {
      grid-template-columns: minmax(220px, 280px) minmax(0, 1fr);
    }
    .runs-page > :global(.inspector) {
      display: none;
    }
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
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--surface);
  }
  /* A session's header on its way: as the header will be. */
  .run-head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  .run-heading {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .run-title {
    margin: 0;
    overflow: hidden;
    font-size: 15px;
    font-weight: 600;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .run-facts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin: 0;
    color: var(--muted);
  }
  /* A session's conversation on its way, under its header's outline. */
  .conversation {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .approvals {
    border-top: 1px solid var(--warning-border);
    background: var(--warning-soft);
  }
  .approvals > :global(.approval + .approval) {
    border-top: 1px solid var(--warning-border);
  }
  /* Back to the list: a phone's way out of a session. */
  .pane-back {
    display: none;
    align-items: center;
    gap: 4px;
    padding: 9px 12px;
    border: 0;
    border-bottom: 1px solid var(--line);
    background: var(--sidebar);
    color: var(--muted);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .pane-back:hover {
    color: var(--ink);
  }
  .pane-back-icon {
    display: inline-flex;
    transform: rotate(180deg);
  }
  @media (max-width: 760px) {
    .runs-page {
      grid-template-columns: 1fr;
      grid-template-rows: 1fr;
    }
    .runs-sidebar {
      border-right: 0;
    }
    .runs-page[data-pane='list'] .run-main,
    .runs-page:not([data-pane='list']) .runs-sidebar {
      display: none;
    }
    .pane-back {
      display: flex;
    }
  }
</style>
