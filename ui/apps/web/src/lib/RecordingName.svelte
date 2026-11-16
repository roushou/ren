<!--
  A recording as people recognise it: its client's colour and name, its newest
  prompt, and the folder it works in, falling back to the ID the client
  reported while the prompt has not been read.
-->
<script lang="ts">
  import { short, type RecordingView } from './api.ts';
  import { Board } from './board.ts';
  import type { RecordingDetail } from './recordings.ts';

  let {
    recording,
    detail,
    client = true,
  }: {
    recording: RecordingView;
    detail: RecordingDetail | null | undefined;
    /** Show the client's name, not only its colour. */
    client?: boolean;
  } = $props();

  const harness = $derived(Board.harness(recording.source));
  const prompt = $derived(detail?.prompt?.text.trim() || null);
  const folder = $derived(detail?.workdir ? Board.folder(detail.workdir) : null);
</script>

<span
  class="recording-name"
  title={[Board.clientName(harness), prompt, detail?.workdir, `session ${recording.native_id}`]
    .filter(Boolean)
    .join(' · ')}
  ><span class="harness-mark harness-{harness ?? 'unknown'}"></span>{#if client}<b
      >{Board.clientName(harness)}</b
    >{/if}{#if prompt}<span class="recording-name-prompt">{prompt}</span>{:else}<span
      class="recording-name-id mono">{short(recording.native_id)}</span
    >{/if}{#if folder}<span class="recording-name-folder mono">{folder}</span>{/if}</span
>

<style>
  .recording-name {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    max-width: 100%;
  }
  .recording-name :global(.harness-mark) {
    margin-right: 0;
  }
  b {
    flex: none;
    font-weight: 600;
  }
  .recording-name-prompt {
    min-width: 0;
    overflow: hidden;
    color: var(--ink);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .recording-name-id,
  .recording-name-folder {
    flex: none;
    color: var(--muted);
    font-size: 0.92em;
  }
</style>
