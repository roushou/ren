<!--
  Starting a run: the prompt, where the run works, and with which client.
  The folder is always chosen, never assumed: its picker lists the folders
  recent runs worked in first, and Enter without one opens it. The client is
  the last one used until changed.
-->
<script lang="ts">
  import { harnessLabel } from '#lib/activity.ts';
  import Picker, { type PickerGroup } from '#lib/Picker.svelte';
  import {
    DRIVABLE,
    Desk,
    Runs,
    type DeskRun,
    type Drivable,
    type RunClient,
    type RunProject,
  } from '#lib/runs.ts';

  let {
    offered,
    clients,
    runs,
    busy,
    onstart,
  }: {
    offered: readonly RunProject[];
    clients: readonly RunClient[];
    runs: readonly DeskRun[];
    busy: boolean;
    onstart: (harness: Drivable, dir: string, prompt: string | null) => void;
  } = $props();

  /** The client the last run started with: a per-viewer convenience. */
  const remembered = 'ren.runs.last';
  function recall(): Drivable {
    try {
      const saved = JSON.parse(localStorage.getItem(remembered) ?? 'null') as {
        harness?: unknown;
      } | null;
      return DRIVABLE.find((drivable) => drivable === saved?.harness) ?? 'codex';
    } catch {
      return 'codex';
    }
  }

  let prompt = $state('');
  /** Where the run works: chosen each time, never assumed. */
  let dir = $state<string | null>(null);
  let harness = $state<Drivable>(recall());
  let field = $state<HTMLTextAreaElement>();
  let folders = $state<ReturnType<typeof Picker>>();

  const recent = $derived(Desk.recent(runs, offered));
  const projectGroups = $derived<PickerGroup[]>([
    ...(recent.length
      ? [
          {
            label: 'Recent',
            options: recent.map((project) => ({
              value: project.path,
              label: project.name,
              hint: Runs.branch(project.git),
              hintIcon: Runs.versioned(project.git) ? ('branch' as const) : undefined,
            })),
          },
        ]
      : []),
    ...Runs.places(offered).map((place) => ({
      label: place.parent,
      options: place.projects.map((project) => ({
        value: project.path,
        label: project.name,
        hint: Runs.branch(project.git),
        hintIcon: Runs.versioned(project.git) ? ('branch' as const) : undefined,
      })),
    })),
  ]);
  const clientGroups = $derived<PickerGroup[]>([
    {
      label: null,
      options: (clients.length
        ? clients
        : DRIVABLE.map((harness) => ({ harness, status: 'ready' as const }))
      ).map((client) => ({
        value: client.harness,
        label: harnessLabel(client.harness),
        unavailable: Runs.unavailable(client.status),
      })),
    },
  ]);
  const chosenProject = $derived(offered.find((project) => project.path === dir));
  const ready = $derived(!busy);

  export function focus() {
    field?.focus();
  }

  function start() {
    if (busy) return;
    if (!dir) {
      folders?.open();
      return;
    }
    try {
      localStorage.setItem(remembered, JSON.stringify({ harness }));
    } catch {
      // Remembering is a convenience.
    }
    onstart(harness, dir, prompt.trim() || null);
    prompt = '';
  }

  function key(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      start();
    }
  }
</script>

<div class="new-run">
  <h2>What should an agent work on?</h2>
  <form
    class="composer"
    onsubmit={(event) => {
      event.preventDefault();
      start();
    }}
  >
    <textarea
      bind:this={field}
      bind:value={prompt}
      onkeydown={key}
      rows="4"
      placeholder={chosenProject
        ? `Ask ${harnessLabel(harness)} to do something in ${chosenProject.name}. Enter starts, Shift+Enter adds a line.`
        : 'Describe the task, and choose the folder it works in below.'}
      aria-label="First prompt"></textarea>
    <div class="row">
      <Picker
        bind:this={folders}
        label="Folder"
        icon="file"
        groups={projectGroups}
        value={dir}
        placeholder="Choose a folder"
        free={(typed) => typed.startsWith('/')}
        onchange={(value) => (dir = value)}
      />
      <Picker
        label="Client"
        icon="terminal"
        groups={clientGroups}
        value={harness}
        placeholder="Client"
        search={false}
        onchange={(value) => {
          harness = DRIVABLE.find((drivable) => drivable === value) ?? harness;
        }}
      />
      <button class="button primary" disabled={!ready}
        >{!dir ? 'Choose a folder' : prompt.trim() ? 'Start' : 'Start without a prompt'}</button
      >
    </div>
  </form>
  {#if recent.length}
    <p class="recent muted">
      Recent:
      {#each recent as project, index (project.path)}
        {#if index}<span class="dot" aria-hidden="true">·</span>{/if}<button
          class="text-button"
          onclick={() => {
            dir = project.path;
            field?.focus();
          }}>{project.name}</button
        >
      {/each}
    </p>
  {/if}
  <p class="about muted">
    A run is Codex, Claude Code, or Pi working in one of your folders, driven from here. Each keeps
    its own sign-in; its model traffic is recorded like any session’s.
  </p>
</div>

<style>
  .new-run {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    gap: 12px;
    padding: 24px;
    min-height: 0;
  }
  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }
  .composer {
    width: min(720px, 100%);
    display: flex;
    flex-direction: column;
    border: 1px solid var(--control-border);
    background: var(--surface);
  }
  textarea {
    resize: vertical;
    min-height: 96px;
    max-height: 40vh;
    padding: 12px 14px;
    border: 0;
    background: transparent;
    color: var(--ink);
    font: inherit;
    font-size: 13px;
    outline: none;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px;
    border-top: 1px solid var(--line);
  }
  .row .button {
    margin-left: auto;
  }
  .dot {
    margin: 0 6px;
  }
  .recent,
  .about {
    width: min(720px, 100%);
    margin: 0;
    font-size: 12px;
  }
  .about {
    text-align: center;
  }
</style>
