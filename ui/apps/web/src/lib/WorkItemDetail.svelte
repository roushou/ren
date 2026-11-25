<!--
  One work item in full: where it stands (its session, a preview to compare
  with production, the commit it landed as, the build that shipped it), its
  marks, what was on the page besides, and the brief it makes. The Work
  view shows it beside its list; a project's page in a drawer.
-->
<script lang="ts">
  import Icon from '#lib/Icon.svelte';
  import Glyph from '#lib/Glyph.svelte';
  import CopyButton from '#lib/CopyButton.svelte';
  import Markdown from 'markdown/Markdown.svelte';
  import { Work, type WorkItem } from '#lib/work.ts';
  import type { BriefProject } from '#lib/briefing.ts';
  import type { Pull } from '#lib/api.ts';
  import { PullChips } from '#lib/release.ts';

  let {
    item,
    project,
    ondismiss,
    pulls = [],
    repository = null,
    desk,
  }: {
    item: WorkItem;
    /** Where its session opens on its desk; null before it is sent. */
    desk: string | null;
    /** The open pull requests that name it. */
    pulls?: readonly Pull[];
    /** The repository they are on. */
    repository?: string | null;
    /** Whose page it is about, for its brief. */
    project: BriefProject;
    /** Dismiss it; offered while it is queued or sent. */
    ondismiss: (item: WorkItem) => void;
  } = $props();
  /** What a session is sent for it. */
  const brief = $derived(Work.batch([item], 'worktree', project));
</script>

