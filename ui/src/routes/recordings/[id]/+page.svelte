<script lang="ts">
  import TrafficWorkbench from '#lib/TrafficWorkbench.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
</script>

{#key data.sessionId}<TrafficWorkbench snapshot={data.traffic} sessionId={data.sessionId} />{/key}
