<!--
  Starting a session: only where it works. Choosing a folder makes a draft
  session there at once, as a folder's `+` does, and opens it; its client,
  model, and settings are chosen in the draft, where the composer offers the
  client's commands and the folder's files.
-->
<script lang="ts">
  import type { UnixMillis } from '#lib/api.ts';
  import Picker, { type PickerGroup } from '#lib/Picker.svelte';
  import Icon from '#lib/Icon.svelte';
  import { Desk, Runs, type RunProject } from '#lib/runs.ts';

  let {
    offered,
    scratch,
    runs,
    busy,
    oncreate,
  }: {
    offered: readonly RunProject[];
    /** The folder scratch sessions get theirs in; null: no scratch here. */
    scratch: string | null;
    runs: readonly { workdir: string; started_at: UnixMillis | null }[];
    busy: boolean;
    /** Make a session in `dir`; null: a new scratch folder of its own. */
    oncreate: (dir: string | null) => void;
  } = $props();

  /** Whether the folder list shows the other places (`~/.config`). */
  let others = $state(false);
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
  const groups = $derived<PickerGroup[]>([
    ...(scratch
      ? [
          {
            label: null,
            options: [
              {
                value: SCRATCH,
                label: 'Scratch',
                detail: 'A new folder of its own, kept with the session',
              },
            ],
          },
        ]
      : []),
    ...(recent.length ? [{ label: 'Recent', options: recent.map(placeOption) }] : []),
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

  /** Open the folder list, for `n` and New session. */
  export function open() {
    folders?.open();
  }

  function create(dir: string | null) {
    if (!busy) oncreate(dir);
  }
</script>

<div class="new-session">
  <h2>Start a session</h2>
  <p class="muted">
    Choose the folder it works in. It opens as a draft: choose its client and settings there, and
    write its first prompt with the client’s <kbd>/</kbd> commands and the folder’s <kbd>@</kbd> files.
  </p>
  <div class="choose">
    <Picker
      bind:this={folders}
      label="Folder"
      icon="file"
      {groups}
      value={null}
      placeholder="Choose a folder"
      free={(typed) => typed.startsWith('/')}
      toggle={hasOthers
        ? { label: 'Show other places', on: others, onchange: (on) => (others = on) }
        : null}
      onchange={(value) => create(value === SCRATCH ? null : value)}
    />
  </div>
  {#if scratch || recent.length}
    <ul class="quick" aria-label="Start a session in">
      {#if scratch}
        <li>
          <button class="place" disabled={busy} onclick={() => create(null)}>
            <Icon name="plus" size={12} /><span>Scratch</span><span class="muted"
              >a folder of its own</span
            >
          </button>
        </li>
      {/if}
      {#each recent as project (project.path)}
        <li>
          <button class="place" disabled={busy} onclick={() => create(project.path)}>
            <Icon name="plus" size={12} /><span>{project.name}</span>
            {#if Runs.branch(project.git)}<span class="muted">{Runs.branch(project.git)}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .new-session {
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
  p {
    width: min(520px, 100%);
    margin: 0;
    font-size: 12px;
    text-align: center;
  }
  .quick {
    width: min(360px, 100%);
    margin: 0;
    padding: 0;
    list-style: none;
    border: 1px solid var(--line);
  }
  .quick li + li {
    border-top: 1px solid var(--line);
  }
  .place {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border: 0;
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }
  .place:hover:not(:disabled) {
    background: var(--surface-hover, var(--surface));
  }
  .place:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .place .muted {
    margin-left: auto;
    font-size: 12px;
  }
</style>
