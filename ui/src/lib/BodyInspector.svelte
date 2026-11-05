<script lang="ts">
  import { onMount } from 'svelte';
  import {
    ManagementApi,
    bytes,
    title,
    ApiError,
    type BodyPreview,
    type BodyView,
    type CallDetail,
    type CallId,
    type Direction,
    type MessageView,
  } from './api.ts';
  import Icon from './Icon.svelte';
  import CopyButton from './CopyButton.svelte';
  import { bodyView } from './body-view.ts';
  import { snapshotError } from './snapshots.ts';
  let {
    id,
    initialDirection = 'request',
    bodies,
    onupdate,
  }: {
    id: CallId;
    initialDirection?: Direction;
    /** The call's latest body records, refreshed by the live feed. */
    bodies: Record<Direction, BodyView | null>;
    onupdate: (detail: CallDetail) => void;
  } = $props();
  const api = new ManagementApi();
  let direction = $state<Direction>('request');
  let mode = $state<'readable' | 'raw'>('readable');
  let preview = $state<BodyPreview | null>(null);
  let error = $state('');
  let following = $state(false);
  let updated = $state<number | null>(null);
  let stopped = $state('');
  let busy = $state(false);
  let controller: AbortController | undefined;
  let generation = 0;
  /** WebSocket message boundaries, from the call detail read after each preview. */
  let messages = $state<MessageView[]>([]);
  const readable = $derived(
    preview?.text != null
      ? bodyView(
          preview.text,
          messages.filter((message) => message.direction === direction),
          direction,
        )
      : null,
  );
  const open = $derived(preview?.ending === 'open');
  async function load(next: Direction, quiet = false) {
    controller?.abort();
    controller = new AbortController();
    const revision = ++generation;
    direction = next;
    if (!quiet) preview = null;
    error = '';
    busy = true;
    try {
      const result = await api.body(id, next, controller.signal);
      if (revision !== generation) return;
      preview = result;
      updated = Date.now();
      // Read metadata after the body snapshot so final capture precedes final status.
      const record = await api.call(id, controller.signal);
      if (revision !== generation) return;
      messages = record.messages;
      onupdate(record);
      if (following && result.ending !== 'open') {
        following = false;
        stopped =
          record.call.state === 'recovered'
            ? 'Recording interrupted by restart.'
            : 'Recording ended.';
      }
    } catch (e) {
      if (revision === generation && !controller.signal.aborted) {
        error = snapshotError(e, 'Body could not be loaded.').message;
        if (e instanceof ApiError && e.status === 404) {
          following = false;
          preview = null;
          stopped = 'Capture is no longer available.';
        }
      }
    } finally {
      if (revision === generation) busy = false;
    }
  }
  // A snapshot taken while the body was open goes stale once the call's record
  // shows the body ended; reload it once so the preview matches the row.
  $effect(() => {
    const body = bodies[direction];
    if (preview?.ending === 'open' && body && body.ending !== 'open' && !following && !busy)
      void load(direction, true);
  });
  function follow(enabled: boolean) {
    following = enabled;
    stopped = '';
    if (following) void load(direction, true);
    else {
      generation++;
      controller?.abort();
      busy = false;
    }
  }
  onMount(() => {
    void load(initialDirection);
    const timer = setInterval(() => {
      if (following && !busy && !document.hidden) void load(direction, true);
    }, 2000);
    return () => {
      clearInterval(timer);
      generation++;
      controller?.abort();
    };
  });
</script>

