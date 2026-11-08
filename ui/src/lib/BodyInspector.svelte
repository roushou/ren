<script lang="ts">
  import { onMount } from 'svelte';
  import {
    ManagementApi,
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
  import Segmented, { type SegmentedOption } from './Segmented.svelte';
  import { bodyView, toolFields, type Card, type CardKind } from './body-view.ts';
  import { previewChips } from './display.ts';
  import Markdown from './Markdown.svelte';
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
  const modes: SegmentedOption<'readable' | 'raw'>[] = [
    { value: 'readable', label: 'Readable' },
    { value: 'raw', label: 'Raw', icon: 'code' },
  ];
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
  const chips = $derived(preview ? previewChips(preview) : []);
  /** Thinking the provider withheld is a fact about the response, not a
   * message: it becomes a chip instead of an empty card. */
  const hiddenThinking = $derived(
    readable?.cards.filter((card) => card.kind === 'thinking' && !card.text).length ?? 0,
  );
  const cards = $derived(
    readable?.cards
      .map((card, index) => ({ card, index }))
      .filter(({ card }) => !(card.kind === 'thinking' && !card.text)) ?? [],
  );
  /** Cards that start folded: context that is the same on every call. */
  function folded(card: Card) {
    return (
      card.kind === 'system' ||
      card.kind === 'tools' ||
      card.label === 'Other fields' ||
      (card.kind === 'thinking' && !card.text)
    );
  }
  /** Long cards show their start until expanded. */
  let expanded = $state(new Set<number>());
  /** The model's own answers are never cut short; context and data are. */
  const long = (card: Card) => {
    if (card.kind === 'assistant') return false;
    // Tagged context folds on its own, so it does not count toward length.
    const shown =
      card.format === 'prose'
        ? card.text.replace(/<([a-z][a-z0-9]*[-_][a-z0-9_-]*)>[\s\S]*?<\/\1>/g, '')
        : card.text;
    return shown.length > 2400 || shown.split('\n').length > 40;
  };
  let cardList = $state<HTMLElement>();
  function latest() {
    const cards = cardList?.querySelectorAll('.body-card');
    cards?.[cards.length - 1]?.scrollIntoView({ block: 'start', behavior: 'smooth' });
  }
  function icon(kind: CardKind) {
    switch (kind) {
      case 'user':
        return 'user';
      case 'assistant':
        return 'spark';
      case 'thinking':
        return 'thought';
      case 'tool_call':
      case 'tool_result':
      case 'tool_error':
      case 'tools':
        return 'tool';
      case 'error':
        return 'alert';
      case 'system':
        return 'info';
      case 'other':
        return 'code';
      default:
        return 'code';
    }
  }
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
    <Segmented label="Payload view" options={modes} bind:value={mode} />
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
    >{#if cards.length > 6}<button class="text-button jump-latest" onclick={latest}
        >Jump to latest <Icon name="chevron" size={12} /></button
      >{/if}
  </div>
  {#if busy && !preview}<div class="payload-loading" role="status">
      <span class="spinning"><Icon name="refresh" size={20} /></span>
      <p>Loading {direction} payload…</p>
    </div>{/if}
  {#if error}<p class="callout" role="alert">
      {error}{preview ? ' Showing the last loaded snapshot.' : ''}
    </p>{/if}
  {#if preview}
    <div class="chip-row" aria-label="Capture of this body">
      {#each chips as chip}<span class="chip {chip.tone}" title={chip.title}>{chip.label}</span
        >{/each}{#if mode === 'readable' && hiddenThinking}<span
          class="chip thinking-chip"
          title="The model thought before answering; the provider sent only a signature, not the text"
          >Thinking hidden</span
        >{/if}
    </div>
    {#if open}<p class="callout">
        {following
          ? 'Streaming captured output. Totals arrive when recording ends.'
          : 'Recording in progress. Enable Follow live to watch output arrive.'}
      </p>
    {:else if preview.ending === 'unfinished'}<p class="callout">
        The body never finished. Only the captured bytes are shown; missing bytes are not
        reconstructed.
      </p>{/if}
    {#if preview.text_end === 'split_character'}<p class="fine-print">
        Captured text ends mid-character. Readable shows only complete UTF-8 characters; Raw retains
        every captured byte.
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
      <div class="card-list" bind:this={cardList}>
        {#each cards as { card, index } (index)}<details
            class="body-card {card.kind}"
            open={!folded(card)}
          >
            <summary
              title={card.kind === 'tool_call'
                ? 'Requested by the model. Whether it ran is up to your client.'
                : undefined}
            >
              <span class="card-icon"><Icon name={icon(card.kind)} size={13} /></span><span
                class="card-label">{card.label}</span
              >{#if card.detail}<code class="card-detail">{card.detail}</code>{/if}
              {#if folded(card)}<span class="card-meta">{card.text.split('\n').length} lines</span
                >{/if}
            </summary>
            {#if card.text}
              <div class="card-body" class:clamped={long(card) && !expanded.has(index)}>
                {#if card.format === 'prose'}<Markdown
                    text={card.text}
                  />{:else if toolFields(card)}<dl class="tool-fields">
                    {#each toolFields(card) ?? [] as [key, value]}<dt>{key}</dt>
                      <dd><pre>{value}</pre></dd>{/each}
                  </dl>{:else}<pre>{card.text}</pre>{/if}
              </div>
              {#if long(card)}<button
                  class="text-button card-more"
                  onclick={() => {
                    const next = new Set(expanded);
                    if (next.has(index)) next.delete(index);
                    else next.add(index);
                    expanded = next;
                  }}>{expanded.has(index) ? 'Show less' : 'Show all'}</button
                >{/if}
            {/if}
          </details>{/each}
      </div>
    {:else if !preview.captured_bytes}<p class="fine-print">No body bytes available.</p>
    {:else}<p class="fine-print">
        No readable text: the body is binary, or compressed with an encoding the preview cannot
        decode. Raw shows the captured bytes.
      </p>{/if}
  {:else if !busy && !error}<p class="fine-print">
      Load a body to inspect messages, tool arguments, and results. Content stays in this view and
      is not saved in browser storage.
    </p>{/if}
</section>
