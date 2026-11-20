<!--
  One work item of a project (docs/plans/self-build.md, phase 5): what was
  marked, where it stands, and the evidence it carries, with Dismiss. Redline,
  the Pending panel, and the Release page link here.
-->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import WorkItemDetail from '#lib/WorkItemDetail.svelte';
  import { invalidateAll } from '$app/navigation';
  import { WorkApi, WorkError, type WorkItem } from '#lib/work.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
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

{#snippet scope()}
  <span class="muted">{data.item.title}</span>
{/snippet}

<Toolbar {scope} />

<section class="item" aria-label={data.item.title}>
  {#if failed}<p class="error" role="alert">{failed}</p>{/if}
  <WorkItemDetail
    item={data.item}
    project={data.project.factory === 'ren'
      ? { kind: 'ren' }
      : { kind: 'other', name: data.project.name }}
    ondismiss={(item) => void dismiss(item)}
  />
</section>

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
  .muted {
    color: var(--muted);
  }
</style>
