<!--
  A project's work: what was marked to be done on its pages, from queued
  to shipped. Queued items can be picked and sent together, as one brief,
  to a new session in a worktree of its own in the project's repository
  (Ren's source checkout for Ren's); each row says where its item stands,
  and the one picked shows its marks, what else was on screen, and the
  brief it makes.
-->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import WorkItemDetail from '#lib/WorkItemDetail.svelte';
  import Picker, { type PickerGroup } from '#lib/Picker.svelte';
  import { goto, invalidateAll } from '$app/navigation';
  import { page } from '$app/state';
  import { onMount } from 'svelte';
  import { ago } from '#lib/overview.ts';
  import { harnessLabel } from '#lib/activity.ts';
  import { DRIVABLE, RunCommandError, RunsClient, Sessions, type Drivable } from '#lib/runs.ts';
  import { Work, WorkApi, WorkError, type WorkItem, type WorkItemId } from '#lib/work.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const work = new WorkApi(fetch);
  const runs = new RunsClient();

  const items = $derived(data.listing.kind === 'ready' ? data.listing.items : []);
  /** The queued items picked to send together. */
  let picked = $state<WorkItemId[]>([]);
  const batch = $derived(items.filter((item) => picked.includes(item.id)));
  /** The item whose detail is shown: the one in the address, else the first. */
  const shown = $derived(items.find((item) => item.id === data.selected) ?? items[0] ?? null);
  let error = $state<string | null>(null);
  let busy = $state(false);

  /** The clients ready to take a batch, read once. */
  let ready = $state<Drivable[]>([]);
  let harness = $state<Drivable | null>(null);
  onMount(() => {
    if (!page.data.runsControl) return;
    runs.clients().then(
      (clients) => {
        ready = DRIVABLE.filter((drivable) =>
          clients.some((entry) => entry.harness === drivable && entry.status === 'ready'),
        );
        harness = ready[0] ?? null;
      },
      () => (ready = []),
    );
  });
  const clientGroups = $derived<PickerGroup[]>([
    {
      label: 'New session',
      options: ready.map((drivable) => ({
        value: drivable,
        label: `New ${harnessLabel(drivable)} session`,
      })),
    },
  ]);

  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => (now = Date.now()), 15_000);
    return () => clearInterval(timer);
  });

  function toggle(item: WorkItem) {
    picked = picked.includes(item.id)
      ? picked.filter((id) => id !== item.id)
      : [...picked, item.id];
  }

  /** Whose pages the picked items are about: one project, or null when
   * they mix and go one project at a time. */
  const batchProject = $derived(Work.oneProject(batch, data.projects));
  /** Why the picked items cannot go as one brief, if they cannot. */
  const unsendable = $derived(
    batchProject === null
      ? 'The picked items are for different projects: send one project’s at a time'
      : batchProject.kind === 'ren' && !page.data.redlineSource
        ? 'Set REN_SOURCE_DIR to send work to Ren’s checkout'
        : null,
  );

  /** The picked items, as one brief, to a new session in a worktree of
   * its own in their project's repository (Ren's own checkout for Ren's);
   * each item is sent to it. */
  async function send() {
    const source = page.data.redlineSource;
    if (!batch.length || !harness || !batchProject) return;
    if (batchProject.kind === 'ren' && !source) return;
    error = null;
    busy = true;
    try {
      const session =
        batchProject.kind === 'ren'
          ? await runs.createSession(harness, source, {}, 'worktree')
          : await runs.createProjectSession(harness, batchProject.name);
      await runs.toSession(session, {
        kind: 'prompt',
        text: Work.batch(batch, 'worktree', batchProject),
        delivery: 'start',
      });
      for (const item of batch) await work.move(item.id, { kind: 'send', session });
      picked = [];
      await goto(Sessions.deskHref(data.project.name, { session }));
    } catch (failure) {
      error =
        failure instanceof RunCommandError || failure instanceof WorkError
          ? failure.message
          : 'The work items were not sent.';
    } finally {
      busy = false;
    }
  }

  async function dismiss(item: WorkItem) {
    error = null;
    try {
      await work.move(item.id, { kind: 'dismiss' });
      picked = picked.filter((id) => id !== item.id);
      await invalidateAll();
    } catch (failure) {
      error = failure instanceof WorkError ? failure.message : 'The work item was not dismissed.';
    }
  }

  const select = (item: WorkItem) =>
    goto(
      `?${new URLSearchParams({ ...(data.shelf === 'archived' ? { shelf: 'archived' } : {}), item: item.id })}`,
      {
        replaceState: true,
      },
    );
</script>

