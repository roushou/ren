<!--
  System · GitHub: each project's watched repository with its open pull requests by where
  they stand and GitHub's newest answer, then the pull requests themselves.
-->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import Glyph from '#lib/Glyph.svelte';
  import Figures from '#lib/Figures.svelte';
  import Figure from '#lib/Figure.svelte';
  import { ago } from '#lib/overview.ts';
  import { PullChips } from '#lib/release.ts';
  import { GitHubWatches, type PullGroup } from '#lib/github.ts';
  import type { Mark } from '#lib/marks.ts';
  import type { GitHubData } from './+page.ts';

  let { data }: { data: GitHubData } = $props();
  const rows = $derived(GitHubWatches.rows(data.watches));
  const summary = $derived(GitHubWatches.summary(rows));
  const pulls = $derived(GitHubWatches.pulls(rows));
  const watched = $derived(rows.filter((row) => row.state !== 'off'));
  let only = $state<PullGroup | null>(null);
  const shown = $derived(only ? pulls.filter((row) => row.group === only) : pulls);
  const now = Date.now();

  const labels: Record<PullGroup, string> = {
    ready: 'Ready to merge',
    review: 'Review required',
    held: 'Held',
    failing: 'Failing',
    running: 'Checks running',
    draft: 'Drafts',
    unknown: 'Unknown',
  };
  const marks: Record<PullGroup, Mark> = {
    ready: 'passed',
    review: 'yours',
    held: 'warning',
    failing: 'failed',
    running: 'running',
    draft: 'draft',
    unknown: 'unknown',
  };
  const filters = $derived(
    (['ready', 'failing', 'review', 'draft'] as const).map((standing) => ({
      standing,
      count: pulls.filter((row) => row.group === standing).length,
    })),
  );
</script>