<section class="body-inspector" aria-label="Captured payload">
  <div class="payload-toolbar">
    <div class="segmented" aria-label="Payload view">
      <button
        class:chosen={mode === 'readable'}
        aria-pressed={mode === 'readable'}
        onclick={() => (mode = 'readable')}>Readable</button
      ><button
        class:chosen={mode === 'raw'}
        aria-pressed={mode === 'raw'}
        onclick={() => (mode = 'raw')}><Icon name="code" size={13} /> Raw</button
      >
    </div>
    <div class="payload-actions">
      <label class="toggle-control"
        ><input
          type="checkbox"
          checked={following}
          onchange={(e) => follow(e.currentTarget.checked)}
        /><span class="switch"></span><span>Follow live</span></label
      ><button
        class="icon-button"
        aria-label="Refresh payload"
        disabled={busy}
        onclick={() => load(direction, true)}><Icon name="refresh" size={14} /></button
      >{#if preview}<CopyButton
          text={mode === 'raw' ? preview.raw.text : (preview.text ?? preview.raw.text)}
          label="Copy payload preview"
        />{/if}
    </div>
  </div>
  <div class="payload-status" role="status">
    <span class="status-dot" class:live={following} class:neutral={!following}></span><span
      >{following ? 'Live · every 2 seconds' : stopped || 'Snapshot'}{updated
        ? ` · ${new Date(updated).toLocaleTimeString()}`
        : ''}</span
    >{#if preview}<span class="payload-size">{bytes(preview.captured_bytes)}</span>{/if}
  </div>
  {#if busy && !preview}<div class="payload-loading" role="status">
      <span class="spinning"><Icon name="refresh" size={20} /></span>
      <p>Loading {direction} payload…</p>
    </div>{/if}
  {#if error}<p class="callout" role="alert">
      {error}{preview ? ' Showing the last loaded snapshot.' : ''}
    </p>{/if}
  {#if preview}
    <p class="fine-print">
      {bytes(preview.captured_bytes)} shown · {bytes(preview.stored_bytes)} stored · {preview.observed_bytes !==
      null
        ? `${bytes(preview.observed_bytes)} observed`
        : open
          ? 'observed size unknown while open'
          : 'observed size unknown'} · {preview.extent === 'complete'
        ? 'Complete body'
        : preview.extent === 'partial'
          ? 'Partial body'
          : preview.extent === 'truncated'
            ? 'Preview limit reached'
            : preview.extent === 'gap'
              ? 'Stops at a capture gap'
              : title(preview.extent)}
    </p>
    {#if open}<p class="callout">
        {following
          ? 'Streaming captured output. Totals arrive when recording ends.'
          : 'Recording in progress. Enable Follow live to watch output arrive.'}
      </p>
    {:else if preview.ending === 'unfinished'}<p class="callout">
        The body never finished. Only the captured bytes are shown; missing bytes are not
        reconstructed.
      </p>
    {:else if preview.extent === 'partial'}<p class="callout">
        Not every observed byte was stored. Missing bytes are not reconstructed.
      </p>{/if}
    {#if preview.extent === 'gap'}<p class="callout">
        Capture gap: preview stops before missing bytes.
      </p>{/if}
    {#if preview.extent === 'truncated' || preview.decoding.truncated}<p class="callout">
        Preview size limit reached. This view may not contain the whole captured body.
      </p>{/if}
    {#if preview.text_end === 'split_character'}<p class="fine-print">
        Captured text ends mid-character. Readable shows only complete UTF-8 characters; Raw retains
        every captured byte.
      </p>{/if}
    {#if preview.decoding.applied !== 'none'}<p class="fine-print">
        {preview.decoding.applied === 'gzip_magic'
          ? 'Gzip'
          : preview.decoding.applied === 'zstd_magic'
            ? 'Zstandard'
            : title(preview.decoding.applied)} detected from captured bytes. Readable view uses decoded
        content; Raw preserves the original bytes.{preview.decoding.integrity === 'verified'
          ? ' Integrity verified.'
          : ''}
      </p>{/if}
    {#if preview.decoding.integrity === 'prefix'}<p class="callout">
        Unverified decoded prefix: the compressed stream is incomplete. Its final integrity check is
        not yet available.
      </p>
    {:else if preview.decoding.integrity === 'failed'}<p class="callout">
        Compressed content is incomplete, corrupt, or exceeds decoder limits. Original bytes remain
        in Raw.
      </p>{/if}
    {#if mode === 'raw'}
      <p class="fine-print">
        {preview.raw.format === 'hex' ? 'Hexadecimal bytes' : 'Captured UTF-8 text'}{preview.raw
          .truncated
          ? ' · first 16 KiB only'
          : ''}
      </p>
      <pre class="body-raw">{preview.raw.text || 'Empty captured body.'}</pre>
    {:else if readable}
      {#each readable.notices as notice}<p class="callout">{notice}</p>{/each}
      {#each readable.cards as card}<article class="body-card">
          <h5>
            <span class="message-icon"
              ><Icon
                name={card.label.toLowerCase().includes('tool') ? 'tool' : 'sessions'}
                size={13}
              /></span
            >{card.label}
          </h5>
          <pre>{card.text || '(empty)'}</pre>
        </article>{/each}
    {:else if !preview.captured_bytes}<p class="fine-print">No body bytes available.</p>
    {:else}<p class="fine-print">
        No readable text is available. Use Raw to inspect the captured bytes.
      </p>{/if}
  {:else if !busy && !error}<p class="fine-print">
      Load a body to inspect messages, tool arguments, and results. Content stays in this view and
      is not saved in browser storage.
    </p>{/if}
</section>
