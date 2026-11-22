<!--
  An open session's conversation: the ruler over more than one turn, then
  every turn as one scroll laid out from its end, so it opens on the newest
  entry and follows it unless the operator scrolled up; the prompts on
  their way below. Its steps can be opened one at a time beside it, and
  picked to copy, see their calls, or ask about. `foot` is drawn under it,
  before the open step.
-->
<script lang="ts">
  import { tick, type Snippet } from 'svelte';
  import { SvelteMap, SvelteSet } from 'svelte/reactivity';
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import FloatingBar from '#lib/layer/FloatingBar.svelte';
  import SlideOver from '#lib/layer/SlideOver.svelte';
  import { toasts } from '#lib/layer/toasts.svelte.ts';
  import type { RecordingId } from '#lib/api.ts';
  import type { Viewable } from '#lib/attachments.ts';
  import type { Outgoing } from '#lib/outbox.ts';
  import type { DeskSession, Entry, ItemId, RunId, Transcript } from '#lib/runs.ts';
  import { Steps, TurnLines, type Block, type Step } from '#lib/steps.ts';
  import ConversationEntry from './ConversationEntry.svelte';
  import ConversationOutline from './ConversationOutline.svelte';
  import StepDetail from './StepDetail.svelte';
  import StepGroup from './StepGroup.svelte';
  import TurnRuler from './TurnRuler.svelte';
  import type { ComposerState } from './composer.svelte.ts';

  let {
    session,
    run,
    transcript,
    blocks,
    steps,
    pending,
    readError,
    working,
    recording,
    composer,
    askable,
    onask,
    onview,
    foot,
  }: {
    session: DeskSession;
    /** The run shown: the session's newest process. */
    run: RunId | null;
    transcript: Transcript | null;
    /** Each turn's blocks, and every step of them in order. */
    blocks: Block[][];
    steps: Step[];
    /** The conversation is on its way: its outline is drawn. */
    pending: boolean;
    /** Why it could not be read. */
    readError: string | null;
    /** A turn is working with nothing waiting for an answer. */
    working: boolean;
    /** The session's recording, for the calls around its steps. */
    recording: RecordingId | null;
    composer: ComposerState;
    /** Whether the picked steps can be asked about in the composer. */
    askable: boolean;
    /** Ask about `quote` in the composer. */
    onask: (quote: string) => void;
    /** Open `images` full size, from the one at `start`. */
    onview: (images: readonly Viewable[], start: number) => void;
    foot: Snippet;
  } = $props();

  let conversation = $state<HTMLElement>();
  /** Follow the newest entry unless the operator scrolled up. */
  let following = true;

  /** Follow the newest entry again: a prompt went. */
  export function follow() {
    following = true;
  }

  /** Groups of steps the operator opened, by their first step: the rest
   * stay folded, running or failed alike. */
  const folds = new SvelteMap<string, boolean>();
  /** Each turn at a glance, for the ruler and the outline above the conversation. */
  const turnLines = $derived(TurnLines.of(blocks));
  /** The step open beside the transcript, by its item; another session
   * closes it. */
  let inspecting = $derived.by((): ItemId | null => (void session.session, null));
  const inspected = $derived(
    inspecting === null ? null : (steps.find((step) => step.entry.item === inspecting) ?? null),
  );
  /** Where the open step sits: its turn's number and its place among the
   * turn's steps. */
  const inspectedPlace = $derived(inspecting === null ? null : Steps.place(blocks, inspecting));
  /** Traffic narrowed to the calls around the open step. */
  const inspectedCalls = $derived(inspected ? Steps.calls(recording, [inspected]) : null);
  /** The steps picked to act on together, by item; another session starts
   * with none. */
  const picked = $derived.by(() => (void session.session, new SvelteSet<ItemId>()));
  const pickedSteps = $derived(steps.filter((step) => picked.has(step.entry.item)));
  const pickedCalls = $derived(Steps.calls(recording, pickedSteps));

  async function copyPicked() {
    try {
      await navigator.clipboard.writeText(Steps.text(pickedSteps));
      toasts.show({
        mark: 'passed',
        text: pickedSteps.length === 1 ? 'Copied 1 step' : `Copied ${pickedSteps.length} steps`,
        detail: null,
      });
    } catch {
      toasts.show({ mark: 'failed', text: 'The browser refused the copy', detail: null });
    }
  }

  /** Quote the picked steps in the composer, to ask about them. */
  function askPicked() {
    onask(Steps.quote(pickedSteps));
    picked.clear();
  }

  /** ↑ and ↓ move the open step along the conversation, while one is open. */
  export function stepKey(event: KeyboardEvent) {
    if (inspecting === null) return;
    const at = steps.findIndex((step) => step.entry.item === inspecting);
    const next = steps[at + (event.key === 'ArrowDown' ? 1 : -1)];
    if (at === -1 || !next) return;
    event.preventDefault();
    inspecting = next.entry.item;
  }

  // The conversation scrolls from its end (`column-reverse`): its newest
  // entry is at scrollTop 0, and scrolling up makes scrollTop negative.
  function onScroll() {
    if (!conversation) return;
    following = Math.abs(conversation.scrollTop) < 48;
  }

  $effect(() => {
    // Another run shown starts at its newest entry.
    void run;
    following = true;
  });

  $effect(() => {
    // Keep the newest entry in view as the conversation grows.
    void transcript?.entries;
    if (!following || !conversation) return;
    void tick().then(() => conversation?.scrollTo({ top: 0 }));
  });
