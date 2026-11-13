<!--
  Archive, unarchive, or delete a recorded session, through this UI server's
  control proxy. Shown only when the server drives runs (it holds the
  daemon's control token). A delete asks once more first.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { ManagementApi, type RecordingId } from './api.ts';
  import { RunCommandError, RunsClient, type HistoryChange } from './runs.ts';

  let { session }: { session: RecordingId } = $props();

  const client = new RunsClient();
  /** Whether it is archived; null until read. */
  let archived = $state<boolean | null>(null);
  let confirming = $state(false);
  let busy = $state(false);
  let notice = $state<string | null>(null);

  async function readShelf() {
    try {
      const listed = await new ManagementApi().sessions(100, undefined, 'archived');
      archived = listed.sessions.some((entry) => entry.id === session);
    } catch {
      archived = false;
    }
  }

  async function edit(change: HistoryChange) {
    busy = true;
    notice = null;
    confirming = false;
    try {
      await client.edit({ kind: 'session', id: session }, change);
      if (change === 'delete') {
        await goto('/recordings');
        return;
      }
      archived = change === 'archive';
    } catch (error) {
      notice = error instanceof RunCommandError ? error.message : 'The change was not made.';
    } finally {
      busy = false;
    }
  }

  onMount(() => void readShelf());
</script>

<span class="session-actions">
  {#if notice}<span class="danger-text">{notice}</span>{/if}
  {#if confirming}
    <span class="danger-text">Delete this recording’s calls for good?</span>
    <button class="button small danger confirm" disabled={busy} onclick={() => edit('delete')}
      >Delete</button
    >
    <button class="button small" onclick={() => (confirming = false)}>Cancel</button>
  {:else if archived !== null}
    <button
      class="button small"
      disabled={busy}
      onclick={() => edit(archived ? 'unarchive' : 'archive')}
      title={archived
        ? 'List it with the current recordings again'
        : 'Hide it from the recordings list until it has new traffic'}
      >{archived ? 'Unarchive' : 'Archive'}</button
    >
    <button class="button small danger" disabled={busy} onclick={() => (confirming = true)}
      >Delete</button
    >
  {/if}
</span>

<style>
  .session-actions {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .danger-text {
    color: var(--error);
    font-size: 11px;
  }
</style>
