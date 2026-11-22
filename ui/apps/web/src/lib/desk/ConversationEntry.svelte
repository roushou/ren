<!--
  One entry of a session's conversation, as something said or done: a
  prompt (with the images it sent), a reply, a step, the questions asked
  and their answers, how a turn ended, a notice. Or a prompt still on its
  way, drawn as a prompt with how far it has got.
-->
<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import Markdown from 'markdown/Markdown.svelte';
  import { harnessLabel } from '#lib/activity.ts';
  import { Gallery, Strip, type Viewable } from '#lib/attachments.ts';
  import type { Outgoing } from '#lib/outbox.ts';
  import { Runs, type DeskSession, type Entry, type SessionId } from '#lib/runs.ts';
  import CommandStep from './CommandStep.svelte';
  import Thumbnail from './Thumbnail.svelte';

  let {
    line,
    session,
    chosen,
    gallery,
    sending,
    onview,
    onresend,
    ontakeback,
  }: {
    /** What is drawn: an entry of the conversation, or a prompt on its way. */
    line: { kind: 'entry'; entry: Entry } | { kind: 'outgoing'; item: Outgoing };
    session: DeskSession | null;
    chosen: SessionId | null;
    /** The session's kept attachments, to find a sent image's thumbnail. */
    gallery: Gallery;
    /** One of the session's prompts has no answer from the daemon yet. */
    sending: boolean;
    /** Open `images` full size, from the one at `start`. */
    onview: (images: readonly Viewable[], start: number) => void;
    onresend: (item: Outgoing) => void;
    ontakeback: (item: Outgoing) => void;
  } = $props();
</script>

