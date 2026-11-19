<!--
  A project (docs/plans/hierarchy.md): what it is, where its release
  stands, its work items by state, its sessions (each in the worktree or
  folder it works in, with the work it took up), and their runs. Each item
  opens in a drawer, each session and run in Sessions, and the release in
  full in Release.
-->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import Icon from '#lib/Icon.svelte';
  import WorkItemDetail from '#lib/WorkItemDetail.svelte';
  import { onMount, tick } from 'svelte';
  import { goto, invalidateAll } from '$app/navigation';
  import { page } from '$app/state';
  import { Work, WorkApi, WorkError, type WorkItem } from '#lib/work.ts';
  import { ago } from '#lib/overview.ts';
  import { harnessLabel } from '#lib/activity.ts';
  import { Sessions } from '#lib/runs.ts';
  import { ProjectBoard } from '#lib/projects.ts';
  import { ReleaseLine, ReleasePage } from '#lib/release.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const project = $derived(data.project);
  const groups = $derived(ProjectBoard.groups(data.work));
  const view = $derived(project.release ? new ReleasePage(project.release, null) : null);

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
    <a class="muted crumb" href="/">Projects</a>
    <span class="muted">/</span>
    <strong>{project.name}</strong>
    {#if project.factory === 'ren'}<span class="tag">Ren itself</span>{/if}
    {#if project.archived_at !== null}<span class="tag archived">archived</span>{/if}
  </div>
  <div class="scope-group">
    <span class="muted mono path">{project.repository}</span>
  </div>
{/snippet}

<svelte:window
  onkeydown={(event) => {
    if (shown && event.key === 'Escape') void close();
  }}
/>

<Toolbar {scope} />

<div class="frame">
  <section class="project" aria-label={project.name}>
    <h2 id="release">Release</h2>
    {#if view}
      {@const release = view.release}
      {@const [decision, tone] = ReleaseLine.decision(release)}
      <div class="lanes" aria-label="Builds">
        <div class="lane">
          <span class="lane-label">Main</span>
          <span class="lane-commit mono">{ReleasePage.short(release.main)}</span>
          <span class="lane-note">its tip, as the canary last fetched it</span>
        </div>
        <div class="lane">
          <span class="lane-label">Canary</span>
          <span class="lane-commit mono">{ReleasePage.short(release.canary?.commit ?? null)}</span>
          <span
            class="lane-note"
            class:ok={release.canary?.passed}
            class:bad={release.canary && !release.canary.passed}
            >{release.canary ? (release.canary.passed ? 'passed' : 'failed') : 'none checked'}</span
          >
        </div>
        <div class="lane">
          <span class="lane-label">Production</span>
          <span class="lane-commit mono">{ReleasePage.short(view.production)}</span>
          <span class="lane-note">{view.behind}</span>
        </div>
        <div class="lane">
          <span class="lane-label">Promotion · {release.policy}</span>
          <span class="tone-{tone}">{decision}</span>
          <a class="lane-note" href={ProjectBoard.releaseHref(project)}>The full release</a>
        </div>
      </div>
      {#if release.canary}
        <ul class="checks" aria-label="Canary checks">
          {#each release.canary.checks as check, index (index)}
            <li class:failed={!check.passed}>
              <Icon name={check.passed ? 'check' : 'alert'} size={13} />
              <span class="check-name">{check.name ?? check.check}</span>
              {#if check.detail}<span class="check-detail mono">{check.detail}</span>{/if}
            </li>
          {/each}
        </ul>
      {/if}
    {:else}
      <p class="empty">
        No canary has run yet: <code
          >{project.factory === 'ren'
            ? 'cargo xtask canary run'
            : `ren project canary --project ${project.name}`}</code
        >
      </p>
    {/if}

    <h2>Work</h2>
    {#if !groups.length}
      <p class="empty">No work items for {project.name} yet. Redline marks them.</p>
    {/if}
    {#each groups as group (group.state)}
      <div class="group state-{group.state}">
        <span class="dot"></span><strong>{group.label}</strong>
        <span class="muted">{group.items.length}</span>
        <span class="muted note">{group.note}</span>
      </div>
      <ul class="rows">
        {#each group.items as item (item.id)}
          <li>
            <a
              href={`?item=${item.id}`}
              data-sveltekit-reset="false"
              onclick={() => (opened = true)}
              aria-current={shown?.id === item.id ? 'true' : undefined}>{item.title}</a
            >
            <span class="muted mono">{item.order.page.path}</span>
            <span class="muted">
              {#if item.preview}<a href={item.preview.ui} target="_blank" rel="noopener"
                  >preview up</a
                >{:else if item.landed}{ReleasePage.short(item.landed)}{:else if item.session}<a
                  href={Sessions.deskHref(project.name, { session: item.session })}>its session</a
                >{/if}
            </span>
            <span class="muted when">{ago(item.updated_at, now)}</span>
          </li>
        {/each}
      </ul>
    {/each}

    <h2>Sessions</h2>
    {#if !data.sessions.length}
      <p class="empty">No session works for {project.name} now.</p>
    {:else}
      <ul class="rows sessions">
        {#each data.sessions as session (session.session)}
          {@const taken = data.work.filter((item) => item.session === session.session)}
          <li>
            <span class="session">
              <a href={Sessions.deskHref(project.name, { session: session.session })}
                >{session.asked ?? 'Not asked anything yet'}</a
              >
              <span class="muted where">
                {harnessLabel(session.harness)} in
                {#if session.worktree}worktree <span class="mono">{session.worktree.name}</span
                  >{:else}<span class="mono">{session.workdir}</span>{/if}
              </span>
              {#each taken as item (item.id)}
                <span class="taken">
                  <span class="work-state state-{item.state}">{Work.state(item.state)}</span>
                  <a
                    href={`?item=${item.id}`}
                    data-sveltekit-reset="false"
                    onclick={() => (opened = true)}>{item.title}</a
                  >
                  {#if item.preview}<a
                      class="muted"
                      href={item.preview.ui}
                      target="_blank"
                      rel="noopener">preview up</a
                    >{/if}
                </span>
              {/each}
            </span>
            <span class="muted mono">
              {#if session.worktree}<span title={`Data in ${session.worktree.data}`}
                  >ports {session.worktree.ports[0]}–{session.worktree.ports[1]}</span
                >{/if}
            </span>
            <span class="muted">
              {#if Sessions.running(session)}<span class="live"
                  ><span class="dot"></span>running</span
                >{:else}idle{/if}
            </span>
          </li>
        {/each}
      </ul>
    {/if}

    <h2 class="with-links">
      Runs
      <span class="inspect">
        <a href={`/traffic?${new URLSearchParams({ project: project.name })}`}>Their calls</a>
        <a href={`/recordings?${new URLSearchParams({ project: project.name })}`}
          >Their recordings</a
        >
      </span>
    </h2>
    {#if !data.runs.length}
      <p class="empty">No session has run for {project.name} yet.</p>
    {:else}
      <ul class="rows">
        {#each data.runs as run (run.id)}
          <li>
            <a href={Sessions.deskHref(project.name, { run: run.id })}
              >{harnessLabel(run.harness)} in <span class="mono">{run.workdir}</span></a
            >
            <span class="muted">
              {#if run.ended_at === null}<span class="live"><span class="dot"></span>running</span
                >{:else}ended{/if}
            </span>
            <span class="muted when">{ago(run.started_at, now)}</span>
          </li>
        {/each}
      </ul>
    {/if}
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
        <a class="button small" href={Work.href(project.name, { item: shown.id })}>Open in Work</a>
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
  .rows a[aria-current] {
    font-weight: 600;
  }
  .project {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 0 0 24px;
    background: var(--surface);
  }
  .crumb {
    color: var(--muted);
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
  .inspect {
    display: flex;
    gap: 12px;
    margin-left: auto;
    text-transform: none;
    letter-spacing: 0;
    font-weight: 500;
    font-size: 12px;
  }
  .lanes {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    border-bottom: 1px solid var(--line);
  }
  .lane {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 12px 16px;
    min-width: 0;
    border-right: 1px solid var(--line);
  }
  .lane-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .lane-commit {
    font-size: 16px;
  }
  .lane-note {
    color: var(--muted);
    font-size: 12px;
  }
  a.lane-note {
    color: var(--accent-text);
  }
  .lane-note.ok,
  .tone-ok {
    color: var(--success);
  }
  .lane-note.bad,
  .tone-bad {
    color: var(--error);
  }
  .tone-warning {
    color: var(--warning);
  }
  .checks {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .checks li {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 6px 16px;
    border-bottom: 1px solid var(--line-subtle);
    color: var(--success);
  }
  .checks li.failed {
    color: var(--error);
  }
  .check-name {
    color: var(--ink);
    font-weight: 500;
  }
  .check-detail {
    color: var(--muted);
    font-size: 11px;
    overflow-wrap: anywhere;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    background: var(--surface-subtle);
    border-bottom: 1px solid var(--line);
  }
  .group .note {
    margin-left: auto;
    font-size: 12px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--muted);
    flex: none;
  }
  .state-sent .dot,
  .live .dot {
    background: var(--live);
  }
  .state-landed .dot {
    background: var(--warning);
  }
  .state-shipped .dot {
    background: var(--success);
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .rows li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 220px) 120px 90px;
    gap: 12px;
    align-items: baseline;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line-subtle);
  }
  .rows li > :first-child {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rows.sessions li {
    grid-template-columns: minmax(0, 1fr) 140px 90px;
  }
  .rows.sessions li > :first-child {
    white-space: normal;
  }
  .session {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .where {
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .taken {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 12px;
  }
  .when {
    text-align: right;
    font-size: 12px;
  }
  .live {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .empty {
    padding: 12px 16px;
    margin: 0;
    color: var(--muted);
  }
  @media (max-width: 640px) {
    .rows li {
      grid-template-columns: minmax(0, 1fr) auto;
    }
    .rows li > :nth-child(2) {
      display: none;
    }
  }
</style>
