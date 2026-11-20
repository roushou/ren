<!--
  A project's release: how far production is behind main, and what stands
  between them. The canary's build and each of its checks, the pages it
  captured on a copy of production's history (Ren's canary captures them),
  what the operator's policy decided, and, for Ren, a Promote that asks the
  factory's next pass to install it (another project promotes with
  `ren project promote`). Below, the project's work items production's
  build shipped and those on main waiting for it.
-->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import Icon from '#lib/Icon.svelte';
  import Modal from '#lib/layer/Modal.svelte';
  import { toasts } from '#lib/layer/toasts.svelte.ts';
  import { invalidateAll } from '$app/navigation';
  import { untrack } from 'svelte';
  import { page } from '$app/state';
  import { Promotion, ReleaseLine, ReleaseApi, ReleasePage } from '#lib/release.ts';
  import Glyph from '#lib/Glyph.svelte';
  import { Work } from '#lib/work.ts';
  import type { ReleaseView } from '#lib/api.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const view = $derived(
    data.body?.release ? new ReleasePage(data.body.release, data.body.promotion_requested) : null,
  );
  const shots = $derived(data.body?.shots ?? []);
  const shipped = $derived(view ? view.shipped(data.work) : []);
  const waiting = $derived(ReleasePage.waiting(data.work));
  let asking = $state(false);
  /** The canary build the operator is asked to confirm promoting. */
  let confirming = $state<string | null>(null);

  async function promote(commit: string) {
    confirming = null;
    asking = true;
    try {
      await new ReleaseApi(fetch).promote(commit);
      toasts.show({
        mark: 'passed',
        text: `Promotion of ${ReleasePage.short(commit)} asked`,
        detail: 'The factory installs it on its next pass.',
      });
      await invalidateAll();
    } catch (failure) {
      toasts.show({
        mark: 'failed',
        text: 'The promotion was not asked for',
        detail: failure instanceof Error ? failure.message : null,
      });
    } finally {
      asking = false;
    }
  }

  const decided = (release: ReleaseView) => ReleaseLine.decision(release);
  // Arriving from ⌘K's Promote: ask, as the button would, once the canary
  // it names can be promoted; the modal still decides.
  let prompted: string | null = null;
  $effect(() => {
    const asked = page.url.searchParams.get('promote');
    const canary = view?.release.canary?.commit ?? null;
    if (!asked || asked === prompted || asked !== canary || !view?.promotable) return;
    if (!page.data.runsControl) return;
    prompted = asked;
    untrack(() => (confirming = asked));
  });
</script>

