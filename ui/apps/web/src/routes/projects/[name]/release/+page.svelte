<!-- A project's release: the page once its read is in, its outline before. -->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import PageOutline from '#lib/PageOutline.svelte';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import Release from './Release.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const release = createArrival(() => data.release);
</script>

{#if release.snapshot?.value}
  <Release data={release.snapshot.value} />
{:else}
  <Toolbar />
  <PageOutline figures={4} rows={8} error={release.snapshot?.error ?? null} />
{/if}
