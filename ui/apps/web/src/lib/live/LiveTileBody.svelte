<!--
  A card's body, the part that scrolls: what was asked, then its newest
  lines, or the whole turn with each tool's input and output, then the calls
  not read yet with their replies as they stream. Attributes and
  attachments given to it land on its element. As an outline while the
  page's reads are on their way.
-->
<script lang="ts">
  import { Tile, type ResultMark, type ToolLine } from '#lib/board.ts';
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import { Marks } from '#lib/marks.ts';
  import Skeleton from '#lib/Skeleton.svelte';
  import { Steps } from '#lib/steps.ts';
  import StreamingReply from '#lib/StreamingReply.svelte';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import Markdown from 'markdown/Markdown.svelte';
  import type { HTMLAttributes } from 'svelte/elements';

  type Content =
    /** The page's reads are on their way: the outline of the card in this wall slot. */
    | { kind: 'outline'; slot: number }
    | {
        kind: 'tile';
        tile: Tile;
        /** The newest lines, cut short, or the whole turn (the expanded card). */
        extent: 'newest' | 'turn';
        /** The time elapsed labels read, every second. */
        labelAt: number;
      };
  let { content, ...rest }: { content: Content } & HTMLAttributes<HTMLDivElement> = $props();

  const workbench = getWorkbench();
  /** Text widths for the outline's lines, varied so they read as text. */
  const outlineLines = ['82%', '64%', '91%', '57%', '76%', '69%', '88%', '61%'];
</script>

