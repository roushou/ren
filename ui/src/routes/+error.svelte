<script lang="ts">
  import { page } from '$app/state';
</script>

<svelte:head><title>TGV - {page.status}</title></svelte:head>
<div class="empty-state route-error">
  <h2>{page.error?.message ?? 'Unable to open this page'}</h2>
  <a class="button" href="/traffic">Return to Traffic</a>
</div>
