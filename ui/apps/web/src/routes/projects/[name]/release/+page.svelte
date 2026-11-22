<!--
  A project's release, as the way a change moves: its open pull requests
  into main, main onto the canary, the canary to production, and what
  promotion decides. Each pull request's chip says how it stands, its
  checks, or its preview. Under the flow, the figures that matter; then the
  canary's checks, the pages it captured (Ren's canary captures them), the
  factory's commit previews (Ren's alone), and the work production shipped
  and the work on main waiting for it. Promote asks the factory's next
  pass to install the canary's build (another project promotes with
  `ren project promote`).
-->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import Icon from '#lib/Icon.svelte';
  import Modal from '#lib/layer/Modal.svelte';
  import Segmented, { type SegmentedOption } from '#lib/Segmented.svelte';
  import { toasts } from '#lib/layer/toasts.svelte.ts';
  import { invalidateAll } from '$app/navigation';
  import { untrack } from 'svelte';
  import { page } from '$app/state';
  import {
    Previewing,
    Promotion,
    PullChips,
    PullWatch,
    ReleaseApi,
    ReleaseFigures,
    ReleasePage,
    type PullMode,
  } from '#lib/release.ts';
  import Glyph from '#lib/Glyph.svelte';
  import { Work } from '#lib/work.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const view = $derived(
    data.body?.release ? new ReleasePage(data.body.release, data.body.promotion_requested) : null,
  );
  const shots = $derived(data.body?.shots ?? []);
  const shipped = $derived(view ? view.shipped(data.work) : []);
  const waiting = $derived(ReleasePage.waiting(data.work));
  const pulls = $derived(PullWatch.pulls(data.pulls));
  const watchNote = $derived(PullWatch.note(data.pulls));
  const repository = $derived(PullWatch.repository(data.pulls));
  const previews = $derived(data.previews.previews);
  const previewRows = $derived(Previewing.rows(previews, pulls, page.url.hostname));
  const previewed = $derived(data.project.factory === 'ren');
  const figures = $derived(view ? ReleaseFigures.of(view, pulls) : []);
  const control = $derived(!!page.data.runsControl);

  let mode = $state<PullMode>('status');
  const modes: SegmentedOption<PullMode>[] = PullChips.modes.map(({ mode, label }) => ({
    value: mode,
    label,
  }));

  /** A pull request's row on the flow is 48px with 12px between rows. */
  const ROW = 60;
  const flowHeight = $derived(Math.max(1, pulls.length) * ROW - 12);

  let asking = $state(false);
  /** The canary build the operator is asked to confirm promoting. */
  let confirming = $state<string | null>(null);
  let wanted = $state('');

  async function act(action: () => Promise<void>, done: string, failed: string) {
    asking = true;
    try {
      await action();
      toasts.show({ mark: 'passed', text: done, detail: 'The factory acts on its next pass.' });
      await invalidateAll();
    } catch (failure) {
      toasts.show({
        mark: 'failed',
        text: failed,
        detail: failure instanceof Error ? failure.message : null,
      });
    } finally {
      asking = false;
    }
  }
  const api = new ReleaseApi(fetch);
  function promote(commit: string) {
    confirming = null;
    void act(
      () => api.promote(commit),
      `Promotion of ${ReleasePage.short(commit)} asked`,
      'The promotion was not asked for',
    );
  }
  function ask(commit: string) {
    void act(
      () => api.ask(commit),
      `Preview of ${ReleasePage.short(commit)} asked`,
      'The preview was not asked for',
    ).then(() => (wanted = ''));
  }
  function withdraw(commit: string) {
    void act(
      () => api.withdraw(commit),
      `Preview of ${ReleasePage.short(commit)} withdrawn`,
      'The preview was not withdrawn',
    );
  }

  // Arriving from ⌘K's Promote: ask, as the button would, once the canary
  // it names can be promoted; the modal still decides.
  let prompted: string | null = null;
  $effect(() => {
    const asked = page.url.searchParams.get('promote');
    const canary = view?.release.canary?.commit ?? null;
    if (!asked || asked === prompted || asked !== canary || !view?.promotable) return;
    if (!control) return;
    prompted = asked;
    untrack(() => (confirming = asked));
  });
</script>

