<!--
  A draft's folder, as a chip in the composer's bar: Scratch (a new folder of
  its own), the folders worked in most recently, then each place's folders
  under the place they are in, and the other places (`~/.config`) on asking.
  A folder appears once: a recent one is not repeated under its place.
-->
<script lang="ts">
  import type { UnixMillis } from '#lib/api.ts';
  import Picker, { type PickerGroup, type PickerOption } from '#lib/Picker.svelte';
  import { Desk, Runs, Sessions, type RunProject } from '#lib/runs.ts';

  let {
    workdir,
    offered,
    scratch,
    runs,
    disabled,
    onchange,
  }: {
    /** The folder the draft is in now. */
    workdir: string;
    offered: readonly RunProject[];
    /** The folder scratch sessions get theirs in; null: no scratch here. */
    scratch: string | null;
    runs: readonly { workdir: string; started_at: UnixMillis | null }[];
    /** Why it cannot change now, or null. */
    disabled: string | null;
    /** Move the draft to `dir`; null: a new scratch folder of its own. */
    onchange: (dir: string | null) => void;
  } = $props();

  /** Whether the list shows the other places (`~/.config`). */
  let others = $state(false);
  let picker = $state<ReturnType<typeof Picker>>();

  /** The picker's value for scratch: never a folder's path, which is never blank. */
  const SCRATCH = '';
  const inScratch = $derived(Sessions.inScratch(workdir, scratch));
  const value = $derived(inScratch ? SCRATCH : workdir);
  const shownPlaces = $derived(
    offered.filter((place) => place.kind === 'project' || (others && place.kind === 'other')),
  );
  const hasOthers = $derived(offered.some((place) => place.kind === 'other'));
  const recent = $derived(Desk.recent(runs, shownPlaces));
  const option = (project: RunProject): PickerOption => ({
    value: project.path,
    label: project.name,
    hint: Runs.branch(project.git),
    hintIcon: Runs.versioned(project.git) ? 'branch' : undefined,
  });
  /** A place's folders, without those already listed as recent. */
  const placed = (kind: RunProject['kind']): PickerGroup[] =>
    Runs.places(offered.filter((place) => place.kind === kind))
      .map((place) => ({
        label: place.parent === '~' ? 'Home' : `${place.parent}/`,
        options: place.projects
          .filter((project) => !recent.some((one) => one.path === project.path))
          .map(option),
      }))
      .filter((group) => group.options.length > 0);
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
    ...(recent.length ? [{ label: 'Recent', options: recent.map(option) }] : []),
    ...placed('project'),
    ...(others ? placed('other') : []),
  ]);

  /** Open the folder list. */
  export function open() {
    picker?.open();
  }
</script>

<Picker
  bind:this={picker}
  label="Folder the session works in"
  icon="file"
  {groups}
  {value}
  placeholder={Runs.place(workdir, scratch, offered)}
  free={(typed) => typed.startsWith('/')}
  toggle={hasOthers
    ? { label: 'Show other places', on: others, onchange: (on) => (others = on) }
    : null}
  {disabled}
  onchange={(chosen) => {
    if (chosen !== value) onchange(chosen === SCRATCH ? null : chosen);
  }}
/>
