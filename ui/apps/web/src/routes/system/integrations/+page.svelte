<!-- System · Integrations: the page once its read is in, its outline before. -->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import PageOutline from '#lib/PageOutline.svelte';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import Integrations from './Integrations.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const integrations = createArrival(() => data.integrations);
</script>

{#if integrations.snapshot?.value}
  <Integrations data={integrations.snapshot.value} />
{:else}
  <Toolbar />
  <PageOutline figures={5} rows={6} error={integrations.snapshot?.error ?? null} />
{/if}
