<!--
  Scratch: the session desk (lib/desk/SessionDesk.svelte) for the sessions
  that work for no project, in scratch folders or in folders no project is
  registered for; a new one starts in a scratch folder of its own.
-->
<script lang="ts">
  import SessionDesk from '#lib/desk/SessionDesk.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
</script>

<SessionDesk {data} scope={{ kind: 'scratch', projects: data.projects }} />