<Toolbar />
<section class="release" aria-label="Release">
  {#if data.failed}
    <p class="error" role="alert">{data.failed}</p>
  {:else if !view}
    <p class="empty">
      {#if data.project.factory === 'ren'}
        Nothing checked yet. A canary pass (<code>cargo xtask canary run</code>, or on its timer
        with <code>cargo xtask canary enable</code>) builds main, checks it on a copy of
        production's history, and writes what it found here.
      {:else}
        Nothing checked yet. A canary pass (<code
          >ren project canary --project {data.project.name}</code
        >) builds main, runs the project's checks, and writes what it found here.
      {/if}
    </p>
  {:else}
    {@const release = view.release}
    {@const promotion = Promotion.of(release.decision)}
    <section class="flow-panel" aria-labelledby="flow-title">
      <header class="flow-head">
        <span class="flow-icon" aria-hidden="true"><Icon name="branch" size={14} /></span>
        <div>
          <h1 id="flow-title">Where a change moves</h1>
          <p class="meta">
            {repository ?? data.project.name} · {pulls.length} open pull request{pulls.length === 1
              ? ''
              : 's'} → main → canary → production
          </p>
        </div>
        {#if pulls.length}<span class="flow-modes"
            ><Segmented label="What the chips say" options={modes} bind:value={mode} /></span
          >{/if}
      </header>

      <div class="flow">
        <div class="pulls">
          <div class="pull-nodes">
            {#each pulls as pull (pull.number)}
              {@const chip = PullChips.of(pull, mode, previews)}
              <a
                class="node pull"
                class:bad={chip.mark === 'failed'}
                href={pull.url}
                target="_blank"
                rel="noopener"
                title={pull.title}
              >
                <Glyph mark={chip.mark} label={chip.label} />
                <span class="meta">#{pull.number}</span>
                <span class="pull-title">{pull.title}</span>
              </a>
            {:else}
              <div class="node pull none">
                <span class="meta">{watchNote ?? 'No open pull requests'}</span>
              </div>
            {/each}
          </div>
          <svg
            class="edges"
            viewBox="0 0 64 {flowHeight}"
            preserveAspectRatio="none"
            aria-hidden="true"
          >
            {#each pulls.length ? pulls : [null] as pull, index (index)}
              {@const y = 24 + index * ROW}
              <path
                class:bad={pull !== null && PullChips.of(pull, mode, previews).mark === 'failed'}
                d="M 0 {y} C 32 {y}, 32 {flowHeight / 2}, 64 {flowHeight / 2}"
              />
            {/each}
          </svg>
        </div>

        <div class="node stage">
          <span class="cap">Main</span>
          <span class="commit">{ReleasePage.short(release.main)}</span>
          <span class="meta"
            >{release.behind === null
              ? 'production is not on it'
              : release.behind === 0
                ? 'production runs it'
                : `${release.behind} ahead of production`}</span
          >
        </div>
        <span class="link" aria-hidden="true"></span>
        <div
          class="node stage canary"
          class:passed={release.canary?.passed === true}
          class:failed={release.canary?.passed === false}
        >
          <span class="cap">Canary</span>
          <span class="commit">{ReleasePage.short(release.canary?.commit ?? null)}</span>
          {#if release.canary}
            {@const checks = release.canary.checks}
            <span class="blocks" aria-hidden="true"
              >{#each checks as check, index (index)}<span class:failed={!check.passed}
                ></span>{/each}</span
            >
            <span class="meta figure"
              ><Glyph
                mark={release.canary.passed ? 'passed' : 'failed'}
                label={release.canary.passed ? 'Passed' : 'Failed'}
              />{checks.filter((check) => check.passed)
                .length}/{checks.length}{#if shots.length}<span>· {shots.length} pages</span
                >{/if}</span
            >
          {:else}<span class="meta">none checked</span>{/if}
        </div>
        <span class="link" class:ahead={view.promotable} aria-hidden="true"></span>
        <div class="node stage production">
          <span class="cap">Production</span>
          <span class="commit">{ReleasePage.short(view.production)}</span>
          {#if release.behind}<span
              class="behind mono"
              title="Production is {view.behind}"
              aria-label="Production is {view.behind}">↓{release.behind}</span
            >{:else}<span class="meta">{view.behind}</span>{/if}
        </div>
        <span class="link" aria-hidden="true"></span>
        <div class="node stage decision">
          <span class="cap">Decision</span>
          <span class="decided"
            ><Glyph mark={promotion.mark} label={promotion.sentence} />{promotion.word}</span
          >
          {#if view.asked}
            <span class="meta">Asked: the factory's next pass installs it.</span>
          {:else if view.promotable && release.canary && data.project.factory !== 'ren'}
            <span class="meta mono">ren project promote --project {data.project.name}</span>
          {:else if view.promotable && release.canary}
            {@const commit = release.canary.commit}
            <button
              class="button primary small"
              disabled={asking || !control}
              title={control
                ? 'The factory installs it on its next pass, with the install’s steps'
                : 'This UI only reads: give it the control token to promote'}
              onclick={() => (confirming = commit)}>Promote {commit.slice(0, 7)}</button
            >
          {/if}
        </div>
      </div>

      <div class="figures">
        {#each figures as item (item.label)}
          <div>
            <span class="cap">{item.label}</span>
            <span class="figure-value"
              >{#if item.mark}<Glyph mark={item.mark} />{/if}{item.value}</span
            >
          </div>
        {/each}
      </div>
    </section>

    <div class="panels">
      {#if release.canary}
        {@const checks = release.canary.checks}
        <section class="panel checks" aria-labelledby="checks-title">
          <header class="panel-head">
            <h2 id="checks-title" class="cap">Canary checks · {checks.length}</h2>
            <Glyph
              mark={release.canary.passed ? 'passed' : 'failed'}
              label={release.canary.passed ? 'All passed' : 'Failed'}
            />
          </header>
          {#each checks as check, index (index)}
            <div class="row check" class:failed={!check.passed}>
              <Glyph
                mark={check.passed ? 'passed' : 'failed'}
                label={check.passed ? 'Passed' : 'Failed'}
              />
              <span class="check-name">
                <span class="mono">{check.name ?? check.check}</span>
                {#if check.detail}<span class="meta">{check.detail}</span>{/if}
              </span>
              <span class="meta">{ReleaseFigures.source(check)}</span>
            </div>
          {/each}
        </section>
      {/if}

      {#if shots.length}
        <section class="panel shots-panel" aria-labelledby="pages-title">
          <header class="panel-head">
            <h2 id="pages-title" class="cap">Pages on the canary · {shots.length}</h2>
          </header>
          <div class="shots">
            {#each shots as shot (shot)}
              <a
                class="shot"
                href={`/api/projects/${data.project.name}/release/shots/${shot}`}
                target="_blank"
                rel="noopener"
              >
                <img
                  src={`/api/projects/${data.project.name}/release/shots/${shot}`}
                  alt={`The ${shot} page on the canary`}
                  loading="lazy"
                />
                <span class="mono">{shot}</span>
              </a>
            {/each}
          </div>
        </section>
      {/if}

      {#if previewed}
        <section class="panel previews" aria-labelledby="previews-title">
          <header class="panel-head">
            <h2 id="previews-title" class="cap">
              Commit previews{#if data.previews.requested.length}{' · '}{data.previews.requested
                  .length} asked{/if}
            </h2>
          </header>
          {#each previewRows as row (row.slot)}
            <div class="row preview">
              <span class="meta">s{row.slot}</span>
              <Glyph mark={row.chip.mark} label={row.chip.label} />
              <span class="preview-of">{row.of}</span>
              {#if row.action.kind === 'open'}<a
                  class="button small"
                  href={row.action.href}
                  target="_blank"
                  rel="noopener">Open</a
                >{:else if row.action.kind === 'stop'}{@const commit = row.action.commit}<button
                  class="button small"
                  disabled={asking || !control}
                  onclick={() => withdraw(commit)}>Stop</button
                >{:else if row.action.kind === 'retry'}{@const commit = row.action.commit}<button
                  class="button small"
                  disabled={asking || !control}
                  onclick={() => ask(commit)}>Retry</button
                >{:else}<span></span>{/if}
            </div>
          {:else}
            <p class="none meta">No previews running.</p>
          {/each}
          <form
            class="ask"
            onsubmit={(event) => {
              event.preventDefault();
              if (wanted.trim()) ask(wanted.trim());
            }}
          >
            <input
              class="mono"
              aria-label="A commit to preview"
              placeholder="commit or branch"
              bind:value={wanted}
              disabled={!control}
            />
            <button
              class="button small"
              type="submit"
              disabled={asking || !control || !wanted.trim()}>Preview</button
            >
          </form>
        </section>
      {/if}
    </div>

    <section class="work" aria-labelledby="work-title">
      <header class="panel-head">
        <h2 id="work-title" class="cap">
          Work · {shipped.length} shipped · {waiting.length} waiting
        </h2>
      </header>
      <div class="work-columns">
        <div>
          <h3 class="cap">On main, waiting for production</h3>
          {#each waiting as item (item.id)}
            <a class="row work-row" href={Work.href(data.project.name, item.id)}>
              <span class="ellipsis">{item.title}</span>
              <span class="meta">{ReleasePage.short(item.landed).slice(0, 7)}</span>
            </a>
          {:else}<p class="none meta">None.</p>{/each}
        </div>
        <div>
          <h3 class="cap">Shipped in production's build</h3>
          {#each shipped as item (item.id)}
            <a class="row work-row shipped" href={Work.href(data.project.name, item.id)}>
              <span class="ellipsis">{item.title}</span>
              <span class="meta"
                >{item.shipped?.kind === 'commit' ? item.shipped.commit.slice(0, 7) : ''}</span
              >
            </a>
          {:else}<p class="none meta">None.</p>{/each}
        </div>
      </div>
    </section>
  {/if}
</section>

{#if confirming && view}
  {@const commit = confirming}
  {@const release = view.release}
  <Modal
    eyebrow="Promote · {data.project.name} production"
    title="Promote {commit.slice(0, 7)} to production?"
    onclose={() => (confirming = null)}
  >
    <p>
      {release.behind === null
        ? 'Production runs a build that is not on main.'
        : `${release.behind} ${release.behind === 1 ? 'commit' : 'commits'} on main that production does not run.`}
    </p>
    {#if waiting.length}
      <ul class="promoting">
        {#each waiting as item (item.id)}<li>{item.title}</li>{/each}
      </ul>
    {/if}
    {#if release.canary}
      <p class="passed-line">
        <Glyph
          mark={release.canary.passed ? 'passed' : 'failed'}
          label={release.canary.passed ? 'Passed' : 'Failed'}
        />Canary {release.canary.passed ? 'passed' : 'failed'}
        {release.canary.checks.filter((c) => c.passed).length}/{release.canary.checks.length}
        · policy {release.policy}
      </p>
    {/if}
    <p class="meta">
      production runs {ReleasePage.short(view.production).slice(0, 7)} · the factory installs it on its
      next pass
    </p>
    {#snippet actions()}
      <button class="button" onclick={() => (confirming = null)}>Cancel</button>
      <button class="button primary" onclick={() => promote(commit)}>Promote</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .release {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    background: var(--line);
    gap: 1px;
  }
  .cap {
    margin: 0;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    font-weight: 400;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .meta {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11px;
  }
  .ellipsis {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .error,
  .empty {
    margin: 0;
    padding: 16px;
    background: var(--surface);
  }
  .error {
    color: var(--error);
  }

  .flow-panel {
    background: var(--surface);
  }
  .flow-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px 0;
  }
  .flow-head h1 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .flow-head p {
    margin: 0;
  }
  .flow-icon {
    display: grid;
    width: 28px;
    height: 28px;
    place-items: center;
    border: 1px dashed var(--control-border);
    color: var(--muted);
  }
  .flow-modes {
    margin-left: auto;
  }
  .flow {
    display: grid;
    grid-template-columns:
      minmax(220px, 300px) minmax(120px, 1fr) 40px minmax(140px, 1fr) 40px
      minmax(140px, 1fr) 40px minmax(150px, 190px);
    align-items: center;
    min-height: 260px;
    margin: 10px 16px 0;
    padding: 16px 12px;
    border: 1px solid var(--line);
    background:
      radial-gradient(var(--line) 1px, transparent 1px) 0 0 / 14px 14px,
      var(--surface-inset);
  }
  .pulls {
    grid-column: 1 / span 1;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 64px;
    margin-right: -1px;
  }
  .flow > .pulls {
    grid-column: 1;
  }
  .pull-nodes {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .edges {
    width: 64px;
    height: 100%;
  }
  .edges path {
    fill: none;
    stroke: var(--control-border);
    stroke-width: 1.5;
    vector-effect: non-scaling-stroke;
  }
  .edges path.bad {
    stroke: var(--error);
  }
  .node {
    display: flex;
    box-sizing: border-box;
    flex-direction: column;
    justify-content: center;
    gap: 1px;
    padding: 6px 10px;
    border: 1px solid var(--line);
    background: var(--surface);
    color: inherit;
    min-width: 0;
  }
  .node.pull {
    flex-direction: row;
    align-items: center;
    gap: 6px;
    height: 48px;
    white-space: nowrap;
  }
  a.node.pull:hover {
    border-color: var(--control-border);
  }
  .node.pull.bad {
    border-color: var(--error);
  }
  .node.pull.none {
    justify-content: flex-start;
  }
  .pull-title {
    overflow: hidden;
    font-size: 12px;
    text-overflow: ellipsis;
  }
  .stage {
    min-height: 76px;
  }
  .stage .cap {
    font-size: 9.5px;
  }
  .commit {
    font-family: var(--mono);
    font-size: 13px;
    font-weight: 500;
  }
  .canary.passed {
    border-color: var(--success);
  }
  .canary.failed {
    border-color: var(--error);
  }
  .production {
    border-color: var(--accent);
    background: var(--accent-soft);
    box-shadow: var(--elev-popover);
  }
  .production .cap {
    color: var(--accent);
  }
  .behind {
    align-self: flex-start;
    margin-top: 3px;
    padding: 0 6px;
    border: 1px solid var(--line);
    color: var(--warning);
    font-size: 10.5px;
  }
  .decided,
  .figure,
  .figure-value {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .decision .button {
    align-self: flex-start;
    margin-top: 4px;
  }
  .blocks {
    display: flex;
    gap: 1px;
    margin: 3px 0 2px;
  }
  .blocks span {
    flex: 1;
    height: 6px;
    background: var(--success);
  }
  .blocks span.failed {
    background: var(--error);
  }
  .link {
    height: 0;
    border-top: 1.5px solid var(--control-border);
  }
  .link.ahead {
    border-top: 2px dashed var(--accent);
  }
  .figures {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 1px;
    margin-top: 12px;
    border-top: 1px solid var(--line);
    background: var(--line);
  }
  .figures > div {
    display: flex;
    flex-direction: column;
    padding: 10px 16px;
    background: var(--surface);
  }
  .figure-value {
    font-family: var(--mono);
    font-size: 14px;
  }

  .panels {
    display: flex;
    flex-wrap: wrap;
    gap: 1px;
  }
  .panel {
    min-width: 0;
    background: var(--surface);
  }
  .checks,
  .previews {
    flex: 1 1 340px;
  }
  .shots-panel {
    flex: 2 1 480px;
  }
  .panel-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px 8px;
  }
  .row {
    display: grid;
    align-items: center;
    column-gap: 10px;
    min-height: 29px;
    padding: 0 16px;
    border-top: 1px solid var(--line-subtle);
    color: inherit;
  }
  a.row:hover {
    background: var(--row-hover);
  }
  .check {
    grid-template-columns: 10px minmax(0, 1fr) auto;
  }
  .check-name {
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 5px 0;
    font-size: 11.5px;
  }
  .check.failed .check-name > .mono {
    color: var(--error);
  }
  .shots {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 1px;
    border-top: 1px solid var(--line);
    background: var(--line);
  }
  .shot {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px;
    background: var(--surface);
    color: inherit;
    font-size: 11px;
  }
  .shot img {
    width: 100%;
    height: 190px;
    border: 1px solid var(--line);
    object-fit: cover;
    object-position: top;
    background: var(--surface-inset);
  }
  .preview {
    grid-template-columns: 20px 10px minmax(0, 1fr) auto;
    min-height: 32px;
  }
  .preview-of {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .none {
    margin: 0;
    padding: 6px 16px 10px;
  }
  .ask {
    display: flex;
    gap: 6px;
    padding: 10px 16px;
  }
  .ask input {
    flex: 1;
    min-width: 0;
    height: 24px;
    padding: 0 8px;
    border: 1px solid var(--control-border);
    background: var(--surface-inset);
    color: var(--ink);
    font-size: 11.5px;
  }

  .work {
    flex: 1;
    background: var(--surface);
  }
  .work-columns {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 1px;
    border-top: 1px solid var(--line);
    background: var(--line);
  }
  .work-columns > div {
    background: var(--surface);
  }
  .work-columns h3 {
    padding: 8px 16px 4px;
  }
  .work-row {
    grid-template-columns: minmax(0, 1fr) 70px;
  }
  .work-row .meta {
    text-align: right;
  }
  .work-row.shipped .ellipsis {
    color: var(--muted);
  }
  .promoting {
    margin: 0 0 8px;
    padding-left: 18px;
  }
  .passed-line {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  @media (max-width: 1100px) {
    .flow {
      grid-template-columns: 1fr;
      gap: 10px;
    }
    .pulls {
      grid-template-columns: 1fr;
    }
    .edges,
    .link {
      display: none;
    }
    .figures {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .work-columns {
      grid-template-columns: 1fr;
    }
  }
</style>