<header class="detail-head">
  <div class="head-line">
    <h2>{item.title}</h2>
    <span class="actions">
      <CopyButton text={brief} label="Copy brief" />
      {#if item.state === 'queued' || item.state === 'sent'}
        <button class="button small" onclick={() => ondismiss(item)}
          ><Icon name="close" size={11} />Dismiss</button
        >
      {/if}
    </span>
  </div>
  <p class="quiet-line">
    {item.order.marks.length} mark{item.order.marks.length === 1 ? '' : 's'} · made on
    <a class="mono" href={item.order.page.path}>{item.order.page.path}</a> ·
    <span class="mono"
      >{item.order.page.width}×{item.order.page.height}, {item.order.page.theme}</span
    >
    · {new Date(item.created_at).toLocaleString([], {
      weekday: 'short',
      hour: '2-digit',
      minute: '2-digit',
    })}
  </p>
  <ol class="way" aria-label="Its way to production">
    {#each Work.way(item) as step (step.step)}
      <li class="state-{step.mark}">
        <Glyph mark={step.mark} label={true} />
        <span class="way-text">
          <span class="way-label">{step.label}</span>
          {#if step.detail}<span class="quiet-line">{step.detail}</span>{/if}
        </span>
      </li>
    {/each}
  </ol>
</header>

<div class="detail-body">
  <div class="detail-main">
    {#if item.state === 'sent'}
      <section>
        <h3>
          Compare · {item.order.page.path} at {item.order.page.width}×{item.order.page.height}
        </h3>
        <div class="compare">
          <div class="side">
            <span class="side-label"
              ><Glyph mark={Work.previewed(item) ? 'passed' : 'queued'} label={true} />On the branch</span
            >
            {#if Work.previewed(item)}<a
                class="button small"
                href={Work.previewed(item)}
                target="_blank"
                rel="noopener">Open <Icon name="arrow" size={11} /></a
              >{:else}<span
                class="quiet-line"
                title="The agent starts one with `ren deploy HEAD --work` from its worktree"
                >No preview yet</span
              >{/if}
          </div>
          <div class="side">
            <span class="side-label"><Glyph mark="passed" label={true} />In production</span>
            <a class="button small" href={item.order.page.path} target="_blank" rel="noopener"
              >Open <Icon name="arrow" size={11} /></a
            >
          </div>
        </div>
      </section>
    {/if}
    <section>
      <h3>Marks · {item.order.marks.length}</h3>
      <ol class="marks">
        {#each item.order.marks as mark, index (index)}
          {@const scale = Math.min(56 / Math.max(1, mark.width), 36 / Math.max(1, mark.height))}
          <li>
            <span class="mark-number mono">{index + 1}</span>
            <span class="mark-text">
              <span class="note">{mark.note}</span>
              <span class="mono quiet-line">{mark.selector}</span>
              {#if mark.text || mark.source}<span class="quiet-line"
                  >{mark.text ? `“${mark.text}”` : ''}{mark.text && mark.source
                    ? ' · '
                    : ''}{#if mark.source}<span class="mono">{mark.source}</span>{/if}</span
                >{/if}
            </span>
            <span class="mark-size">
              <svg width="56" height="36" aria-hidden="true"
                ><rect
                  class="mark-box"
                  x={(56 - mark.width * scale) / 2}
                  y={(36 - mark.height * scale) / 2}
                  width={Math.max(2, mark.width * scale)}
                  height={Math.max(2, mark.height * scale)}
                /></svg
              >
              <span class="quiet-line mono">{mark.width}×{mark.height}</span>
            </span>
          </li>
        {/each}
      </ol>
    </section>
  </div>

  <aside class="detail-aside">
    <section>
      <h3>Its session</h3>
      {#if desk}<a class="session-link" href={desk}
          ><Icon name="sessions" size={12} />Open its session <Icon name="arrow" size={11} /></a
        >{:else}<p class="quiet-line">Not sent to a session yet.</p>{/if}
    </section>
    {#if pulls.length}
      <section>
        <h3>
          Its pull requests · {pulls.length}{#if repository}<span class="quiet-line repo"
              >{repository}</span
            >{/if}
        </h3>
        <ul class="pulls">
          {#each pulls as pull (pull.number)}
            {@const chip = PullChips.status(pull)}
            {@const checks = PullChips.checks(pull)}
            <li>
              <Glyph mark={chip.mark} label={chip.label} />
              <a href={pull.url} target="_blank" rel="noopener"
                ><span class="quiet-line">#{pull.number}</span> {pull.title}</a
              >
              <span class="quiet-line"><Glyph mark={checks.mark} label={checks.label} /></span>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
    {#if item.order.evidence.console.length}
      <section>
        <h3>Errors on the page · {item.order.evidence.console.length}</h3>
        <ul class="evidence errors mono">
          {#each item.order.evidence.console as line, index (index)}<li>{line}</li>{/each}
        </ul>
      </section>
    {/if}
    {#if item.order.evidence.reads.length}
      <section>
        <h3>Reads behind it · {item.order.evidence.reads.length}</h3>
        <ul class="evidence mono">
          {#each item.order.evidence.reads as read, index (index)}<li>{read}</li>{/each}
        </ul>
      </section>
    {/if}
    <section>
      <details class="brief-fold">
        <summary><h3>Brief</h3></summary>
        <div class="brief"><Markdown text={brief} /></div>
      </details>
    </section>
  </aside>
</div>

<style>
  .repo {
    float: right;
    font-weight: 400;
    text-transform: none;
    letter-spacing: 0;
  }
  .pulls {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .pulls li {
    display: grid;
    grid-template-columns: 12px minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
    min-height: 26px;
  }
  .pulls a {
    overflow: hidden;
    color: inherit;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .detail-head {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 14px 16px 0;
    border-bottom: 1px solid var(--line);
  }
  .head-line {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }
  .actions {
    display: flex;
    gap: 6px;
  }
  h3 {
    display: inline;
    margin: 0;
    color: var(--muted);
    font-size: 10.5px;
    font-weight: 500;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .quiet-line {
    margin: 0;
    color: var(--muted);
    font-size: 11.5px;
    overflow-wrap: anywhere;
  }
  .way {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    margin: 8px 0 0;
    padding: 0;
    list-style: none;
  }
  .way li {
    display: flex;
    gap: 8px;
    padding: 10px 12px 12px 0;
    border-top: 2px solid var(--line);
  }
  .way li.state-passed {
    border-top-color: var(--success);
  }
  .way li.state-running,
  .way li.state-queued {
    border-top-color: var(--accent);
  }
  .way li.state-draft,
  .way li.state-idle {
    opacity: 0.6;
  }
  .way-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .way-label {
    font-size: 12.5px;
    font-weight: 500;
  }
  .detail-body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 340px;
    align-items: start;
  }
  .detail-main {
    min-width: 0;
  }
  .detail-aside {
    border-left: 1px solid var(--line);
  }
  section {
    padding: 12px 16px;
    border-bottom: 1px solid var(--line);
  }
  section > h3 {
    display: block;
    margin-bottom: 8px;
  }
  .compare {
    display: grid;
    grid-template-columns: 1fr 1fr;
    border: 1px solid var(--line);
  }
  .side {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 10px 12px;
  }
  .side + .side {
    border-left: 1px solid var(--line);
  }
  .side-label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
  }
  .marks {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .marks li {
    display: grid;
    grid-template-columns: 24px minmax(0, 1fr) auto;
    gap: 12px;
    padding: 10px 0;
    border-top: 1px solid var(--line);
  }
  .marks li:first-child {
    border-top: 0;
  }
  .mark-number {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border: 1px solid var(--accent);
    color: var(--accent);
    font-size: 11px;
  }
  .mark-text {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .note {
    white-space: pre-wrap;
  }
  .mark-size {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }
  .mark-size svg {
    background: var(--surface-inset);
  }
  .mark-box {
    fill: color-mix(in srgb, var(--accent) 25%, transparent);
    stroke: var(--accent);
    stroke-width: 1;
  }
  .session-link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .evidence {
    margin: 0;
    padding: 0;
    font-size: 11px;
    list-style: none;
    overflow-wrap: anywhere;
  }
  .evidence li + li {
    margin-top: 3px;
  }
  .evidence.errors li {
    color: var(--error);
  }
  .brief-fold summary {
    cursor: pointer;
  }
  .brief {
    margin-top: 8px;
    padding: 8px 12px;
    border: 1px solid var(--line);
    font-size: 12px;
  }
  @media (max-width: 960px) {
    .detail-body {
      grid-template-columns: minmax(0, 1fr);
    }
    .detail-aside {
      border-left: 0;
    }
    .way {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
</style>