{#snippet scope()}
  <div class="scope-group">
    <strong>{items.length}</strong>
    <span class="muted"
      >{data.shelf === 'archived' ? 'shipped or dismissed' : 'work items under way'}</span
    >
  </div>
  <div class="scope-group segmented" role="group" aria-label="Which work items">
    <a class="button small" class:chosen={data.shelf === 'current'} href={page.url.pathname}
      >Current</a
    >
    <a class="button small" class:chosen={data.shelf === 'archived'} href="?shelf=archived"
      >Archived</a
    >
  </div>
{/snippet}

<Toolbar {scope} />

<section class="work" aria-label="Work items">
  <div class="work-list">
    {#if data.listing.kind === 'failed'}
      <p class="error" role="alert">{data.listing.message}</p>
    {:else if !items.length}
      <p class="empty">
        {data.shelf === 'archived'
          ? 'Nothing shipped or dismissed yet.'
          : 'Nothing marked yet. Open Redline on any page, mark what is wrong, and send it or keep it for Later.'}
      </p>
    {:else}
      <table class="work-table">
        <thead>
          <tr>
            <th class="col-pick"><span class="sr-only">Pick</span></th>
            <th>Work item</th>
            <th>Where it stands</th>
            <th class="numeric">Changed</th>
          </tr>
        </thead>
        <tbody>
          {#each items as item (item.id)}
            <tr class:current={shown?.id === item.id} onclick={() => void select(item)}>
              <td class="col-pick">
                {#if item.state === 'queued'}<input
                    type="checkbox"
                    aria-label={`Send “${item.title}” with the others picked`}
                    checked={picked.includes(item.id)}
                    onclick={(event) => event.stopPropagation()}
                    onchange={() => toggle(item)}
                  />{/if}
              </td>
              <td>
                <span class="title">{item.title}</span>
                <span class="meta mono"
                  >{item.order.page.path} · {item.order.marks.length}
                  {item.order.marks.length === 1
                    ? 'mark'
                    : 'marks'}{#if item.order.evidence.console.length}{' '}·
                    <span class="errors"
                      >{item.order.evidence.console.length}
                      {item.order.evidence.console.length === 1 ? 'error' : 'errors'}</span
                    >{/if}</span
                >
              </td>
              <td>
                <span class="work-state state-{item.state}">{Work.state(item.state)}</span>
                {#if item.session}<a
                    class="session-link"
                    href={Sessions.deskHref(data.project.name, { session: item.session })}
                    onclick={(event) => event.stopPropagation()}>Open session</a
                  >{/if}
              </td>
              <td class="numeric mono">{ago(item.updated_at, now)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
    {#if picked.length}
      <div class="batch" role="region" aria-label="Send picked work items">
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <span class="batch-count">{picked.length} picked</span>
        <span class="batch-client">
          <Picker
            label="Where the brief goes"
            icon="arrow"
            groups={clientGroups}
            value={harness}
            placeholder="New session"
            search={false}
            disabled={unsendable ?? (ready.length ? null : 'No client is ready')}
            onchange={(value) => (harness = DRIVABLE.find((entry) => entry === value) ?? harness)}
          />
        </span>
        <button
          class="button primary"
          disabled={busy || !harness || unsendable !== null}
          title={unsendable ?? undefined}
          onclick={() => void send()}>{busy ? 'Sending…' : 'Send as one brief'}</button
        >
      </div>
    {/if}
  </div>
  <aside class="work-detail" aria-label="Work item">
    {#if shown}
      <WorkItemDetail
        item={shown}
        project={Work.projectOf(shown, data.projects)}
        ondismiss={(item) => void dismiss(item)}
      />
    {:else}
      <p class="empty">Pick a work item to see what was marked.</p>
    {/if}
  </aside>
</section>

<style>
  .work {
    display: grid;
    grid-template-columns: minmax(0, 3fr) minmax(320px, 2fr);
    min-height: 0;
    flex: 1;
  }
  .work-list {
    display: flex;
    flex-direction: column;
    min-width: 0;
    border-right: 1px solid var(--line);
    overflow: auto;
  }
  .work-table {
    width: 100%;
    border-collapse: collapse;
  }
  .work-table th {
    text-align: left;
    font-weight: 500;
    color: var(--muted);
    padding: 6px 12px;
    border-bottom: 1px solid var(--line);
  }
  .work-table td {
    padding: 8px 12px;
    border-bottom: 1px solid var(--line);
    vertical-align: top;
  }
  .work-table tr {
    cursor: pointer;
  }
  .work-table tbody tr:hover {
    background: var(--surface-hover);
  }
  .work-table tr.current {
    background: var(--surface-hover);
  }
  .col-pick {
    width: 32px;
  }
  .numeric {
    text-align: right;
    white-space: nowrap;
  }
  .title {
    display: block;
    font-weight: 500;
  }
  .meta {
    color: var(--muted);
    font-size: 11px;
    margin: 2px 0 0;
    overflow-wrap: anywhere;
  }
  .errors {
    color: var(--error);
  }
  .session-link {
    margin-left: 8px;
    font-size: 12px;
  }
  .batch {
    position: sticky;
    bottom: 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    margin-top: auto;
    border-top: 1px solid var(--line);
    background: var(--surface);
  }
  .batch .error {
    flex-basis: 100%;
    margin: 0;
  }
  .batch-client {
    flex: 1;
    min-width: 0;
  }
  .work-detail {
    min-width: 0;
    overflow: auto;
    padding: 12px 16px;
  }
  .empty {
    color: var(--muted);
    padding: 16px;
  }
  .error {
    color: var(--error);
    padding: 0 12px;
  }
  .muted {
    color: var(--muted);
  }
  @media (max-width: 900px) {
    .work {
      grid-template-columns: minmax(0, 1fr);
    }
    .work-list {
      border-right: none;
      border-bottom: 1px solid var(--line);
    }
  }
</style>
