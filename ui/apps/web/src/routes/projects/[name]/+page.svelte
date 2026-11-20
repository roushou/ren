<!--
  A project's Overview (docs/plans/hierarchy.md): where its release stands,
  its work under way, its sessions running, and, beside them, what waits on
  the operator in it. Sessions and Release have pages of their own beside
  this one; a work item opens in a drawer here.
-->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import Icon from '#lib/Icon.svelte';
  import PendingPanel from '#lib/PendingPanel.svelte';
  import WorkItemDetail from '#lib/WorkItemDetail.svelte';
  import { onMount, tick } from 'svelte';
  import { goto, invalidateAll } from '$app/navigation';
  import { page } from '$app/state';
  import { Work, WorkApi, WorkError, type WorkItem } from '#lib/work.ts';
  import { ago } from '#lib/overview.ts';
  import { harnessLabel } from '#lib/activity.ts';
  import { Sessions } from '#lib/runs.ts';
  import { ProjectBoard } from '#lib/projects.ts';
  import { ReleaseLine } from '#lib/release.ts';
  import { PullRequests } from '#lib/pulls.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const project = $derived(data.project);
  const board = $derived(ProjectBoard.of(project, data.work, data.live, data.sessions));
  /** Its work not yet shipped or dismissed, by state. */
  const open = $derived(
    ProjectBoard.groups(data.work).filter(
      (group) => group.state !== 'shipped' && group.state !== 'dismissed',
    ),
  );
  /** Its sessions with a run under way. */
  const running = $derived(
    data.sessions
      .flatMap((body) => Sessions.desk(body, data.live) ?? [])
      .filter((session) => Sessions.running(session)),
  );
  const line = $derived(project.release ? ReleaseLine.of(project.release) : null);
  const pulls = $derived(data.pulls);
  /** The newest answer from GitHub, and whether it is current. */
  const seen = $derived(pulls ? PullRequests.sighting(pulls) : null);
  const base = $derived(ProjectBoard.href(project.name));
  const inspect = $derived(new URLSearchParams({ project: project.name }).toString());

  /** The work item open in the drawer, as the address names it. */
  const shown = $derived(
    data.work.find((item) => item.id === page.url.searchParams.get('item')) ?? null,
  );
  let drawer = $state<HTMLElement>();
  let failed = $state<string | null>(null);
  /** The open item's id: a refresh rebuilds the item, not which one is open. */
  const openId = $derived(shown?.id ?? null);
  $effect(() => {
    // Opening an item moves focus into the drawer, once per item.
    if (openId) void tick().then(() => drawer?.focus());
  });
  /** Whether this page opened the drawer, so closing it steps back. */
  let opened = false;
  const close = async () => {
    if (opened) {
      opened = false;
      history.back();
    } else {
      // Arrived at with the item in the address: close in place.
      await goto(page.url.pathname, { replace: true, reset: false });
    }
  };
  const work = new WorkApi(fetch);

  async function dismiss(item: WorkItem) {
    failed = null;
    try {
      await work.move(item.id, { kind: 'dismiss' });
      await invalidateAll();
    } catch (failure) {
      failed = failure instanceof WorkError ? failure.message : 'The work item was not dismissed.';
    }
  }

  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => (now = Date.now()), 15_000);
    return () => clearInterval(timer);
  });
</script>

