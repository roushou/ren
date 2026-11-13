<!--
  Redline's panel, flush beside the page while marking: the marks with their
  notes, the brief exactly as the agent will get it, and where it goes (the
  client and the folder it works in). Sending makes a session there with the
  brief as its first prompt, and opens it. The folder and client last sent
  to are remembered in this browser only.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { harnessLabel } from './activity.ts';
  import Picker, { type PickerGroup } from './Picker.svelte';
  import { Redline, type PageContext } from './redline.ts';
  import type { RedlineDesk } from './redline.svelte.ts';
  import {
    DRIVABLE,
    RunCommandError,
    Runs,
    RunsClient,
    type Drivable,
    type RunProject,
  } from './runs.ts';

  let { desk }: { desk: RedlineDesk } = $props();

  const client = new RunsClient();
  const KEEP = 'ren.redline';

  let projects = $state.raw<RunProject[]>([]);
  let ready = $state.raw<Drivable[]>([]);
  let folder = $state<string | null>(null);
  let harness = $state<Drivable>('claude_code');
  let others = $state(false);
  let loading = $state<string | null>('Reading the folders and clients…');
  let error = $state<string | null>(null);
  let sending = $state(false);

  /** What this browser last sent to: a convenience, so a refusal is ignored. */
  function recall(): { folder?: string; harness?: string } {
    try {
      return JSON.parse(localStorage.getItem(KEEP) ?? '{}') as {
        folder?: string;
        harness?: string;
      };
    } catch {
      return {};
    }
  }
  function keep() {
    try {
      localStorage.setItem(KEEP, JSON.stringify({ folder, harness }));
    } catch {
      // Not kept; the next send asks again.
    }
  }

  $effect(() => {
    const abort = new AbortController();
    Promise.all([client.projects(abort.signal), client.clients(abort.signal)]).then(
      ([places, clients]) => {
        projects = places.projects;
        ready = DRIVABLE.filter((drivable) =>
          clients.some((entry) => entry.harness === drivable && entry.status === 'ready'),
        );
        const kept = recall();
        folder = places.projects.some((project) => project.path === kept.folder)
          ? (kept.folder ?? null)
          : null;
        harness = ready.find((drivable) => drivable === kept.harness) ?? ready[0] ?? harness;
        loading = null;
      },
      (failure: unknown) => {
        if (abort.signal.aborted) return;
        loading = failure instanceof RunCommandError ? failure.message : 'Cannot read the folders.';
      },
    );
    return () => abort.abort();
  });

  const context = $derived.by((): PageContext => {
    const chosen = document.documentElement.dataset.theme;
    const dark = chosen ? chosen === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches;
    return {
      path: `${page.url.pathname}${page.url.search}`,
      route: page.route.id,
      title: document.title,
      width: window.innerWidth,
      height: window.innerHeight,
      theme: dark ? 'dark' : 'light',
    };
  });
  const noted = $derived(desk.marks.filter((mark) => mark.note));
  const brief = $derived(Redline.brief(context, noted));

  const option = (project: RunProject) => ({
    value: project.path,
    label: project.name,
    hint: Runs.branch(project.git),
    hintIcon: Runs.versioned(project.git) ? ('branch' as const) : undefined,
  });
  const groups = $derived<PickerGroup[]>([
    ...Runs.places(projects.filter((place) => place.kind === 'project')).map((place) => ({
      label: place.parent,
      options: place.projects.map(option),
    })),
    ...(others
      ? Runs.places(projects.filter((place) => place.kind === 'other')).map((place) => ({
          label: place.parent,
          options: place.projects.map(option),
        }))
      : []),
  ]);

  async function send() {
    error = null;
    if (!noted.length) {
      error = 'Mark something and write a note first.';
      return;
    }
    if (!folder) {
      error = 'Choose the folder the agent works in.';
      return;
    }
    sending = true;
    try {
      const made = await client.createSession(harness, folder);
      await client.toSession(made, { kind: 'prompt', text: brief, delivery: 'start' });
      keep();
      desk.clear();
      desk.open = false;
      await goto(`/sessions?session=${made}`);
    } catch (failure) {
      error = failure instanceof RunCommandError ? failure.message : 'The brief was not sent.';
    } finally {
      sending = false;
    }
  }
</script>

