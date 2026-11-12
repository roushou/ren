<!--
  The runs, grouped by the folder each works in: folders with a run that
  needs you first. Each run is named by what it was asked, when that is
  known here, and says how it stands and how long ago it started; ended
  runs fold under their folder.
-->
<script lang="ts">
  import { SvelteSet } from 'svelte/reactivity';
  import { harnessLabel } from '#lib/activity.ts';
  import Icon from '#lib/Icon.svelte';
  import { ago } from '#lib/overview.ts';
  import { Desk, Runs, type DeskRun, type Project, type RunId } from '#lib/runs.ts';

  let {
    projects,
    selected,
    now,
    title,
    onselect,
    onnew,
  }: {
    projects: readonly Project[];
    selected: RunId | null;
    now: number;
    /** What a run was asked, when known here. */
    title: (run: DeskRun) => string | null;
    onselect: (run: RunId) => void;
    onnew: () => void;
  } = $props();

  /** Folders whose ended runs are shown. */
  const unfolded = new SvelteSet<string>();
</script>

{#snippet row(run: DeskRun)}
  {@const mark = Runs.mark(Desk.state(run))}
  {@const asked = title(run)}
  <button
    class="run-row"
    class:selected={run.run === selected}
    class:needs={mark.tone === 'warn'}
    aria-current={run.run === selected ? 'true' : undefined}
    onclick={() => onselect(run.run)}
  >
    <span class="phase-dot tone-{mark.tone}"></span>
    <span class="run-name">{asked ?? `${harnessLabel(run.harness)} run`}</span>
    <span class="run-state tone-{mark.tone}">{mark.label}</span>
    <span class="run-when muted">{run.started_at ? ago(run.started_at, now) : ''}</span>
  </button>
{/snippet}

<nav class="run-list" aria-label="Runs by folder">
  <button class="new-run" onclick={onnew}>
    <Icon name="spark" size={13} /> New run <kbd>n</kbd>
  </button>
  {#each projects as project (project.workdir)}
    {@const { open, ended } = Desk.split(project)}
    <section class="folder">
      <header title={project.workdir}>
        <Icon name="file" size={12} />
        <strong>{project.name}</strong>
        {#if project.waiting}<span class="chip warning">{project.waiting} needs you</span>{/if}
        <span class="muted mono parent">{project.workdir.slice(0, -project.name.length)}</span>
      </header>
      {#each open as run (run.run)}{@render row(run)}{/each}
      {#if ended.length}
        {#if unfolded.has(project.workdir) || ended.some((run) => run.run === selected)}
          {#each ended as run (run.run)}{@render row(run)}{/each}
        {:else}
          <button class="fold" onclick={() => unfolded.add(project.workdir)}>
            <Icon name="chevron" size={10} />
            {ended.length} ended
          </button>
        {/if}
      {/if}
    </section>
  {:else}
    <p class="note">No runs yet. Start one with New run.</p>
  {/each}
</nav>

<style>
  .run-list {
    overflow: auto;
    min-height: 0;
  }
  .new-run {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 10px 12px;
    border: 0;
    border-bottom: 1px solid var(--line);
    background: none;
    color: var(--ink);
    font: inherit;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }
  .new-run:hover {
    background: var(--row-hover);
  }
  .new-run kbd {
    margin-left: auto;
    font-size: 11px;
    color: var(--muted);
  }
  .folder {
    border-bottom: 1px solid var(--line);
    padding-bottom: 4px;
  }
  .folder header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 10px 12px 4px;
    font-size: 12px;
    min-width: 0;
  }
  .folder header strong {
    white-space: nowrap;
  }
  .folder header .chip {
    white-space: nowrap;
  }
  .folder header .parent {
    flex: 1;
    min-width: 0;
    margin-left: auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
    direction: rtl;
    text-align: left;
  }
  .run-row {
    display: grid;
    grid-template-columns: 7px minmax(0, 1fr) auto;
    grid-template-rows: auto auto;
    column-gap: 8px;
    align-items: center;
    width: 100%;
    padding: 5px 12px 5px 18px;
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
  .run-row.needs .run-name {
    font-weight: 600;
  }
  .run-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .run-state {
    grid-column: 2;
    grid-row: 2;
    font-size: 11px;
    color: var(--muted);
  }
  .run-state.tone-warn {
    color: var(--warning);
  }
  .run-state.tone-live {
    color: var(--live);
  }
  .run-when {
    grid-column: 3;
    grid-row: 1 / span 2;
    font-size: 11px;
  }
  .fold {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 12px 4px 33px;
    border: 0;
    background: none;
    color: var(--muted);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }
  .fold:hover {
    color: var(--ink);
  }
  .note {
    margin: 0;
    padding: 12px;
    font-size: 12px;
    color: var(--muted);
  }
  .phase-dot {
    grid-row: 1 / span 2;
    width: 7px;
    height: 7px;
    border-radius: 50%;
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
</style>