{#snippet scope()}
  <div class="scope-group">
    <span class="muted mono path">{project.repository}</span>
    {#if project.archived_at !== null}<span class="tag archived">archived</span>{/if}
  </div>
{/snippet}

<svelte:window
  onkeydown={(event) => {
    if (shown && event.key === 'Escape') void close();
  }}
/>

<Toolbar {scope} />

<div class="frame">
  <section class="overview" aria-label={project.name}>
    <div class="main">
      <h2 class="with-links">
        Release <a href={`${base}/release`}>The full release</a>
      </h2>
      {#if line}
        <p class="line tone-{line.tone}">{line.text}</p>
      {:else}
        <p class="empty">
          No canary has run yet: <code
            >{project.factory === 'ren'
              ? 'cargo xtask canary run'
              : `ren project canary --project ${project.name}`}</code
          >
        </p>
      {/if}

      <h2 class="with-links">
        Pull requests
        {#if pulls && pulls.watch !== 'off' && pulls.watch !== 'unknown'}
          <a href={`https://github.com/${pulls.repository}/pulls`} target="_blank" rel="noreferrer"
            >{pulls.repository} on GitHub</a
          >
        {/if}
      </h2>
      {#if pulls === null}
        <p class="empty">Its pull requests could not be read.</p>
      {:else if pulls.watch === 'off'}
        <p class="empty">
          Not watched on GitHub: name its repository in the daemon's <code>[github]</code> configuration.
        </p>
      {:else if pulls.watch === 'waiting'}
        <p class="empty">Waiting for GitHub's first answer about {pulls.repository}.</p>
      {:else if pulls.watch === 'unknown'}
        <p class="empty">The daemon reports its pull requests as “{pulls.text}”.</p>
      {:else if pulls.watch === 'failed'}
        <p class="line tone-bad" role="status">
          GitHub's newest poll failed {ago(pulls.at, now)}: {pulls.error}{#if seen}. Shown as it was {ago(
              seen.sighting.at,
              now,
            )}.{/if}
        </p>
      {/if}
      {#if seen}
        {#if !seen.sighting.pull_requests.length}
          <p class="empty">No pull request is open.</p>
        {/if}
        <ul class="rows pulls" class:stale={!seen.current}>
          {#each seen.sighting.pull_requests as pull (pull.number)}
            {@const [checks, checksTone] = PullRequests.checks(pull.checks)}
            {@const notes = [PullRequests.merge(pull), PullRequests.review(pull.review)].filter(
              (note) => note !== null,
            )}
            <li>
              <span
                class="tone-{checksTone}"
                title={pull.check_list
                  .map((check) => `${check.name}: ${PullRequests.checkState(check.state)[0]}`)
                  .join('\n')}>{checks}</span
              >
              <a href={pull.url} target="_blank" rel="noreferrer"
                ><span class="muted mono">#{pull.number}</span> {pull.title}</a
              >
              <span class="notes">
                {#each notes as [note, tone] (note)}
                  <span class="note tone-{tone}">{note}</span>
                {/each}
                {#each pull.work_items as id (id)}
                  <a
                    class="note"
                    href={`?item=${id}`}
                    data-sveltekit-reset="false"
                    onclick={() => (opened = true)}
                    >{data.work.find((item) => item.id === id)?.title ?? 'A work item'}</a
                  >
                {/each}
              </span>
              <span class="muted mono when" title={pull.author ? `by ${pull.author}` : undefined}
                >{pull.head_branch}</span
              >
            </li>
          {/each}
        </ul>
        {#if seen.sighting.truncated}
          <p class="empty">Only the most recently updated are listed.</p>
        {/if}
      {/if}

      <h2>Work under way</h2>
      {#if !open.length}
        <p class="empty">Nothing open. Redline marks work on any of {project.name}'s pages.</p>
      {/if}
      <ul class="rows">
        {#each open as group (group.state)}
          {#each group.items as item (item.id)}
            <li>
              <span class="work-state state-{item.state}">{Work.state(item.state)}</span>
              <a
                href={`?item=${item.id}`}
                data-sveltekit-reset="false"
                onclick={() => (opened = true)}
                aria-current={shown?.id === item.id ? 'true' : undefined}>{item.title}</a
              >
              <span class="muted mono">{item.order.page.path}</span>
              <span class="muted when">{ago(item.updated_at, now)}</span>
            </li>
          {/each}
        {/each}
      </ul>

      <h2 class="with-links">
        Sessions running <a href={`${base}/sessions`}>All sessions</a>
      </h2>
      {#if !page.data.runsControl}
        <p class="empty">This UI only reads: give it the control token to see sessions.</p>
      {:else if !running.length}
        <p class="empty">No session is running for {project.name}.</p>
      {/if}
      <ul class="rows">
        {#each running as session (session.session)}
          <li>
            <span class="live"><span class="dot"></span></span>
            <a href={Sessions.deskHref(project.name, { session: session.session })}
              >{session.asked ?? 'Not asked anything yet'}</a
            >
            <span class="muted"
              >{harnessLabel(session.harness)}{session.worktree ? ' in ' : ''}<span class="mono"
                >{session.worktree?.name ?? ''}</span
              ></span
            >
            <span class="muted mono when"
              >{#if session.worktree}ports {session.worktree.ports[0]}–{session.worktree
                  .ports[1]}{/if}</span
            >
          </li>
        {/each}
      </ul>

      <h2>Inspect</h2>
      <p class="inspect">
        <a href={`/traffic?${inspect}`}>Its calls</a>
        <a href={`/recordings?${inspect}`}>Its recordings</a>
        <a href={`/live?${inspect}`}>Live</a>
        <a href={`/overview?${inspect}`}>Traffic overview</a>
      </p>
    </div>
    <aside class="pending" aria-label="Pending"><PendingPanel {board} /></aside>
  </section>
  {#if shown}
    <button
      class="scrim"
      aria-label="Close the work item"
      tabindex="-1"
      onclick={() => void close()}
    ></button>
    <div
      class="drawer"
      role="dialog"
      aria-modal="true"
      aria-label={shown.title}
      tabindex="-1"
      bind:this={drawer}
    >
      <div class="drawer-bar">
        <span class="muted">{project.name}</span>
        <button class="button small" aria-label="Close" onclick={() => void close()}
          ><Icon name="close" size={12} /></button
        >
      </div>
      {#if failed}<p class="error" role="alert">{failed}</p>{/if}
      <div class="drawer-body">
        <WorkItemDetail
          item={shown}
          project={project.factory === 'ren'
            ? { kind: 'ren' }
            : { kind: 'other', name: project.name }}
          ondismiss={(item) => void dismiss(item)}
        />
      </div>
    </div>
  {/if}
</div>

<style>
  .frame {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .overview {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 1px;
    background: var(--line);
    overflow: auto;
  }
  .main {
    flex: 999 1 560px;
    min-width: 0;
    padding: 0 0 24px;
    background: var(--surface);
  }
  .pending {
    flex: 1 1 300px;
    background: var(--surface);
  }
  .scrim {
    position: absolute;
    inset: 0;
    border: 0;
    padding: 0;
    background: rgb(16 18 27 / 0.28);
    cursor: default;
  }
  .drawer {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(560px, 100%);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border-left: 1px solid var(--popover-border);
    box-shadow: var(--popover-shadow);
    outline: none;
  }
  .drawer-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--line);
    background: var(--surface-subtle);
  }
  .drawer-bar .muted {
    margin-right: auto;
    font-size: 12px;
  }
  .drawer-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 12px 16px;
  }
  .error {
    margin: 0;
    padding: 8px 16px;
    color: var(--error);
  }
  .path {
    font-size: 11px;
  }
  .tag {
    padding: 0 5px;
    border: 1px solid var(--accent-border);
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: 11px;
  }
  .tag.archived {
    border-color: var(--line);
    background: var(--surface-subtle);
    color: var(--muted);
  }
  h2 {
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin: 0;
    padding: 16px 16px 8px;
    border-bottom: 1px solid var(--line);
  }
  .with-links {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }
  .with-links a {
    margin-left: auto;
    text-transform: none;
    letter-spacing: 0;
    font-weight: 500;
    font-size: 12px;
  }
  .line {
    margin: 0;
    padding: 12px 16px;
    font-weight: 500;
  }
  .tone-ok {
    color: var(--success);
  }
  .tone-warning {
    color: var(--warning);
  }
  .tone-bad {
    color: var(--error);
  }
  .tone-quiet {
    color: var(--muted);
  }
  .notes {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
  }
  .note + .note::before {
    content: '·';
    margin: 0 6px;
    color: var(--muted);
  }
  .pulls.stale {
    opacity: 0.6;
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .rows li {
    display: grid;
    grid-template-columns: 110px minmax(0, 1fr) minmax(0, 200px) 150px;
    gap: 12px;
    align-items: baseline;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line-subtle);
  }
  .rows li > a {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rows a[aria-current] {
    font-weight: 600;
  }
  .when {
    text-align: right;
    font-size: 12px;
    white-space: nowrap;
  }
  .live {
    display: inline-flex;
    align-items: center;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--live);
  }
  .inspect {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    margin: 0;
    padding: 12px 16px;
  }
  .empty {
    padding: 12px 16px;
    margin: 0;
    color: var(--muted);
  }
  .muted {
    color: var(--muted);
  }
  @media (max-width: 640px) {
    .rows li {
      grid-template-columns: minmax(0, 1fr) auto;
    }
    .rows li > .mono:not(.when) {
      display: none;
    }
  }
</style>
