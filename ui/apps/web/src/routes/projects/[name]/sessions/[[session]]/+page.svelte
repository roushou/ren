<!--
  A project's sessions: the session desk (lib/desk/SessionDesk.svelte),
  holding only the sessions working for it; a new one starts in its
  repository.
-->
<script lang="ts">
  import SessionDesk from '#lib/desk/SessionDesk.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
</script>

<SessionDesk {data} scope={{ kind: 'project', project: data.project }} />
