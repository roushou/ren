<!--
  Starting a run: the prompt, where the run works, with which client, and on
  which model. The folder is always chosen, never assumed: its picker lists
  the folders recent runs worked in first, and Enter without one opens it.
  The client is the last one used until changed. Its models are the ones the
  client itself lists on the daemon's host; with none chosen, the client's
  own default, and the last model chosen for a client is kept while listed.
-->
<script lang="ts">
  import { harnessLabel } from '#lib/activity.ts';
  import Picker, { type PickerGroup } from '#lib/Picker.svelte';
  import {
    DRIVABLE,
    Desk,
    Models,
    RunCommandError,
    Runs,
    type DeskRun,
    type Drivable,
    type ModelId,
    type RunClient,
    type RunModel,
    type RunProject,
  } from '#lib/runs.ts';

  let {
    offered,
    clients,
    runs,
    busy,
    listModels,
    onstart,
  }: {
    offered: readonly RunProject[];
    clients: readonly RunClient[];
    runs: readonly DeskRun[];
    busy: boolean;
    listModels: (harness: Drivable) => Promise<RunModel[]>;
    onstart: (harness: Drivable, dir: string, prompt: string | null, model: ModelId | null) => void;
  } = $props();

  /** The client the last run started with, and the model last chosen for
   * each client: a per-viewer convenience. */
  type Saved = { harness: Drivable; models: Partial<Record<Drivable, string>> };
  const remembered = 'ren.runs.last';
  function recall(): Saved {
    try {
      const saved = JSON.parse(localStorage.getItem(remembered) ?? 'null') as {
        harness?: unknown;
        models?: unknown;
      } | null;
      const models: Partial<Record<Drivable, string>> = {};
      if (saved?.models && typeof saved.models === 'object') {
        for (const drivable of DRIVABLE) {
          const id = (saved.models as Record<string, unknown>)[drivable];
          if (typeof id === 'string') models[drivable] = id;
        }
      }
      return {
        harness: DRIVABLE.find((drivable) => drivable === saved?.harness) ?? 'codex',
        models,
      };
    } catch {
      return { harness: 'codex', models: {} };
    }
  }
  const saved = recall();

  /** A client's models, as far as they are known. */
  type Listing =
    | { state: 'loading' }
    | { state: 'listed'; models: RunModel[] }
    | { state: 'failed'; reason: string };

  let prompt = $state('');
  /** Where the run works: chosen each time, never assumed. */
  let dir = $state<string | null>(null);
  let harness = $state<Drivable>(saved.harness);
  let listings = $state<Partial<Record<Drivable, Listing>>>({});
  /** The chosen model; none is the client's own default. */
  let model = $state<ModelId | null>(null);
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
  const listing = $derived<Listing>(listings[harness] ?? { state: 'loading' });
  const listed = $derived(listing.state === 'listed' ? listing.models : []);
  const fallback = $derived(Models.fallback(listed));
  /** The picker's value for "no model chosen": never a model's id, which is never blank. */
  const DEFAULT = '';
  const modelGroups = $derived<PickerGroup[]>([
    {
      label: null,
      options: [
        {
          value: DEFAULT,
          label: 'Default',
          detail: `${harnessLabel(harness)}’s own choice`,
          // What the default is now, on the chip as in the list.
          hint: fallback?.name,
        },
      ],
    },
    ...Models.groups(listed).map((group) => ({
      label: group.provider ?? 'Models',
      options: group.models.map((offered) => ({
        value: offered.id,
        label: offered.name,
        detail: offered.description ?? (offered.name === offered.id ? null : offered.id),
        badge: offered.choice === 'default' ? 'default' : null,
      })),
    })),
  ]);
  const modelNote = $derived.by(() => {
    switch (listing.state) {
      case 'loading':
        return `Asking ${harnessLabel(harness)} for its models…`;
      case 'failed':
        return listing.reason;
      case 'listed':
        return listing.models.length
          ? null
          : `${harnessLabel(harness)} lists no models here. Is it signed in on the daemon’s host?`;
    }
  });

  /** Ask the chosen client for its models once; the daemon keeps them too. */
  $effect(() => {
    const asked = harness;
    if (listings[asked]) return;
    listings[asked] = { state: 'loading' };
    listModels(asked).then(
      (models) => {
        listings[asked] = { state: 'listed', models };
        if (asked === harness) model = Models.keep(models, saved.models[asked] ?? null);
      },
      (error: unknown) => {
        listings[asked] = {
          state: 'failed',
          reason:
            error instanceof RunCommandError
              ? error.message
              : `${harnessLabel(asked)} could not list its models.`,
        };
      },
    );
  });

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
    if (model) saved.models[harness] = model;
    else delete saved.models[harness];
    saved.harness = harness;
    try {
      localStorage.setItem(remembered, JSON.stringify(saved));
    } catch {
      // Remembering is a convenience.
    }
    onstart(harness, dir, prompt.trim() || null, model);
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
          const next = DRIVABLE.find((drivable) => drivable === value) ?? harness;
          if (next === harness) return;
          harness = next;
          // Another client's models are its own.
          const known = listings[next];
          model =
            known?.state === 'listed'
              ? Models.keep(known.models, saved.models[next] ?? null)
              : null;
        }}
      />
      <Picker
        label="Model"
        icon="cpu"
        groups={modelGroups}
        value={model ?? DEFAULT}
        placeholder="Default"
        search={listed.length > 8}
        note={modelNote}
        onchange={(value) => (model = Models.keep(listed, value))}
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
