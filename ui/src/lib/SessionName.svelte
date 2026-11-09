<!--
  A session as people recognise it: its client's colour and name, its newest
  prompt, and the folder it works in, falling back to the ID the client
  reported while the prompt has not been read.
-->
<script lang="ts">
  import { short, type SessionView } from './api.ts';
  import { Board } from './board.ts';
  import type { SessionDetail } from './sessions.ts';

  let {
    session,
    detail,
    client = true,
  }: {
    session: SessionView;
    detail: SessionDetail | null | undefined;
    /** Show the client's name, not only its colour. */
    client?: boolean;
  } = $props();

  const harness = $derived(Board.harness(session.source));
  const prompt = $derived(detail?.prompt?.text.trim() || null);
  const folder = $derived(detail?.workdir ? Board.folder(detail.workdir) : null);
</script>

<span
  class="session-name"
  title={[Board.clientName(harness), prompt, detail?.workdir, `session ${session.native_id}`]
    .filter(Boolean)
    .join(' · ')}
  ><span class="harness-mark harness-{harness ?? 'unknown'}"></span>{#if client}<b
      >{Board.clientName(harness)}</b
    >{/if}{#if prompt}<span class="session-name-prompt">{prompt}</span>{:else}<span
      class="session-name-id mono">{short(session.native_id)}</span
    >{/if}{#if folder}<span class="session-name-folder mono">{folder}</span>{/if}</span
>

<style>
  .session-name {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    max-width: 100%;
  }
  .session-name :global(.harness-mark) {
    margin-right: 0;
  }
  b {
    flex: none;
    font-weight: 600;
  }
  .session-name-prompt {
    min-width: 0;
    overflow: hidden;
    color: var(--ink);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .session-name-id,
  .session-name-folder {
    flex: none;
    color: var(--muted);
    font-size: 0.92em;
  }
</style>
