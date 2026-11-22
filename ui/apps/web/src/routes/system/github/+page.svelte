<!-- System · GitHub: the page once its read is in, its outline before. -->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import PageOutline from '#lib/PageOutline.svelte';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import GitHub from './GitHub.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const github = createArrival(() => data.github);
</script>

{#if github.snapshot?.value}
  <GitHub data={github.snapshot.value} />
{:else}
  <Toolbar />
  <PageOutline figures={5} rows={6} error={github.snapshot?.error ?? null} />
{/if}
