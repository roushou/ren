<script lang="ts">
  /*
    A session's head in Monitor, over its calls and over its report: where
    it sits (Sessions / its ID), what it is (its newest prompt), and its
    client, provider, folder, and time span; the desk link when Ren drives
    it; Calls and Report; and archive or delete.
  */
  import { short, providerName, type RecordingId, type RecordingView } from './api.ts';
  import { Board } from './board.ts';
  import { span } from './display.ts';
  import type { RecordingDetail } from './recordings.ts';
  import RecordingActions from './RecordingActions.svelte';
  import { page } from '$app/state';

  let {
    recordingId,
    recording,
    detail,
    desk = null,
    view,
    now,
  }: {
    recordingId: RecordingId;
    recording: RecordingView | undefined;
    detail: RecordingDetail | null;
    /** Where it opens on its desk, when Ren drives it. */
    desk?: string | null;
    view: 'calls' | 'report';
    now: number;
  } = $props();

  const base = $derived(`/monitor/sessions/${encodeURIComponent(recordingId)}`);
  /** Seen this recently, the session reads as going on now. */
  const RECENT = 2 * 60_000;
  const clock = (ms: number) =>
    new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const title = $derived(
    detail?.prompt?.text.trim().split('\n')[0] ||
      (recording ? Board.clientName(Board.harness(recording.source)) : short(recordingId)),
  );
  const facts = $derived.by(() => {
    if (!recording) return [];
    const live = now - recording.last_seen < RECENT;
    return [
      Board.clientName(Board.harness(recording.source)).toLowerCase(),
      providerName(recording.provider).toLowerCase(),
      ...(detail?.workdir ? [Board.folder(detail.workdir)] : []),
      `${clock(recording.first_seen)} – ${live ? 'now' : clock(recording.last_seen)}`,
      span((live ? now : recording.last_seen) - recording.first_seen),
    ];
  });
</script>

<section class="session-head" aria-labelledby="session-title">
  <div class="who">
    <div class="meta">
      <a href="/monitor/sessions">Sessions</a> / {short(
        recording?.native_id ?? recordingId,
      )}{#if view === 'report'}{' '}/ report{/if}
    </div>
    <h1 id="session-title">{title}</h1>
    <div class="meta">
      {facts.join(' · ')}{#if desk}{facts.length ? ' · ' : ''}<a href={desk}>open in desk ↗</a>{/if}
    </div>
  </div>
  <nav class="views" aria-label="Session views">
    <a href={base} aria-current={view === 'calls' ? 'page' : undefined}>Calls</a><a
      href="{base}/report"
      aria-current={view === 'report' ? 'page' : undefined}>Report</a
    >
  </nav>
  {#if page.data.runsControl}<RecordingActions
      recording={recordingId}
      shelf={recording ? recording.archived_at !== null : null}
    />{/if}
</section>

<style>
  .session-head {
    display: flex;
    align-items: flex-start;
    flex-wrap: wrap;
    gap: 16px;
    padding: 12px 16px 8px;
    background: var(--surface);
  }
  .who {
    flex: 1;
    min-width: 0;
  }
  h1 {
    margin: 2px 0 0;
    overflow: hidden;
    font-size: 16px;
    font-weight: 600;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .meta {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11px;
  }
  .meta a {
    color: var(--muted);
    text-decoration: underline;
  }
  .meta a:hover {
    color: var(--ink);
  }
  .views {
    display: flex;
    border: 1px solid var(--control-border);
  }
  .views a {
    padding: 4px 10px;
    color: var(--muted);
    font-size: 12px;
    font-weight: 500;
  }
  .views a + a {
    border-left: 1px solid var(--control-border);
  }
  .views a[aria-current='page'] {
    background: var(--surface-hover);
    color: var(--ink);
  }
</style>