<aside class="redline-panel" data-redline aria-label="Redline">
  <header>
    <strong>Redline</strong>
    <span class="count">{desk.marks.length} mark{desk.marks.length === 1 ? '' : 's'}</span>
  </header>
  <p class="hint">Click anything on the page to mark it. <kbd>Esc</kbd> leaves; your marks stay.</p>

  {#each desk.marks as mark (mark.id)}
    <div class="mark">
      <span class="num">{mark.id}</span>
      <span class="what" title={mark.target.selector}
        >{mark.target.source ?? mark.target.selector}</span
      >
      <span class="text">{mark.note || 'Writing a note…'}</span>
      <button class="text-button remove" onclick={() => desk.drop(mark.id)}>Remove</button>
    </div>
  {/each}

  {#if noted.length}
    <section>
      <h3>Brief the agent gets</h3>
      <pre class="brief">{brief}</pre>
    </section>
  {/if}

  <section>
    <h3>Send to</h3>
    {#if loading}
      <p class="hint">{loading}</p>
    {:else}
      {#if ready.length}
        <div class="segmented" role="group" aria-label="Client">
          {#each ready as drivable (drivable)}
            <button aria-pressed={harness === drivable} onclick={() => (harness = drivable)}
              >{harnessLabel(drivable)}</button
            >
          {/each}
        </div>
      {:else}
        <p class="hint">No client can start here. Install one on the daemon's host.</p>
      {/if}
      <Picker
        label="Folder"
        icon="file"
        {groups}
        value={folder}
        placeholder="Choose a folder"
        free={(typed) => typed.startsWith('/')}
        toggle={projects.some((place) => place.kind === 'other')
          ? { label: 'Show other places', on: others, onchange: (on) => (others = on) }
          : null}
        onchange={(value) => (folder = value)}
      />
      <p class="hint">
        The agent is asked to work on a new branch in a git worktree, and not to install anything.
      </p>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      <button
        class="button primary redline-send"
        disabled={sending || !!loading || !ready.length}
        onclick={() => void send()}
        >{sending ? 'Sending…' : `Send to ${harnessLabel(harness)}`}</button
      >
      {#if desk.marks.length}
        <button class="button" onclick={() => desk.clear()}>Clear marks</button>
      {/if}
    </div>
  </section>
</aside>

<style>
  .redline-panel {
    width: 340px;
    flex-shrink: 0;
    overflow-x: hidden;
    overflow-y: auto;
    border-left: 1px solid var(--line);
    background: var(--surface);
    display: flex;
    flex-direction: column;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 10px 12px;
    border-top: 2px solid var(--redline);
    border-bottom: 1px solid var(--line);
  }
  .count {
    margin-left: auto;
    color: var(--muted);
    font:
      11px 'SFMono-Regular',
      Consolas,
      monospace;
  }
  .hint {
    margin: 0;
    padding: 8px 12px;
    color: var(--muted);
    font-size: 12px;
  }
  section .hint {
    padding: 0;
  }
  .mark {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr);
    gap: 3px 10px;
    padding: 10px 12px;
    border-top: 1px solid var(--line);
  }
  .num {
    width: 18px;
    height: 18px;
    display: grid;
    place-items: center;
    background: var(--redline);
    color: var(--on-redline);
    font:
      700 11px 'SFMono-Regular',
      Consolas,
      monospace;
  }
  .what {
    color: var(--muted);
    font:
      11px 'SFMono-Regular',
      Consolas,
      monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    align-self: center;
  }
  .text,
  .remove {
    grid-column: 2;
  }
  .text {
    font-size: 13px;
    overflow-wrap: anywhere;
  }
  .remove {
    justify-self: start;
    font-size: 11px;
  }
  section {
    display: grid;
    gap: 8px;
    padding: 10px 12px;
    border-top: 1px solid var(--line);
  }
  h3 {
    margin: 0;
    color: var(--muted);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .brief {
    margin: 0;
    max-height: 240px;
    overflow-y: auto;
    padding: 8px 10px;
    border: 1px solid var(--line);
    background: var(--surface-subtle);
    font:
      11px/1.55 'SFMono-Regular',
      Consolas,
      monospace;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .segmented {
    display: flex;
    flex-wrap: wrap;
    width: fit-content;
    border: 1px solid var(--control-border);
    border-radius: 3px;
    overflow: hidden;
  }
  .segmented button {
    padding: 3px 9px;
    border: 0;
    border-right: 1px solid var(--control-border);
    background: var(--surface);
    color: var(--ink);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .segmented button:last-child {
    border-right: 0;
  }
  .segmented button[aria-pressed='true'] {
    background: var(--accent-soft);
    color: var(--accent-text);
    font-weight: 600;
  }
  /* The folder list fits the panel instead of the page. */
  .redline-panel :global(.picker .panel) {
    width: 316px;
    max-width: calc(100vw - 24px);
  }
  .error {
    margin: 0;
    color: var(--redline);
    font-size: 12px;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .redline-send,
  .redline-send:hover:not(:disabled) {
    background: var(--redline);
    border-color: var(--redline);
    color: var(--on-redline);
  }
  .redline-send:hover:not(:disabled) {
    background: var(--redline-hover);
    border-color: var(--redline-hover);
  }
  @media (max-width: 760px) {
    .redline-panel {
      width: auto;
      max-height: 45vh;
      border-left: 0;
      border-top: 1px solid var(--line);
    }
  }
</style>
