<!--
  Starting a run: the prompt, where the run works, with which client, and on
  which model. The folder is always chosen, never assumed: its picker lists
  the folders recent runs worked in first, and Enter without one opens it.
  The client is the last one used until changed. Its models are the ones the
  client itself lists on the daemon's host; with none chosen, the client's
  own default, and the last model chosen for a client is kept while listed.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { harnessLabel } from '#lib/activity.ts';
  import Picker, { type PickerGroup } from '#lib/Picker.svelte';
  import Icon from '#lib/Icon.svelte';
  import {
    DRIVABLE,
    Desk,
    APPROVAL_POLICIES,
    EFFORTS,
    Memory,
    Models,
    RunCommandError,
    Setup,
    Runs,
    type DeskRun,
    type Drivable,
    type ApprovalPolicy,
    type Effort,
    type ModelId,
    type RunClient,
    type RunModel,
    type RunProject,
    type Remembered,
  } from '#lib/runs.ts';

  let {
    offered,
    scratch,
    remembered,
    clients,
    runs,
    busy,
    listModels,
    onstart,
  }: {
    offered: readonly RunProject[];
    /** The folder scratch runs get theirs in; null: no scratch here. */
    scratch: string | null;
    /** The remembered-settings cookie, as the server read it. */
    remembered: string | null;
    clients: readonly RunClient[];
    runs: readonly DeskRun[];
    busy: boolean;
    listModels: (harness: Drivable) => Promise<RunModel[]>;
    onstart: (
      harness: Drivable,
      /** Null: a new scratch folder. */
      dir: string | null,
      prompt: string | null,
      settings: { model: ModelId | null; effort: Effort | null; approvals: ApprovalPolicy | null },
    ) => void;
  } = $props();

  /** The client the last run started with, and each client's last
   * settings: kept in a cookie, so the server renders them too. */
  const saved: Remembered = untrack(() => Memory.parse(remembered));

  /** A client's models, as far as they are known. */
  type Listing =
    | { state: 'loading' }
    | { state: 'listed'; models: RunModel[] }
    | { state: 'failed'; reason: string };

  let prompt = $state('');
  /** Where the run works: a folder chosen each time, or, with none, a new
   * scratch folder of its own where the daemon offers scratch. */
  let dir = $state<string | null>(null);
  /** Whether the folder list shows the other places (`~/.config`). */
  let others = $state(false);
  let harness = $state<Drivable>(saved.harness);
  let listings = $state<Partial<Record<Drivable, Listing>>>({});
  /** What the client last started with, until its models are listed. */
  const kept = untrack(() => saved.settings[saved.harness] ?? {});
  /** The chosen effort and approval policy; none is the client's own. */
  let effort = $state<Effort | null>(EFFORTS.find((level) => level === kept.effort) ?? null);
  let approvals = $state<ApprovalPolicy | null>(
    APPROVAL_POLICIES.find((policy) => policy === kept.approvals) ?? null,
  );
  /** The chosen model; none is the client's own default. Until the client
   * lists its models, the one last chosen, shown by its kept name. */
  let model = $state<ModelId | null>((kept.model as ModelId | undefined) ?? null);
  let field = $state<HTMLTextAreaElement>();
  let folders = $state<ReturnType<typeof Picker>>();

  /** The picker's value for scratch: never a folder's path, which is never blank. */
  const SCRATCH = '';
  const shownPlaces = $derived(
    offered.filter((place) => place.kind === 'project' || (others && place.kind === 'other')),
  );
  const hasOthers = $derived(offered.some((place) => place.kind === 'other'));
  const recent = $derived(Desk.recent(runs, shownPlaces));
  const placeOption = (project: RunProject) => ({
    value: project.path,
    label: project.name,
    hint: Runs.branch(project.git),
    hintIcon: Runs.versioned(project.git) ? ('branch' as const) : undefined,
  });
  const projectGroups = $derived<PickerGroup[]>([
    ...(scratch
      ? [
          {
            label: null,
            options: [
              {
                value: SCRATCH,
                label: 'Scratch',
                detail: 'A new folder of its own, kept with the run',
              },
            ],
          },
        ]
      : []),
    ...(recent.length
      ? [
          {
            label: 'Recent',
            options: recent.map(placeOption),
          },
        ]
      : []),
    ...Runs.places(offered.filter((place) => place.kind === 'project')).map((place) => ({
      label: place.parent,
      options: place.projects.map(placeOption),
    })),
    ...(others
      ? Runs.places(offered.filter((place) => place.kind === 'other')).map((place) => ({
          label: place.parent,
          options: place.projects.map(placeOption),
        }))
      : []),
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
        // Until the client lists its models, the one chosen, as kept.
        ...(listing.state !== 'listed' && model
          ? [{ value: model, label: saved.settings[harness]?.modelName ?? model }]
          : []),
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
        if (asked === harness) restore(asked, models);
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

  /** The approval policies the chosen client runs under; none for Pi. */
  const policies = $derived<ApprovalPolicy[]>(
    APPROVAL_POLICIES.filter((policy) =>
      (
        clients.find((client) => client.harness === harness)?.approvals ??
        (harness === 'pi' ? [] : APPROVAL_POLICIES)
      ).includes(policy),
    ),
  );
  /** The model the run gets, chosen or the default, and the efforts it takes. */
  const effective = $derived(Setup.efforts(listed, model));
  /** The efforts offered: the model's, once listed; until then the one
   * chosen, so the chip reads as it will stay. */
  const efforts = $derived<Effort[]>(
    listing.state === 'listed'
      ? EFFORTS.filter((level) => effective?.efforts.includes(level))
      : effort
        ? [effort]
        : [],
  );
  const effortGroups = $derived<PickerGroup[]>([
    {
      label: null,
      options: [
        {
          value: DEFAULT,
          label: 'Default',
          detail: effective ? `${effective.name}’s own` : `${harnessLabel(harness)}’s own`,
          hint: effective?.effort ?? undefined,
        },
        ...efforts.map((level) => ({
          value: level,
          label: level === 'xhigh' ? 'Extra high' : level[0]!.toUpperCase() + level.slice(1),
          badge: level === effective?.effort ? 'default' : null,
        })),
      ],
    },
  ]);
  const policyGroups = $derived<PickerGroup[]>([
    {
      label: null,
      options: [
        {
          value: DEFAULT,
          label: 'Default',
          detail: `As ${harnessLabel(harness)} is set up on the daemon’s host`,
        },
        ...policies.map((policy) => ({
          value: policy,
          label: Setup.policy(policy),
          detail: Setup.explain(policy),
        })),
      ],
    },
  ]);

  /** The settings last started with on `drivable`, where they still apply. */
  function restore(drivable: Drivable, models: readonly RunModel[] | null) {
    const kept = saved.settings[drivable] ?? {};
    approvals = APPROVAL_POLICIES.find((policy) => policy === kept.approvals) ?? null;
    if (!models) {
      model = null;
      effort = null;
      return;
    }
    model = Models.keep(models, kept.model ?? null);
    const takes = Setup.efforts(models, model)?.efforts ?? [];
    effort = EFFORTS.find((level) => level === kept.effort && takes.includes(level)) ?? null;
  }

  const chosenProject = $derived(offered.find((project) => project.path === dir));
  /** Whether the run has somewhere to work: a folder, or scratch. */
  const placed = $derived(dir !== null || scratch !== null);
  const ready = $derived(!busy);

  export function focus() {
    field?.focus();
  }

  function start() {
    if (busy) return;
    if (!placed) {
      folders?.open();
      return;
    }
    saved.settings[harness] = {
      model: model ?? undefined,
      modelName: listed.find((offered) => offered.id === model)?.name,
      effort: effort ?? undefined,
      approvals: approvals ?? undefined,
    };
    saved.harness = harness;
    // Remembering is a convenience: a refused cookie changes nothing else.
    document.cookie = Memory.cookie(saved);
    onstart(harness, dir, prompt.trim() || null, { model, effort, approvals });
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
        : dir === null && scratch
          ? `Ask ${harnessLabel(harness)} anything. It works in a scratch folder of its own; choose a folder below to work in a project.`
          : 'Describe the task, and choose the folder it works in below.'}
      aria-label="First prompt"></textarea>
    <div class="row">
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
          // Another client's settings are its own.
          const known = listings[next];
          restore(next, known?.state === 'listed' ? known.models : null);
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
        onchange={(value) => {
          model = Models.keep(listed, value);
          // An effort the new model does not take goes back to its own.
          const takes = Setup.efforts(listed, model)?.efforts ?? [];
          if (effort && !takes.includes(effort)) effort = null;
        }}
      />
      <Picker
        label="Effort"
        icon="thought"
        groups={effortGroups}
        value={effort ?? DEFAULT}
        placeholder="Default"
        search={false}
        disabled={listing.state === 'listed' && efforts.length === 0
          ? 'This model takes no effort setting'
          : null}
        onchange={(value) => (effort = efforts.find((level) => level === value) ?? null)}
      />
      {#if policies.length}
        <Picker
          label="Approvals"
          icon="shield"
          groups={policyGroups}
          value={approvals ?? DEFAULT}
          placeholder="Default"
          search={false}
          onchange={(value) => (approvals = policies.find((policy) => policy === value) ?? null)}
        />
      {:else}
        <span class="never muted" title="{harnessLabel(harness)} runs every tool itself"
          ><Icon name="shield" size={12} />Never asks</span
        >
      {/if}
    </div>
    <div class="row">
      <Picker
        bind:this={folders}
        label="Folder"
        icon="file"
        groups={projectGroups}
        value={dir ?? (scratch ? SCRATCH : null)}
        placeholder="Choose a folder"
        free={(typed) => typed.startsWith('/')}
        toggle={hasOthers
          ? { label: 'Show other places', on: others, onchange: (on) => (others = on) }
          : null}
        onchange={(value) => (dir = value === SCRATCH ? null : value)}
      />
      <button class="button primary" disabled={!ready}
        >{!placed ? 'Choose a folder' : prompt.trim() ? 'Start' : 'Start without a prompt'}</button
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
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 8px;
    border-top: 1px solid var(--line);
  }
  .row .button {
    margin-left: auto;
  }
  .never {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
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
