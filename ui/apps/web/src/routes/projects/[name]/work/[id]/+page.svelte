<!--
  One work item of a project (docs/plans/self-build.md, phase 5): what was
  marked, where it stands, and the evidence it carries, with Dismiss. Redline,
  the Pending panel, and the Release page link here.
-->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import WorkItemDetail from '#lib/WorkItemDetail.svelte';
  import { invalidateAll } from '$app/navigation';
  import PageOutline from '#lib/PageOutline.svelte';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { WorkApi, WorkError, type WorkItem } from '#lib/work.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const arrival = createArrival(() => data.work);
  const read = $derived(arrival.snapshot?.value ?? null);
  const work = new WorkApi(fetch);
  let failed = $state<string | null>(null);

  async function dismiss(item: WorkItem) {
    failed = null;
    try {
      await work.move(item.id, { kind: 'dismiss' });
      await invalidateAll();
    } catch (failure) {
      failed = failure instanceof WorkError ? failure.message : 'The work item was not dismissed.';
    }
  }
</script>

<Toolbar />

{#if read}
  <section class="item" aria-label={read.item.title}>
    {#if failed}<p class="error" role="alert">{failed}</p>{/if}
    <WorkItemDetail
      item={read.item}
      project={read.project.factory === 'ren'
        ? { kind: 'ren' }
        : { kind: 'other', name: read.project.name }}
      ondismiss={(item) => void dismiss(item)}
      pulls={read.pulls}
      repository={read.repository}
    />
  </section>
{:else}
  <PageOutline figures={0} rows={8} error={arrival.snapshot?.error ?? null} />
{/if}

<style>
  .item {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .error {
    margin: 0;
    padding: 8px 16px;
    color: var(--error);
  }
</style>
