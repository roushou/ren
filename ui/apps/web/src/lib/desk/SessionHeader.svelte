<!--
  An open session's header: how it stands, its title, and the line under
  it (its client, settings and when it was last active, in words; where it
  works, its recording and the tokens its calls used, as chips); then its
  actions, or the question a delete asked from here.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import ActionMenu, { type ActionItem } from '#lib/ActionMenu.svelte';
  import { harnessLabel } from '#lib/activity.ts';
  import { ManagementApi, short, type TokenTotals } from '#lib/api.ts';
  import { spentSummary } from '#lib/display.ts';
  import { Elapsed } from '#lib/time.ts';
  import { untrack } from 'svelte';
  import {
    Runs,
    Sessions,
    Setup,
    type Deletion,
    type DeletionEvent,
    type DeskRun,
    type DeskSession,
    type HistoryChange,
    type RunControls,
    type RunState,
    type SessionId,
  } from '#lib/runs.ts';

  let {
    session,
    current,
    runState,
    controls,
    title,
    firstPrompt,
    place,
    now,
    runsControl,
    busy,
    asking,
    ondelete,
    onstop,
    onshelve,
  }: {
    session: DeskSession;
    /** Its newest process, once one started. */
    current: DeskRun | null;
    runState: RunState | null;
    controls: RunControls;
    title: string;
    /** Its first prompt, whole, for the title's tooltip. */
    firstPrompt: string | null;
    /** Where it works, in words. */
    place: string;
    now: number;
    runsControl: boolean;
    busy: boolean;
    /** The delete asked here, of this session, when there is one. */
    asking: Exclude<Deletion, { phase: 'idle' }> | null;
    ondelete: (event: DeletionEvent) => void;
    /** Stop its client. */
    onstop: () => void;
    onshelve: (change: Exclude<HistoryChange, 'delete'>) => void;
  } = $props();

  const api = new ManagementApi();

  /**
   * How the session stands: while its client runs, when it could not
   * start, or that it is a draft. Resting, it says nothing: the next
   * prompt starts its client again.
   */
  const headerMark = $derived.by(() => {
    const marked = Sessions.mark(session, '');
    return marked.label === '' ? null : marked;
  });

  const recording = $derived(current?.recorded?.recording ?? null);

  /** The line under the title: the client, its settings and when it was
   * last active, in words. */
  const said = $derived(
    [
      harnessLabel(session.harness),
      ...Setup.words(session.settings),
      ...(session.activeAt ? [Elapsed.ago(session.activeAt, now)] : []),
    ].join(' · '),
  );

  /** What the run's menu says of it: the details the header leaves out. */
  const details = $derived(
    current
      ? [
          `Session ${short(session.session)}`,
          `Process ${short(current.run)}`,
          current.workdir,
          ...(runState?.phase === 'ended'
            ? [Runs.exitDetail(runState.exit)].filter((d) => d !== null)
            : []),
        ]
      : [],
  );

  /** The run's other actions, least risky first; destructive ones last. */
  const menu = $derived.by((): ActionItem[] => {
    const items: ActionItem[] = [];
    const target = session.session;
    if (runsControl && controls.stop) {
      items.push({ label: 'Stop its client', onselect: onstop });
    }
    if (recording) {
      const calls = recording;
      items.push({
        label: 'Open its calls',
        onselect: () => void goto(`/monitor/sessions/${calls}`),
      });
    }
    if (runsControl) {
      const archived = session.shelf === 'archived';
      items.push({
        label: archived ? 'Unarchive' : 'Archive',
        onselect: () => onshelve(archived ? 'unarchive' : 'archive'),
      });
      // Always: a running client is stopped first.
      items.push({
        label: 'Delete…',
        danger: true,
        onselect: () => ondelete({ kind: 'ask', session: target, from: 'header' }),
      });
    }
    return items;
  });

  /** The tokens the session's calls used, as the gateway recorded them:
   * read when it opens and again as each turn ends. */
  let spent = $state.raw<{ session: SessionId; tokens: TokenTotals } | null>(null);
  const spentLine = $derived(
    spent && spent.session === session.session ? spentSummary(spent.tokens) : null,
  );

  async function readSpent(target: SessionId) {
    try {
      const usage = await api.usage({ session: target });
      if (session.session === target) spent = { session: target, tokens: usage.totals.tokens };
    } catch {
      // Without it, the header leaves the tokens out.
    }
  }

  $effect(() => {
    // Not while a turn runs, when they are still coming in.
    const target = session.session;
    const settled = runState?.phase !== 'starting' && runState?.phase !== 'working';
    if (settled) untrack(() => void readSpent(target));
  });
</script>

<header class="run-head">
  <div class="run-heading">
    <div class="run-title-line">
      {#if headerMark}<Glyph mark={headerMark.mark} label={headerMark.label} size={10} />{/if}
      <h2 class="run-title" title={firstPrompt ?? undefined}>{title}</h2>
    </div>
    <p class="run-facts">
      <span class="said">{said}</span>
      <span class="head-chip mono" title={session.workdir}
        ><Icon name="folder" size={11} />{place}</span
      >
      {#if recording}<a
          class="head-chip mono"
          href="/monitor/sessions/{recording}"
          title="Its recorded calls"><Icon name="rows" size={11} />{short(recording)}</a
        >{/if}
      {#if spentLine}<span class="head-chip mono spent" title={spentLine.title}
          >{spentLine.text}</span
        >{/if}
    </p>
  </div>
  {#if asking}
    <span class="run-actions confirming" role="alert" aria-busy={asking.phase === 'deleting'}>
      <span class="confirm-text"
        >{asking.phase === 'deleting'
          ? 'Deleting this session and its recorded calls…'
          : asking.phase === 'failed'
            ? asking.reason
            : Sessions.running(session)
              ? 'Stop its client, and delete this session and its recorded calls for good?'
              : 'Delete this session and its recorded calls for good?'}</span
      >
      <button
        class="button small danger confirm"
        disabled={asking.phase === 'deleting'}
        onclick={() => ondelete({ kind: 'confirm' })}
        >{asking.phase === 'failed' ? 'Retry' : 'Delete'}</button
      >
      <button
        class="button small"
        disabled={asking.phase === 'deleting'}
        onclick={() => ondelete({ kind: 'cancel' })}>Cancel</button
      >
    </span>
  {:else}
    <span class="run-actions">
      <ActionMenu label="Session actions" items={menu} {details} disabled={busy} />
    </span>
  {/if}
</header>

<style>
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
  .run-title-line {
    display: flex;
    align-items: center;
    gap: 10px;
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
  .run-facts .said {
    margin-right: 4px;
  }
  .head-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    max-width: 320px;
    height: 20px;
    padding: 0 7px;
    overflow: hidden;
    border: 1px solid var(--line);
    border-radius: 2px;
    background: var(--surface-inset);
    color: var(--ink);
    font-size: 11px;
    text-decoration: none;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  a.head-chip:hover {
    border-color: var(--control-border);
  }
  .head-chip.spent {
    margin-left: auto;
    color: var(--muted);
  }
  .confirm-text {
    color: var(--error);
  }
  .run-actions {
    display: flex;
    flex: none;
    align-items: center;
    gap: 6px;
  }
</style>