{#snippet scope()}
  <div class="scope-group">
    <strong>Release</strong>
    <span class="muted"
      >{view
        ? `production ${view.behind} · promotion ${view.release.policy}`
        : 'no factory pass yet'}</span
    >
  </div>
{/snippet}

<Toolbar {scope} />

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
    {@const [decision] = decided(release)}
    {@const promotion = Promotion.of(release.decision)}
    <div class="lanes" aria-label="Where a change moves">
      <div class="lane">
        <span class="lane-label">Main</span>
        <span class="lane-commit mono">{ReleasePage.short(release.main)}</span>
        <span class="lane-note">its tip, as the factory last fetched it</span>
      </div>
      <span class="arrow" aria-hidden="true"><Icon name="arrow" size={14} /></span>
      <div class="lane">
        <span class="lane-label">Canary</span>
        <span class="lane-commit mono">{ReleasePage.short(release.canary?.commit ?? null)}</span>
        {#if release.canary}
          {@const checks = release.canary.checks}
          <span class="check-blocks" aria-hidden="true"
            >{#each checks as check, index (index)}<span class:failed={!check.passed}
              ></span>{/each}</span
          >
          <span class="lane-figure mono"
            ><Glyph
              mark={release.canary.passed ? 'passed' : 'failed'}
              label={release.canary.passed ? 'Passed' : 'Failed'}
            />{checks.filter((check) => check.passed).length}/{checks.length}</span
          >
        {:else}<span class="lane-note">none checked</span>{/if}
      </div>
      <span class="arrow" aria-hidden="true"><Icon name="arrow" size={14} /></span>
      <div class="lane">
        <span class="lane-label">Production</span>
        <span class="lane-commit mono">{ReleasePage.short(view.production)}</span>
        {#if release.behind}<span
            class="behind mono"
            title="Production is {view.behind}"
            aria-label="Production is {view.behind}">↓{release.behind}</span
          >{:else}<span class="lane-note">{view.behind}</span>{/if}
      </div>
      <span class="arrow" aria-hidden="true"><Icon name="arrow" size={14} /></span>
      <div class="lane decision">
        <span class="lane-label">Decision · {release.policy}</span>
        <span class="lane-decision" title={decision}
          ><Glyph mark={promotion.mark} label={promotion.sentence} />{promotion.word}</span
        >
        {#if view.asked}
          <span class="lane-note">Asked: the factory's next pass installs it.</span>
        {:else if view.promotable && release.canary && data.project.factory !== 'ren'}
          <span class="lane-note mono">ren project promote --project {data.project.name}</span>
        {:else if view.promotable && release.canary}
          {@const commit = release.canary.commit}
          <button
            class="button primary small"
            disabled={asking || !page.data.runsControl}
            title={page.data.runsControl
              ? 'The factory installs it on its next pass, with the install’s steps'
              : 'This UI only reads: give it the control token to promote'}
            onclick={() => (confirming = commit)}
            >{asking ? 'Asking…' : `Promote ${ReleasePage.short(commit)}…`}</button
          >
        {/if}
      </div>
    </div>

    {#if release.canary}
      <h2>Canary checks</h2>
      <ul class="checks">
        {#each release.canary.checks as check, index (index)}
          <li class:failed={!check.passed}>
            <Icon name={check.passed ? 'check' : 'alert'} size={13} />
            <span class="check-name">{check.name ?? check.check}</span>
            {#if check.detail}<span class="check-detail mono">{check.detail}</span>{/if}
          </li>
        {/each}
      </ul>
    {/if}

    {#if shots.length}
      <h2>Pages on the canary</h2>
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
            <span>{shot}</span>
          </a>
        {/each}
      </div>
    {/if}

    <h2>Work</h2>
    <div class="work-columns">
      <div>
        <h3>Shipped in production's build</h3>
        {#if shipped.length}
          <ul class="work-list">
            {#each shipped as item (item.id)}
              <li>{item.title}</li>
            {/each}
          </ul>
        {:else}<p class="muted">None.</p>{/if}
      </div>
      <div>
        <h3>On main, waiting for production</h3>
        {#if waiting.length}
          <ul class="work-list">
            {#each waiting as item (item.id)}
              <li>
                <a href={Work.href(data.project.name, item.id)}>{item.title}</a>
                <span class="muted mono">{ReleasePage.short(item.landed)}</span>
              </li>
            {/each}
          </ul>
        {:else}<p class="muted">None.</p>{/if}
      </div>
    </div>
  {/if}
</section>

{#if confirming && view}
  {@const commit = confirming}
  {@const release = view.release}
  <Modal
    eyebrow="Release · {data.project.name}"
    title="Promote {ReleasePage.short(commit)} to production?"
    onclose={() => (confirming = null)}
  >
    <p>
      {release.behind === null
        ? 'Production runs a build that is not on main.'
        : `Production is ${release.behind} ${release.behind === 1 ? 'commit' : 'commits'} behind main; this build brings it to the canary's.`}
    </p>
    {#if release.canary}
      <p class="muted">
        The canary passed {release.canary.checks.filter((c) => c.passed).length} of {release.canary
          .checks.length} checks. The policy is {release.policy}.
      </p>
    {/if}
    {#if waiting.length}
      <ul class="promoting">
        {#each waiting as item (item.id)}<li>{item.title}</li>{/each}
      </ul>
    {/if}
    <p class="muted">The factory installs it on its next pass, with the install's steps.</p>
    {#snippet actions()}
      <button class="button" onclick={() => (confirming = null)}>Cancel</button>
      <button class="button primary" onclick={() => void promote(commit)}>Promote</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .release {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 0 0 24px;
  }
  .lanes {
    display: grid;
    grid-template-columns:
      minmax(0, 1fr) 24px minmax(0, 1fr) 24px minmax(0, 1fr) 24px
      minmax(0, 1.4fr);
    align-items: stretch;
    border-bottom: 1px solid var(--line);
  }
  .arrow {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--muted);
  }
  .check-blocks {
    display: flex;
    gap: 2px;
  }
  .check-blocks span {
    flex: 1;
    max-width: 18px;
    height: 6px;
    background: var(--success);
  }
  .check-blocks span.failed {
    background: var(--error);
  }
  .lane-figure,
  .lane-decision {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .behind {
    align-self: flex-start;
    padding: 0 6px;
    border: 1px solid var(--warning);
    border-radius: 2px;
    color: var(--warning);
    font-size: 12px;
  }
  .lane {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 12px 16px;
    min-width: 0;
  }

  .lane-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .lane-commit {
    font-size: 18px;
  }
  .lane-note {
    color: var(--muted);
    font-size: 12px;
  }
  .decision {
    align-items: flex-start;
  }
  .lane-decision {
    font-size: 14px;
  }
  h2 {
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin: 0;
    padding: 16px 16px 8px;
  }
  h3 {
    font-size: 12px;
    font-weight: 600;
    margin: 0 0 6px;
  }
  .checks {
    list-style: none;
    margin: 0;
    padding: 0;
    border-top: 1px solid var(--line);
  }
  .checks li {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 6px 16px;
    border-bottom: 1px solid var(--line);
    color: var(--success);
  }
  .checks li.failed {
    color: var(--error);
  }
  .check-name {
    color: var(--text);
    font-weight: 500;
  }
  .check-detail {
    color: var(--muted);
    font-size: 11px;
    overflow-wrap: anywhere;
  }
  .shots {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    border-top: 1px solid var(--line);
  }
  .shot {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px;
    border-right: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
    color: var(--text);
    text-decoration: none;
    font-size: 12px;
  }
  .shot img {
    width: 100%;
    aspect-ratio: 16 / 10;
    object-fit: cover;
    object-position: top;
    border: 1px solid var(--line);
    background: var(--surface);
  }
  .work-columns {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  .work-columns > div {
    padding: 12px 16px;
    min-width: 0;
  }
  .work-columns > div + div {
    border-left: 1px solid var(--line);
  }
  .work-list {
    margin: 0;
    padding-left: 18px;
  }
  .work-list li + li {
    margin-top: 4px;
  }
  .empty {
    padding: 16px;
  }
  .promoting {
    margin: 0;
    padding-left: 18px;
    font-size: 12.5px;
  }
  .error {
    color: var(--error);
  }
  .muted {
    color: var(--muted);
  }
  @media (max-width: 900px) {
    .lanes,
    .work-columns {
      grid-template-columns: minmax(0, 1fr);
    }
    .work-columns > div + div {
      border-left: none;
      border-top: 1px solid var(--line);
    }
    .arrow {
      transform: rotate(90deg);
      padding: 2px 0;
    }
  }
</style>
