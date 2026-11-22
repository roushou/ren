<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import { page } from '$app/state';
</script>

<svelte:head><title>Ren - {page.status}</title></svelte:head>
<Toolbar />
<div class="empty-state route-error">
  <h2>{page.error?.message ?? 'Unable to open this page'}</h2>
  <a class="button" href="/projects">Return to Projects</a>
</div>
