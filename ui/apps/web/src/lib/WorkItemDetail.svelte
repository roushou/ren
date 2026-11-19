<!--
  One work item in full: where it stands (its session, a preview to compare
  with production, the commit it landed as, the build that shipped it), its
  marks, what was on the page besides, and the brief it makes. The Work
  view shows it beside its list; a project's page in a drawer.
-->
<script lang="ts">
  import Icon from '#lib/Icon.svelte';
  import Markdown from 'markdown/Markdown.svelte';
  import { Work, type WorkItem } from '#lib/work.ts';
  import type { BriefProject } from '#lib/briefing.ts';

  let {
    item,
    project,
    ondismiss,
  }: {
    item: WorkItem;
    /** Whose page it is about, for its brief. */
    project: BriefProject;
    /** Dismiss it; offered while it is queued or sent. */
    ondismiss: (item: WorkItem) => void;
  } = $props();
</script>

<header class="detail-head">
  <span class="work-state state-{item.state}">{Work.state(item.state)}</span>
  <h2>{item.title}</h2>
  <p class="meta mono">
    <a href={item.order.page.path}>{item.order.page.path}</a> · {item.order.page.width}×{item.order
      .page.height}, {item.order.page.theme}
  </p>
  {#if item.session}
    <p class="meta"><a href={`/sessions?session=${item.session}`}>Its session</a></p>
  {/if}
  {#if item.state === 'sent'}
    <p class="compare">
      {#if Work.previewed(item)}<a
          class="button small"
          href={Work.previewed(item)}
          target="_blank"
          rel="noopener">On the branch</a
        >{:else}<span
          class="muted"
          title="The agent starts one with `cargo xtask preview up --publish` in its workspace"
          >No preview yet</span
        >{/if}
      <a class="button small" href={item.order.page.path} target="_blank" rel="noopener"
        >In production</a
      >
    </p>
  {/if}
  {#if item.landed}<p class="meta mono">Landed as {item.landed.slice(0, 12)}</p>{/if}
  {#if item.shipped?.kind === 'commit'}<p class="meta mono">
      Shipped in production's build of {item.shipped.commit.slice(0, 12)}
    </p>{/if}
  {#if item.state === 'queued' || item.state === 'sent'}
    <button class="button small" onclick={() => ondismiss(item)}
      ><Icon name="close" size={11} />Dismiss</button
    >
  {/if}
</header>
<h3>Marks</h3>
<ol class="marks">
  {#each item.order.marks as mark, index (index)}
    <li>
      <p class="note">{mark.note}</p>
      <p class="meta mono">{mark.selector}{mark.text ? ` · “${mark.text}”` : ''}</p>
    </li>
  {/each}
</ol>
{#if item.order.evidence.console.length}
  <h3>Errors on the page</h3>
  <ul class="evidence mono">
    {#each item.order.evidence.console as line, index (index)}<li>{line}</li>{/each}
  </ul>
{/if}
{#if item.order.evidence.reads.length}
  <h3>Reads behind it</h3>
  <ul class="evidence mono">
    {#each item.order.evidence.reads as read, index (index)}<li>{read}</li>{/each}
  </ul>
{/if}
<h3>Brief</h3>
<div class="brief"><Markdown text={Work.batch([item], 'workspace', project)} /></div>

<style>
  .detail-head {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--line);
  }
  h2 {
    font-size: 15px;
    margin: 0;
  }
  h3 {
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin: 16px 0 6px;
  }
  .meta {
    color: var(--muted);
    font-size: 11px;
    margin: 2px 0 0;
    overflow-wrap: anywhere;
  }
  .compare {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 4px 0;
  }
  .marks {
    margin: 0;
    padding-left: 18px;
  }
  .marks li + li {
    margin-top: 8px;
  }
  .note {
    margin: 0;
    white-space: pre-wrap;
  }
  .evidence {
    margin: 0;
    padding-left: 18px;
    font-size: 11px;
    overflow-wrap: anywhere;
  }
  .brief {
    border: 1px solid var(--line);
    padding: 8px 12px;
    font-size: 12px;
  }
  .brief :global(.markdown p) {
    white-space: pre-wrap;
  }
  .muted {
    color: var(--muted);
  }
</style>