<Toolbar />
<div class="github">
  <section class="main">
    {#if data.failed}
      <p class="error" role="alert">{data.failed}</p>
    {:else}
      <Figures label="GitHub at a glance">
        <Figure
          label="Answering"
          mark={summary.failing ? 'failed' : summary.watched ? 'passed' : 'idle'}
          tone={summary.failing ? 'error' : null}
          >{summary.watched - summary.failing}<span class="muted">/{summary.watched}</span></Figure
        >
        <Figure label="Repositories">{summary.watched}</Figure>
        <Figure label="Open pull requests">{summary.open}</Figure>
        <Figure label="Checks failing" tone={summary.checksFailing ? 'error' : null}
          >{summary.checksFailing}</Figure
        >
        <Figure label="Ready to merge">{summary.ready}</Figure>
      </Figures>

      <section class="block" aria-labelledby="repos-title">
        <header>
          <h2 id="repos-title" class="cap">Watched repositories · {watched.length}</h2>
        </header>
        <div class="repo head cap">
          <span></span><span>Project</span><span>Repository</span><span>Open pull requests</span
          ><span class="num">Open</span><span class="num">Last answer</span>
        </div>
        {#each rows as row (row.project)}
          <div class="repo" class:bad={row.mark === 'failed'}>
            <Glyph
              mark={row.mark}
              label={row.error ?? (row.state === 'off' ? 'Not watched' : row.state)}
            />
            <span>{row.project}</span>
            <span class="mono meta">{row.repository ?? 'not watched'}</span>
            <svg viewBox="0 0 100 6" preserveAspectRatio="none" aria-hidden="true">
              <rect class="track" width="100" height="6" />
              {#if row.pulls.length}
                {#each GitHubWatches.groups as standing, index (standing)}
                  {@const before = GitHubWatches.groups
                    .slice(0, index)
                    .reduce((sum, one) => sum + row.groups[one], 0)}
                  <rect
                    class="part {standing}"
                    x={(before / row.pulls.length) * 100}
                    width={(row.groups[standing] / row.pulls.length) * 100}
                    height="6"
                  />
                {/each}
              {/if}
            </svg>
            <span class="num mono"
              >{row.state === 'seen' || row.pulls.length ? row.pulls.length : '—'}</span
            >
            <span class="num mono meta">{row.at ? ago(row.at, now) : '—'}</span>
          </div>
        {/each}
        <div class="key meta">
          {#each ['ready', 'review', 'held', 'failing', 'running', 'draft'] as const as standing (standing)}
            <span><span class="swatch part {standing}"></span>{labels[standing].toLowerCase()}</span
            >
          {/each}
        </div>
      </section>

      <section class="block" aria-labelledby="pulls-title">
        <header>
          <h2 id="pulls-title" class="cap">Open pull requests</h2>
          <div class="chips">
            <button class="chip filter" class:active={only === null} onclick={() => (only = null)}
              >All {pulls.length}</button
            >
            {#each filters as filter (filter.standing)}
              <button
                class="chip filter"
                class:active={only === filter.standing}
                onclick={() => (only = only === filter.standing ? null : filter.standing)}
                ><Glyph mark={marks[filter.standing]} />{labels[filter.standing]}
                {filter.count}</button
              >
            {/each}
          </div>
        </header>
        <div class="pull head cap">
          <span></span><span>Pull request</span><span>Title</span><span class="num">Checks</span
          ><span class="num">Merge</span>
        </div>
        {#each shown as row (`${row.project}#${row.pull.number}`)}
          {@const status = PullChips.status(row.pull)}
          {@const checks = PullChips.checks(row.pull)}
          <a
            class="pull"
            class:bad={row.group === 'failing'}
            href={row.pull.url}
            target="_blank"
            rel="noopener"
          >
            <Glyph mark={status.mark} label={status.label} />
            <span class="mono meta">{row.project} #{row.pull.number}</span>
            <span class="ellipsis">{row.pull.title}</span>
            <span class="num"><Glyph mark={checks.mark} label={checks.label} /></span>
            <span class="num mono meta">{row.pull.merge}</span>
          </a>
        {:else}
          <p class="meta quiet">No open pull requests.</p>
        {/each}
      </section>
    {/if}
  </section>
</div>

<style>
  .github {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    overflow: auto;
    background: var(--line);
    gap: 1px;
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .error {
    margin: 0;
    padding: 16px;
    background: var(--surface);
    color: var(--error);
  }
  .block {
    background: var(--surface);
  }
  .block:last-child {
    flex: 1;
  }
  .block > header {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px 8px;
  }
  .repo,
  .pull {
    display: grid;
    column-gap: 12px;
    align-items: center;
    min-height: 32px;
    padding: 0 16px;
    border-top: 1px solid var(--line-subtle);
    color: inherit;
  }
  .repo {
    grid-template-columns: 12px 140px 140px minmax(0, 1fr) 50px 90px;
  }
  .pull {
    grid-template-columns: 12px 140px minmax(0, 1fr) 70px 90px;
  }
  .head {
    min-height: 24px;
    font-size: 10px;
  }
  .bad {
    background: color-mix(in srgb, var(--error) 6%, transparent);
  }
  a.pull:hover {
    background: var(--row-hover);
  }
  .repo svg {
    width: 100%;
    height: 8px;
  }
  .track {
    fill: var(--surface-inset);
  }
  .part.ready {
    fill: var(--success);
    background: var(--success);
  }
  .part.review {
    fill: var(--warning);
    background: var(--warning);
  }
  .part.failing {
    fill: var(--error);
    background: var(--error);
  }
  .part.running {
    fill: var(--live);
    background: var(--live);
  }
  .part.held {
    fill: color-mix(in srgb, var(--warning) 50%, transparent);
    background: color-mix(in srgb, var(--warning) 50%, transparent);
  }
  .part.draft,
  .part.unknown {
    fill: var(--muted);
    background: var(--muted);
  }
  .key {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    padding: 8px 16px 12px;
  }
  .swatch {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 5px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .quiet {
    padding: 8px 16px 14px;
  }
</style>
