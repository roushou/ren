<script lang="ts">
  import TrafficWorkbench from '#lib/TrafficWorkbench.svelte';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  const desks = createArrival(() => data.desks);
  /** Where the session opens on its desk, when Ren drives it. */
  const desk = $derived.by(() => {
    return desks.snapshot?.value?.get(data.recordingId) ?? null;
  });
</script>

{#key data.recordingId}<TrafficWorkbench
    snapshot={data.traffic}
    recordingId={data.recordingId}
    {desk}
  />{/key}
