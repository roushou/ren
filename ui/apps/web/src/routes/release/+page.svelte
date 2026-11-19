<!--
  Release: how far production is behind main, and what stands between them.
  The canary's build and each of its checks, the pages it captured on a copy
  of production's history, what the operator's policy decided, and a
  Promote that asks the factory's next pass to install it. Below, the work
  items production's build shipped and those on main waiting for it, and
  each project's release.
-->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import Icon from '#lib/Icon.svelte';
  import { invalidateAll } from '$app/navigation';
  import { page } from '$app/state';
  import { ReleaseLine, ReleaseApi, ReleasePage } from '#lib/release.ts';
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
  let error = $state<string | null>(null);

  async function promote(commit: string) {
    error = null;
    asking = true;
    try {
      await new ReleaseApi(fetch).promote(commit);
      await invalidateAll();
    } catch (failure) {
      error = failure instanceof Error ? failure.message : 'The promotion was not asked for.';
    } finally {
      asking = false;
    }
  }

  const decided = (release: ReleaseView) => ReleaseLine.decision(release);
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
      Nothing checked yet. A canary pass (<code>cargo xtask canary run</code>, or on its timer with
      <code>cargo xtask canary enable</code>) builds main, checks it on a copy of production's
      history, and writes what it found here.
    </p>
  {:else}
    {@const release = view.release}
    {@const [decision, tone] = decided(release)}
    <div class="lanes" aria-label="Builds">
      <div class="lane">
        <span class="lane-label">Production</span>
        <span class="lane-commit mono">{ReleasePage.short(view.production)}</span>
        <span class="lane-note">{view.behind}</span>
      </div>
      <div class="lane">
        <span class="lane-label">Canary</span>
        <span class="lane-commit mono">{ReleasePage.short(release.canary?.commit ?? null)}</span>
        <span
          class="lane-note"
          class:ok={release.canary?.passed}
          class:bad={release.canary && !release.canary.passed}
          >{release.canary ? (release.canary.passed ? 'passed' : 'failed') : 'none checked'}</span
        >
      </div>
      <div class="lane">
        <span class="lane-label">Main</span>
        <span class="lane-commit mono">{ReleasePage.short(release.main)}</span>
        <span class="lane-note">its tip, as the factory last fetched it</span>
      </div>
      <div class="lane decision">
        <span class="lane-label">Promotion · {release.policy}</span>
        <span class="lane-decision tone-{tone}">{decision}</span>
        {#if view.asked}
          <span class="lane-note">Asked: the factory's next pass installs it.</span>
        {:else if view.promotable && release.canary}
          {@const commit = release.canary.commit}
          <button
            class="button primary small"
            disabled={asking || !page.data.runsControl}
            title={page.data.runsControl
              ? 'The factory installs it on its next pass, with the install’s steps'
              : 'This UI only reads: give it the control token to promote'}
            onclick={() => void promote(commit)}
            >{asking ? 'Asking…' : `Promote ${ReleasePage.short(commit)}`}</button
          >
        {/if}
        {#if error}<span class="error" role="alert">{error}</span>{/if}
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
            href={`/api/projects/${data.project}/release/shots/${shot}`}
            target="_blank"
            rel="noopener"
          >
            <img
              src={`/api/projects/${data.project}/release/shots/${shot}`}
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
              <li><a href={`/work?shelf=archived&item=${item.id}`}>{item.title}</a></li>
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
                <a href={`/work?item=${item.id}`}>{item.title}</a>
                <span class="muted mono">{ReleasePage.short(item.landed)}</span>
              </li>
            {/each}
          </ul>
        {:else}<p class="muted">None.</p>{/if}
      </div>
    </div>
  {/if}

  {#if data.projects.length}
    <h2>Projects</h2>
    <table class="projects">
      <thead>
        <tr>
          <th>Project</th>
          <th>Production</th>
          <th>Canary</th>
          <th>Promotion</th>
        </tr>
      </thead>
      <tbody>
        {#each data.projects as project (project.name)}
          {@const line = new ReleasePage(project.release, null)}
          {@const [said, tone] = decided(project.release)}
          <tr>
            <td><strong>{project.name}</strong></td>
            <td class="mono">{ReleasePage.short(line.production)} · {line.behind}</td>
            <td class="mono"
              >{ReleasePage.short(project.release.canary?.commit ?? null)}
              {project.release.canary
                ? project.release.canary.passed
                  ? 'passed'
                  : 'failed'
                : ''}</td
            >
            <td><span class="tone-{tone}">{project.release.policy}: {said}</span></td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</section>

<style>
  .release {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 0 0 24px;
  }
  .lanes {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr)) minmax(0, 1.6fr);
    border-bottom: 1px solid var(--line);
  }
  .lane {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 12px 16px;
    min-width: 0;
  }
  .lane + .lane {
    border-left: 1px solid var(--line);
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
  .lane-note.ok,
  .tone-ok {
    color: var(--success);
  }
  .lane-note.bad,
  .tone-bad {
    color: var(--error);
  }
  .tone-warning {
    color: var(--warning);
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
  .projects {
    width: 100%;
    border-collapse: collapse;
  }
  .projects th {
    text-align: left;
    font-weight: 500;
    color: var(--muted);
    padding: 6px 16px;
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  .projects td {
    padding: 8px 16px;
    border-bottom: 1px solid var(--line);
  }
  .empty {
    padding: 16px;
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
    .lane + .lane,
    .work-columns > div + div {
      border-left: none;
      border-top: 1px solid var(--line);
    }
  }
</style>
