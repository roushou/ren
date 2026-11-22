<script lang="ts">
  import TrafficWorkbench from '#lib/TrafficWorkbench.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
</script>

{#key data.recordingId}<TrafficWorkbench
    snapshot={data.traffic}
    recordingId={data.recordingId}
    desk={data.desk}
  />{/key}