{#snippet toolLine(tool: ToolLine, count: number, result: ResultMark)}
  <div class="live-tool effect-{tool.effect ?? 'none'}">
    <span class="live-tool-icon"><Icon name={Steps.ICONS[Steps.act(tool.effect)]} size={12} /></span
    >
    <b
      >{tool.name}{#if count > 1}<span class="live-count"> ×{count}</span>{/if}</b
    >
    {#if tool.target}<span class="live-target" title={tool.target}>{tool.target}</span>{/if}
    <span class="live-result"><Glyph mark={Marks.ofTone(result.tone)} label={result.label} /></span>
    {#if tool.output}<span class="live-output" class:error={tool.result.tone === 'error'}
        >⎿ {tool.output}</span
      >{/if}
  </div>
{/snippet}

{#if content.kind === 'outline'}
  {@const slot = content.slot}
  <div class="live-tile-body" {...rest}>
    {#each { length: 3 }, line (line)}
      <p><Skeleton width={outlineLines[(slot + line * 3) % outlineLines.length]} /></p>
    {/each}
  </div>
{:else}
  {@const { tile, extent, labelAt } = content}
  <div class="live-tile-body" class:full={extent === 'turn'} {...rest}>
    {#if tile.prompt}
      <p class="live-sent"><span class="live-who">You</span>{tile.prompt.text}</p>
    {/if}
    {#if extent === 'turn'}
      {#each tile.calls as call (call.key)}
        <p class="live-sep">{Tile.callLabel(call)}</p>
        {#if call.reply}<div class="live-step-reply">
            <Markdown text={call.reply} />
          </div>{/if}
        {#each call.tools as tool (tool.key)}
          {@render toolLine(tool, 1, tool.result)}
          <pre class="live-io" title="Input the model sent">{tool.input.text}{Tile.omitted(
              tool.input,
            )}</pre>
          {#if tool.reported && tool.reported.text.trim()}<pre
              class="live-io reported {tool.result.tone}"
              title="Output the client reported">{tool.reported.text}{Tile.omitted(
                tool.reported,
              )}</pre>{/if}
        {/each}
      {/each}
    {:else}
      {#each tile.lines as line (line.key)}
        {#if line.kind === 'tools'}
          {@render toolLine(line.tool, line.count, line.result)}
        {:else}
          <div class="live-step-reply"><Markdown text={line.text} /></div>
        {/if}
      {/each}
    {/if}
    {#each tile.unread as unread (unread.call.id)}
      {@const streamed = workbench.streamed.get(unread.call.id)}
      {@const status = tile.unreadLine(unread, streamed, labelAt)}
      <!-- Once text streams, it holds the call's place: no status line
           comes and goes around it, and the read reply replaces it in place. -->
      {#if status}
        <p
          class="live-sep live"
          class:open={status.phase !== 'finished'}
          class:thinking={status.phase === 'thinking'}
        >
          {status.text}
        </p>
      {/if}
      {#if streamed}
        {#each streamed.tools as tool, index (index)}
          <div class="live-tool live-streaming-tool">
            <span class="live-tool-icon"><Icon name="tool" size={12} /></span><b>{tool}</b>
          </div>
        {/each}
        {#if streamed.reply}
          <div class="live-step-reply live-streaming" class:writing={unread.call.state === 'open'}>
            <StreamingReply
              text={streamed.reply}
              writing={unread.call.state === 'open'}
              cut={streamed.cut}
            />
          </div>
        {/if}
      {/if}
    {/each}
  </div>
{/if}

<style>
  /* The body reads bottom up, newest last; older lines fade out at the top. */
  .live-tile-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    min-height: 0;
    padding: 6px 12px;
    overflow: hidden;
    color: var(--ink);
    font-size: 12.5px;
    line-height: 1.5;
    text-align: left;
    -webkit-mask-image: linear-gradient(to bottom, transparent 0, #000 38px);
    mask-image: linear-gradient(to bottom, transparent 0, #000 38px);
  }
  .live-tile-body > p {
    flex: none;
    margin: 0 0 6px;
    overflow-wrap: anywhere;
  }
  .live-tile-body > p:last-child {
    margin-bottom: 0;
  }
  .live-who {
    margin-right: 7px;
    color: var(--ink);
    font-family: inherit;
    font-size: 9.5px;
    font-weight: 600;
    letter-spacing: 0.07em;
    text-transform: uppercase;
  }
  .live-sent {
    padding: 4px 7px;
    background: var(--surface-inset);
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11.5px;
  }
  /* A tool call: effect glyph, name, what it touched, result; what came back below. */
  .live-tool {
    flex: none;
    display: grid;
    grid-template-columns: 14px auto minmax(0, 1fr) auto;
    column-gap: 6px;
    align-items: baseline;
    margin: 0 0 4px;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11.5px;
    white-space: nowrap;
  }
  .live-tool-icon {
    display: inline-flex;
    align-self: center;
    color: var(--muted);
  }
  .live-tool.effect-write_file .live-tool-icon {
    color: var(--accent);
  }
  .live-tool b {
    color: var(--ink);
    font-weight: 500;
  }
  .live-count {
    color: var(--muted);
    font-weight: 400;
  }
  .live-output {
    grid-column: 3 / 5;
    overflow: hidden;
    color: var(--muted);
    font-size: 11px;
    opacity: 0.8;
    text-overflow: ellipsis;
  }
  .live-output.error {
    color: var(--error);
    opacity: 1;
  }
  /* A call boundary in the expanded card, and calls not read yet. */
  .live-tile-body > .live-sep {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 6px 0 4px;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    white-space: nowrap;
  }
  .live-sep::after {
    flex: 1;
    height: 1px;
    background: var(--line);
    content: '';
  }
  .live-sep.open {
    color: var(--ink);
  }
  .live-io {
    flex: none;
    max-height: 13.5em;
    margin: 0 0 6px 20px;
    padding: 4px 7px;
    overflow: auto;
    background: var(--surface-inset);
    color: var(--muted);
    font: 11px/1.5 var(--mono);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .live-io.reported {
    border-left: 2px solid var(--line);
  }
  .live-io.reported.error {
    border-left-color: var(--error);
  }
  .live-tile-body.full {
    justify-content: flex-start;
    overflow-y: auto;
    -webkit-mask-image: none;
    mask-image: none;
  }
  .live-result {
    display: inline-flex;
    flex: none;
    margin-left: auto;
  }
  .live-tile-body > .live-step-reply {
    flex: none;
    margin: 0 0 6px;
  }
  /* A reply while its call streams, from the live feed: a caret while it is
     still being written; the read reply replaces it once the call is done. */
  .live-streaming {
    color: var(--muted);
  }
  .live-streaming.writing :global(.markdown > :last-child)::after {
    content: '▍';
    margin-left: 1px;
    color: var(--accent);
    animation: live-caret 1s steps(1) infinite;
  }
  /* The model is thinking: its text is not shown, so the status line pulses. */
  .live-sep.thinking {
    color: var(--accent);
    animation: live-thinking 1.6s ease-in-out infinite;
  }
  @keyframes live-thinking {
    50% {
      opacity: 0.45;
    }
  }
  @keyframes live-caret {
    50% {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .live-streaming.writing :global(.markdown > :last-child)::after,
    .live-sep.thinking {
      animation: none;
    }
  }
  .live-step-reply :global(.markdown) {
    font-size: 12.5px;
    line-height: 1.5;
  }
  .live-step-reply :global(.markdown > *) {
    margin-bottom: 6px;
  }
  .live-step-reply :global(.md-heading) {
    margin: 6px 0 3px;
    font-size: 12.5px;
  }
  .live-target {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
  }
  /* A card reads top down: what was asked, then the newest lines, each reply cut short. */
  .live-tile-body:not(.full) {
    justify-content: flex-start;
  }
  .live-tile-body:not(.full) .live-sent {
    flex: none;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .live-tile-body:not(.full) > .live-step-reply {
    display: -webkit-box;
    overflow: hidden;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 4;
    line-clamp: 4;
  }
</style>