<!-- A prompt on its way: drawn as a prompt, with how far it has got. -->
{#snippet outgoingView(item: Outgoing)}
  {@const progress = item.progress}
  <div class="entry prompt outgoing {progress.kind}">
    {#if item.images.length > 0}
      {@const viewable = item.images.map((image) => ({
        src: Gallery.url(item.session, image.id),
        read: image.read,
      }))}
      <div class="sent-images">
        {#each viewable as image, view (item.images[view]?.id ?? view)}
          <Thumbnail
            src={image.src}
            read={image.read}
            shown="sent"
            onview={() => onview(viewable, view)}
          />
        {/each}
      </div>
    {/if}
    {#if item.text}<Markdown text={item.text} />{/if}
    {#if progress.kind === 'sending'}
      <div class="outgoing-state" role="status">
        <Glyph mark="responding" />
        {progress.starts === 'client' && session
          ? `Starting ${harnessLabel(session.harness)}…`
          : 'Sending…'}
      </div>
    {:else if progress.kind === 'refused' || progress.kind === 'unknown'}
      <div class="outgoing-state" role="status">
        <Icon name="alert" size={12} />
        <span class="outgoing-reason"
          >{progress.kind === 'refused'
            ? `Not sent: ${progress.reason}`
            : `It may not have been sent: ${progress.reason} Check the conversation before sending it again.`}</span
        >
        <button class="text-button" onclick={() => ontakeback(item)}>Edit</button>
        <button
          class="text-button"
          disabled={sending}
          title={progress.kind === 'unknown' ? 'It may arrive twice' : null}
          onclick={() => onresend(item)}>Send again</button
        >
      </div>
    {/if}
  </div>
{/snippet}

{#snippet entryView(entry: Entry)}
  {#if entry.kind === 'prompt'}
    <div class="entry prompt">
      {#if entry.images.length > 0}
        {@const sent = Gallery.sent(entry.images, (read) => {
          const id = gallery.find(read);
          return id && chosen ? Gallery.url(chosen, id) : null;
        })}
        {@const viewable = sent.flatMap((one) => (one.kind === 'kept' ? [one.image] : []))}
        {@const strip = new Strip(sent)}
        <div class="sent-images">
          {#each strip.shown as one, index (index)}
            {#if one.kind === 'kept'}
              {@const { image, view } = one}
              <Thumbnail
                src={image.src}
                read={image.read}
                shown="sent"
                loading="lazy"
                onview={() => onview(viewable, view)}
              />
            {:else}
              <span class="sent-image named" title={Gallery.describe(one.read)}
                ><Icon name="image" size={12} />{one.read.name}</span
              >
            {/if}
          {/each}
          {#if strip.rest > 0}
            {@const first = strip.shown.filter((one) => one.kind === 'kept').length}
            <button
              type="button"
              class="sent-image more"
              aria-label="View {strip.rest} more images"
              disabled={first >= viewable.length}
              onclick={() => onview(viewable, first)}>+{strip.rest}</button
            >
          {/if}
        </div>
      {/if}
      {#if entry.text}<Markdown text={entry.text} />{/if}
    </div>
  {:else if entry.kind === 'message'}
    <div class="entry reply" class:streaming={!entry.done}>
      <Markdown text={entry.text} />
    </div>
  {:else if entry.kind === 'reasoning'}
    {#if !entry.done}<div class="entry tool muted">
        <Icon name="thought" size={12} /> Thinking…
      </div>{/if}
  {:else if entry.kind === 'command'}
    <CommandStep {entry} />
  {:else if entry.kind === 'file_change'}
    <div class="entry tool">
      <Icon name="pencil" size={12} />
      <span class="tool-target mono" title={entry.paths.join('\n') || undefined}
        >{entry.paths.map((path) => Runs.within(path, session?.workdir ?? null)).join(', ') ||
          'files'}</span
      >
      {#if entry.status !== 'completed'}<Glyph
          mark={entry.status === 'in_progress' ? 'running' : 'failed'}
          label={entry.status === 'in_progress' ? 'Editing' : entry.status}
        />{/if}
    </div>
  {:else if entry.kind === 'tool'}
    <div class="entry tool">
      <Icon name="tool" size={12} /><span class="tool-name">{entry.name}</span>
      {#each entry.effects as effect, index (index)}<span class="tool-target mono"
          >{effect.target}</span
        >{/each}
      {#if !entry.done}<Glyph mark="running" label={true} />{/if}
    </div>
  {:else if entry.kind === 'questions'}
    <div class="entry asked">
      {#each entry.questions as question (question.key)}
        {@const said = entry.answers[question.key]}
        <div class="asked-row">
          <Icon name="sessions" size={12} />
          <span class="asked-text">{question.text}</span>
          {#if said}<span class="asked-answer">{said}</span>
          {:else}<span class="chip {entry.status === 'in_progress' ? 'live' : 'neutral'}"
              >{entry.status === 'in_progress' ? 'waiting for you' : 'not answered'}</span
            >{/if}
        </div>
      {/each}
    </div>
  {:else if entry.kind === 'turn_end'}
    {#if entry.end.kind !== 'completed'}<p
        class="entry notice-entry {entry.end.kind === 'interrupted' ? 'warn' : 'danger'}"
      >
        Turn {entry.end.kind === 'interrupted'
          ? 'interrupted'
          : entry.end.kind === 'failed'
            ? `failed${entry.end.message ? `: ${entry.end.message}` : ''}`
            : `ended: ${entry.end.status}`}
      </p>{/if}
  {:else if entry.kind === 'notice'}
    <p class="entry notice-entry {entry.tone}">{entry.text}</p>
  {/if}
{/snippet}

{#if line.kind === 'entry'}
  {@render entryView(line.entry)}
{:else}
  {@render outgoingView(line.item)}
{/if}

<style>
  .entry.asked {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px 16px;
    border-bottom: 1px solid var(--line-subtle);
    font-size: 12px;
  }
  .asked-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .asked-text {
    color: var(--muted);
  }
  .asked-answer {
    font-weight: 600;
    color: var(--ink);
    overflow-wrap: anywhere;
  }
  .entry {
    padding: 0 16px;
    font-size: 13px;
  }
  /* A prompt reads as Markdown, like a reply, with its line breaks kept
     as written. */
  .entry.prompt :global(.markdown) {
    padding: 10px 0;
  }
  .entry.prompt :global(.markdown p) {
    white-space: pre-wrap;
  }
  /*
   * A prompt's images, as a row of frames that touch: each frame as high
   * as the others and as wide as its image's shape, the image whole inside
   * it. A narrow window wraps the row; each frame draws its own right and
   * bottom rules so wrapped rows stay ruled. Opened full size on click.
   */
  .sent-images {
    display: flex;
    flex-wrap: wrap;
    margin-top: 10px;
    border-top: 1px solid var(--line);
    border-left: 1px solid var(--line);
    width: fit-content;
  }
  .sent-image {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border-right: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  button.sent-image {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    padding: 0;
    border-top: 0;
    border-left: 0;
    background: var(--surface-inset);
    cursor: zoom-in;
  }
  .sent-image.named {
    padding: 6px 8px;
  }
  .sent-image.more {
    width: 72px;
    color: var(--muted);
    font-size: 15px;
    font-variant-numeric: tabular-nums;
  }
  .sent-image.more:hover:not(:disabled) {
    color: var(--ink);
  }
  .entry.prompt {
    background: var(--surface-inset);
    box-shadow: inset 2px 0 var(--role-user);
    border-bottom: 1px solid var(--line);
  }
  /* A prompt on its way: its words as sent, dimmed until the client has them. */
  .entry.prompt.outgoing.sending :global(.markdown),
  .entry.prompt.outgoing.sent :global(.markdown) {
    color: var(--muted);
  }
  .entry.prompt.outgoing.refused,
  .entry.prompt.outgoing.unknown {
    box-shadow: inset 2px 0 var(--error);
  }
  .outgoing-state {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-bottom: 8px;
    color: var(--muted);
    font-size: 12px;
  }
  .entry.prompt.outgoing.refused .outgoing-state,
  .entry.prompt.outgoing.unknown .outgoing-state {
    color: var(--error);
  }
  .outgoing-reason {
    min-width: 0;
  }
  .entry.reply {
    padding-top: 10px;
    padding-bottom: 10px;
    box-shadow: inset 2px 0 var(--role-assistant);
  }
  .entry.reply.streaming :global(.markdown > :last-child::after) {
    content: '▍';
    color: var(--muted);
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
  .tool-name {
    color: var(--role-tool);
  }
  /* A step's state sits at the row's end, however long its target. */
  .entry.tool > .chip {
    flex: none;
    margin-left: auto;
  }
  .tool-target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .notice-entry {
    margin: 0;
    padding-top: 8px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--line-subtle);
    font-size: 12px;
  }
  .notice-entry.warn {
    color: var(--warning);
  }
  .notice-entry.danger {
    color: var(--error);
  }
</style>
