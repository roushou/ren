<!--
  The Now strip: what the gateway saw lately, newest first, sliding in at
  the left, ongoing waits kept first. As an outline while the page's reads
  are on their way. The recording whose line is under the pointer is
  reported through `onhighlight`, so the page can outline its card.
-->
<script lang="ts">
  import { short, type RecordingId } from '#lib/api.ts';
  import { Feed, type FeedItem } from '#lib/board.ts';
  import Glyph from '#lib/Glyph.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import { Elapsed } from '#lib/time.ts';
  import { prefersReducedMotion } from 'svelte/motion';
  import { slide } from 'svelte/transition';

  type Props =
    /** The page's reads are on their way. */
    | { kind: 'outline' }
    | {
        kind: 'feed';
        feed: FeedItem[];
        /** The time the feed was built at. */
        now: number;
        /** The time its ages read, every second. */
        labelAt: number;
        onhighlight: (recording: RecordingId | null) => void;
      };
  let props: Props = $props();
</script>

{#if props.kind === 'outline'}
  <section class="live-now" aria-label="Now" aria-busy="true">
    <span class="live-now-label"><Glyph mark="idle" />Now</span>
    <div class="live-now-feed" aria-hidden="true">
      {#each ['9em', '12em', '7em', '10em'] as width (width)}
        <span class="live-now-item"><i></i><Skeleton {width} /></span>
      {/each}
    </div>
  </section>
{:else}
  {@const { feed, now, labelAt, onhighlight } = props}
  <section class="live-now" aria-label="Now" aria-live="off">
    <span class="live-now-label"><Glyph mark="responding" />Now</span>
    <div class="live-now-feed">
      {#each feed as item (item.key)}
        <a
          class="live-now-item {item.tone}"
          class:pinned={item.pinned}
          class:fresh={Feed.fresh(item, now)}
          href={item.href}
          title={item.detail ?? undefined}
          in:slide={{ axis: 'x', duration: prefersReducedMotion.current ? 0 : 180 }}
          onpointerenter={() => onhighlight(item.recording?.id ?? null)}
          onpointerleave={() => onhighlight(null)}
          ><i></i>{#if item.recording}<b>{short(item.recording.native_id)}</b>{/if}<span
            >{item.text}</span
          >{#if item.detail}<span class="muted">{item.detail}</span>{/if}{#if !item.pinned}<span
              class="live-now-age">{Elapsed.ago(item.at, labelAt)}</span
            >{/if}</a
        >
      {:else}
        <span class="live-now-item calm"
          ><Glyph mark="idle" label="Nothing in the last {Elapsed.span(Feed.calmMs)}" /><span
            class="mono">0 · {Elapsed.span(Feed.calmMs)}</span
          ></span
        >
      {/each}
    </div>
  </section>
{/if}

<style>
  .live-now {
    display: flex;
    flex: none;
    min-height: 30px;
    overflow: hidden;
    background: var(--surface);
  }
  .live-now-label {
    display: flex;
    flex: none;
    align-items: center;
    gap: 7px;
    width: 150px;
    padding: 0 12px 0 16px;
    border-right: 1px solid var(--line);
    color: var(--muted);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .live-now-feed {
    display: flex;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    mask-image: linear-gradient(to right, #000 85%, transparent);
  }
  .live-now-item {
    display: flex;
    flex: none;
    align-items: center;
    gap: 6px;
    padding: 0 12px;
    border-right: 1px solid var(--line);
    color: var(--muted);
    font: 11.5px var(--mono);
    text-decoration: none;
    white-space: nowrap;
    transition: background-color 1.2s;
  }
  .live-now-item i {
    flex: none;
    width: 7px;
    height: 7px;
    background: var(--muted);
  }
  .live-now-item b {
    color: var(--ink);
    font-weight: 500;
  }
  .live-now-item .muted {
    opacity: 0.75;
  }
  .live-now-age {
    color: var(--muted);
    font-size: 10.5px;
  }
  .live-now-item.success i {
    background: var(--success);
  }
  .live-now-item.warning {
    color: var(--warning);
  }
  .live-now-item.warning i {
    background: var(--warning);
  }
  .live-now-item.error {
    color: var(--error);
  }
  .live-now-item.error i {
    background: var(--error);
  }
  .live-now-item.pinned {
    background: var(--surface-inset);
  }
  .live-now-item.fresh {
    background: var(--row-hover);
    transition: none;
  }
  a.live-now-item:hover {
    background: var(--row-hover);
    transition: none;
  }
  .live-now-item.calm {
    color: var(--muted);
  }
  @media (max-width: 760px) {
    .live-now-label {
      width: 96px;
      padding-left: 12px;
    }
  }
</style>