</script>

{#snippet entry(line: { kind: 'entry'; entry: Entry } | { kind: 'outgoing'; item: Outgoing })}
  <ConversationEntry
    {line}
    {session}
    chosen={session.session}
    gallery={composer.gallery}
    sending={composer.sending}
    {onview}
    onresend={(item) => composer.resend(item)}
    ontakeback={(item) => composer.takeBack(item)}
  />
{/snippet}

{#if turnLines.length > 1}
  <TurnRuler lines={turnLines} onturn={() => (following = false)} />
{/if}
<!-- Scrolled from its end, so it opens on the newest entry from the
       first paint, before the page's scripts run. -->
<div class="conversation-frame">
  <div
    class="conversation from-end"
    bind:this={conversation}
    onscroll={onScroll}
    aria-busy={pending}
  >
    <div class="conversation-body">
      {#if pending}
        <ConversationOutline />
      {:else}
        {#if readError}<p class="entry notice-entry danger">{readError}</p>{/if}
        {#if transcript?.earlier}<p class="quiet-empty">
            Earlier turns are not shown: the conversation is longer than one read.
          </p>{/if}
        <!-- The whole conversation, every turn open, as one scroll. -->
        {#each blocks as turn, index (index)}
          <section class="turn" id="turn-{index}" aria-label="Turn {index + 1}">
            {#each turn as block, at (at)}
              {#if block.kind === 'steps'}
                <StepGroup
                  steps={block.steps}
                  state={block.state}
                  open={folds.get(block.key) ?? false}
                  ontoggle={(open) => folds.set(block.key, open)}
                  onopen={(step) =>
                    (inspecting = inspecting === step.entry.item ? null : step.entry.item)}
                  chosen={inspecting}
                  {picked}
                  onpick={(step, on) =>
                    on ? picked.add(step.entry.item) : picked.delete(step.entry.item)}
                />
              {:else}
                {@render entry({ kind: 'entry', entry: block.entry })}
              {/if}
            {/each}
          </section>
        {/each}
      {/if}
      {#each composer.outgoing as item (item.key)}{@render entry({ kind: 'outgoing', item })}{/each}
      {#if working}<div class="entry tool muted working">
          <Glyph mark="responding" /> Working…
        </div>{/if}
    </div>
  </div>
  {#if pickedSteps.length}
    <FloatingBar label="Picked steps">
      <span class="picked-count"
        ><strong>{pickedSteps.length === 1 ? '1 step' : `${pickedSteps.length} steps`}</strong> picked</span
      >
      <button class="button small" onclick={() => void copyPicked()}>Copy</button>
      {#if pickedCalls}<a class="button small" href={pickedCalls}>Their calls in Traffic</a>{/if}
      {#if askable}<button class="button small primary" onclick={askPicked}
          >Ask about {pickedSteps.length === 1 ? 'it' : 'them'}</button
        >{/if}
      <button class="icon-button" aria-label="Clear the picks" onclick={() => picked.clear()}
        ><Icon name="close" size={12} /></button
      >
    </FloatingBar>
  {/if}
</div>
{@render foot()}
{#if inspected}
  <SlideOver label="Step details" pin="step" onclose={() => (inspecting = null)}>
    <StepDetail step={inspected} place={inspectedPlace} calls={inspectedCalls} />
  </SlideOver>
{/if}

<style>
  /* Holds the conversation and what floats over its foot. */
  .conversation-frame {
    position: relative;
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }
  .conversation {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .picked-count {
    color: var(--muted);
  }
  .picked-count strong {
    color: var(--ink);
    font-weight: 600;
  }
  /* Laid out from its end, so the browser opens it scrolled to the newest
     entry, and keeps it there as entries are added while it is. */
  .conversation.from-end {
    display: flex;
    flex-direction: column-reverse;
  }
  /* A conversation shorter than the pane still starts at its top. */
  .conversation-body {
    margin-bottom: auto;
  }
  /* A turn: its prompt, steps, and reply as rows, ruled off from the next. */
  /* Its number sits in a gutter of its own, so turns can be counted down
     the page. */
  .turn {
    position: relative;
    border-bottom: 1px solid var(--line);
  }
  /* The rows drawn here rather than by an entry: why the conversation
     could not be read, and that a turn is working. */
  .entry {
    padding: 0 16px;
    font-size: 13px;
  }
  .entry.tool {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 6px;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--line-subtle);
    font-size: 12px;
    color: var(--ink);
  }
  .entry.tool.muted {
    color: var(--muted);
  }
  .notice-entry {
    margin: 0;
    padding-top: 8px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--line-subtle);
    font-size: 12px;
  }
  .notice-entry.danger {
    color: var(--error);
  }
  .working {
    color: var(--muted);
  }
</style>
