<!--
  Projects (`/projects`): each registered project at a glance, one row each:
  its state's glyph, its sessions running of those open, its work by
  state, where its release stands (main → canary → production, and what
  promotion decided), its calls over the last day, when it was last
  active, and the one thing to do about it. A row opens the project.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import Glyph from '#lib/Glyph.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import Toolbar from '#lib/Toolbar.svelte';
  import ApprovalModal from '#lib/ApprovalModal.svelte';
  import { ApprovalAnswers } from '#lib/approvals.svelte.ts';
  import { ManagementApi } from '#lib/api.ts';
  import { ProjectBoard } from '#lib/projects.ts';
  import { Promotion, ReleasePage } from '#lib/release.ts';
  import { Sessions } from '#lib/runs.ts';
  import { Home } from '#lib/home.ts';
  import { Work } from '#lib/work.ts';
  import { usagePoints } from '#lib/usage.ts';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const boardRead = createArrival(() => data.board);
  /** The board is on its way, and this browser has none yet: outlines. */
  const pending = $derived(boardRead.current.state === 'pending');
  const board = $derived(ProjectBoard.read(boardRead.snapshot?.value ?? null));
  const running = $derived(board.active.reduce((sum, project) => sum + project.running, 0));
  const answers = new ApprovalAnswers();
  const underWay = $derived(Home.underWay(board));
  const inFlight = $derived(board.inFlight);
  /** A work item's glyph: queued, in a session, or landed. */
  const workMark = (state: string) =>
    state === 'queued' ? 'queued' : state === 'sent' ? 'running' : 'passed';
  let now = $state(Date.now());

  /** Each project's calls by hour over the last day, as a sparkline path. */
  let sparks = $state<Record<string, { line: string; area: string }>>({});
  onMount(() => {
    const api = new ManagementApi();
    const since = Date.now() - 86_400_000;
    for (const project of board.active) {
      api
        .usage({ from_ms: since, project: project.name })
        .then((usage) => {
          const points = usagePoints(usage.hours, '24h', Date.now());
          const top = Math.max(1, ...points.map((point) => point.calls));
          const at = points.map(
            (point, index) =>
              `${((index / Math.max(1, points.length - 1)) * 96).toFixed(1)} ${(22 - (point.calls / top) * 20).toFixed(1)}`,
          );
          const line = `M ${at.join(' L ')}`;
          sparks = { ...sparks, [project.name]: { line, area: `${line} L 96 24 L 0 24 Z` } };
        })
        .catch(() => {});
    }
    const ticking = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(ticking);
  });

  /** A click on a project's row opens it, unless it was on one of the
   * row's own links or buttons. */
  function open(event: MouseEvent, href: string) {
    if (event.target instanceof Element && event.target.closest('a, button')) return;
    void goto(href);
  }
</script>

{#snippet scope()}
  <div class="scope-group glance">
    <span class="cap">Projects · {board.active.length}</span>
    <span class="pair" title="Sessions running"><Glyph mark="responding" />{running}</span>
    <span class="pair" title="Waiting for you"><Glyph mark="yours" />{board.approvals.length}</span>
    <span class="pair" title="Canary ready to promote"
      ><Glyph mark="passed" />{board.toPromote.length}</span
    >
  </div>
{/snippet}

<Toolbar {scope} />

<section class="projects-view" aria-label="Projects">
  {#if boardRead.snapshot?.value?.failed}
    <p class="error" role="alert">{boardRead.snapshot.value.failed}</p>
  {:else if pending}
    <div class="table" aria-hidden="true">
      {#each ['9em', '7em', '11em'] as width, index (index)}
        <div class="row">
          <span></span><Skeleton {width} /><Skeleton width="3em" /><Skeleton width="5em" /><Skeleton
            width="8em"
          /><Skeleton width="6em" /><Skeleton width="3em" /><span></span>
        </div>
      {/each}
    </div>
  {:else if !board.projects.length}
    <p class="empty">No project is registered.</p>
  {:else}
    <div class="table" role="table" aria-label="Projects">
      <div class="row head" role="row">
        <span></span><span class="cap">Project</span><span class="cap">Sessions</span><span
          class="cap">Work</span
        ><span class="cap">Release</span><span class="cap">Last 24 h</span><span class="cap right"
          >Active</span
        ><span></span>
      </div>
      {#each board.active as project (project.id)}
        {@const glance = board.glance(project)}
        {@const href = ProjectBoard.href(project.name)}
        {@const release = project.release}
        {@const checks = release?.canary?.checks ?? []}
        {@const promotion = release ? Promotion.of(release.decision) : null}
        <!-- The row is a larger target for the project's own link, which the
             keyboard reaches. -->
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div
          class="row"
          role="row"
          tabindex="-1"
          title={project.repository}
          onclick={(event) => open(event, href)}
        >
          <Glyph mark={glance.mark} label={glance.state} />
          <span class="name-cell">
            <a class="name" {href}>{project.name}</a>
            {#if project.factory === 'ren'}<span class="tag mono" title="Ren builds itself here"
                >self</span
              >{/if}
          </span>
          <a class="pairs" {href} title="Running of open sessions">
            <span class="pair"
              ><Glyph
                mark={project.running ? 'responding' : 'idle'}
                label="Running"
              />{project.running}</span
            ><span class="pair muted">/ {project.sessions}</span>
          </a>
          <span class="pairs">
            <span class="pair" title="Queued to send"
              ><Glyph mark="queued" />{project.work.queued}</span
            >
            <span class="pair" title="Sent to a session"
              ><Glyph mark="client" />{project.work.sent}</span
            >
            <span class="pair" title="Landed on main"
              ><Glyph mark="passed" />{project.work.landed}</span
            >
          </span>
          {#if release}
            <a class="release" href={ProjectBoard.releaseHref(project)}>
              <span class="pipe">
                <span class="stage">main</span><span class="link"></span>
                <span
                  class="stage"
                  class:ok={release.canary?.passed}
                  class:bad={release.canary && !release.canary.passed}
                  >{#if release.canary}<Glyph
                      mark={release.canary.passed ? 'passed' : 'failed'}
                      label={release.canary.passed ? 'Canary passed' : 'Canary failed'}
                    />{/if}canary {checks.length
                    ? `${checks.filter((check) => check.passed).length}/${checks.length}`
                    : ''}</span
                ><span class="link"></span>
                <span
                  class="stage"
                  class:lag={!!release.behind}
                  title="Production: {new ReleasePage(release, null).behind}"
                  >prod{release.behind ? ` ↓${release.behind}` : ''}</span
                >
              </span>
              {#if promotion}<span class="decision"
                  ><Glyph mark={promotion.mark} label={promotion.sentence} />{promotion.word}</span
                >{/if}
            </a>
          {:else}<span class="muted">No canary yet</span>{/if}
          <svg
            class="spark"
            viewBox="0 0 96 24"
            preserveAspectRatio="none"
            aria-label="Calls by hour over the last day"
            ><line
              class="spark-base"
              x1="0"
              y1="23.5"
              x2="96"
              y2="23.5"
            />{#if sparks[project.name]}<path
                class="spark-area"
                d={sparks[project.name]?.area}
              /><path class="spark-line" d={sparks[project.name]?.line} />{/if}</svg
          >
          <span class="muted mono right"
            >{glance.active ? Sessions.age(glance.active, now) : '—'}</span
          >
          <span class="right">
            {#if glance.action?.kind === 'review'}{@const waiting = glance.action.approval}<button
                class="button small primary"
                onclick={() => answers.review(waiting)}>Review…</button
              >{:else if glance.action?.kind === 'promote' && release?.canary}<a
                class="button small"
                href="{ProjectBoard.releaseHref(project)}?promote={encodeURIComponent(
                  release.canary.commit,
                )}">Promote…</a
              >{:else if glance.action?.kind === 'release'}<a
                class="button small"
                href={ProjectBoard.releaseHref(project)}>Release</a
              >{/if}
          </span>
        </div>
      {/each}
      {#if board.sessions.length}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div class="row" role="row" tabindex="-1" onclick={(event) => open(event, '/scratch')}>
          <Glyph mark="idle" label="Sessions that work for no project" />
          <span class="name-cell"><a class="name" href="/scratch">Scratch</a></span>
          <span class="pairs"><span class="pair muted">{board.scratch} open</span></span>
          <span class="muted mono">no work items</span>
          <span class="muted mono">no release</span>
          <span></span><span></span><span></span>
        </div>
      {/if}
    </div>
    <div class="below">
      <section class="part" aria-labelledby="underway-title">
        <header>
          <h2 id="underway-title" class="cap">Sessions under way · {underWay.length}</h2>
        </header>
        {#each underWay as row (row.session)}
          <a class="item" href={row.href}>
            <Glyph mark={row.mark} label={row.doing} />
            <span class="item-title">{row.title}</span>
            <span class="chip mono">{row.project ?? 'Scratch'}</span>
            <span class="muted mono">{row.since ? Sessions.age(row.since, now) : ''}</span>
          </a>
        {:else}<p class="quiet muted">No session is running.</p>{/each}
      </section>
      <section class="part" aria-labelledby="inflight-title">
        <header><h2 id="inflight-title" class="cap">Work in flight · {inFlight.length}</h2></header>
        {#each inFlight as item (item.id)}
          {@const home = board.homeOf(item)}
          <a class="item" href={home ? Work.href(home, item.id) : '/projects'}>
            <Glyph mark={workMark(item.state)} label={Work.state(item.state)} />
            <span class="item-title">{item.title}</span>
            {#if home}<span class="chip mono">{home}</span>{/if}
            <span class="muted mono">{Sessions.age(item.updated_at, now)}</span>
          </a>
        {:else}<p class="quiet muted">No work is on its way.</p>{/each}
      </section>
    </div>
    {#if board.archived.length}
      <p class="archived muted">
        Archived · {board.archived.length}
      </p>
    {/if}
  {/if}
</section>
<ApprovalModal {answers} />

<style>
  .below {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 1px;
    margin-top: 24px;
    border-top: 1px solid var(--line);
    background: var(--line);
  }
  .part {
    min-width: 0;
    background: var(--surface);
  }
  .part header {
    padding: 10px 12px;
    border-bottom: 1px solid var(--line);
  }
  .part .cap {
    margin: 0;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    font-weight: 400;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .item {
    display: grid;
    grid-template-columns: 12px minmax(0, 1fr) auto 48px;
    column-gap: 10px;
    align-items: center;
    height: 30px;
    padding: 0 12px;
    border-bottom: 1px solid var(--line-subtle);
    color: inherit;
  }
  .item:hover {
    background: var(--row-hover);
  }
  .item-title {
    overflow: hidden;
    font-weight: 500;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .item .mono:last-child {
    text-align: right;
    font-size: 11px;
  }
  .quiet {
    margin: 0;
    padding: 10px 12px;
  }
  @media (max-width: 900px) {
    .below {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .projects-view {
    flex: 1;
    min-height: 0;
    overflow: auto;
    background: var(--canvas);
  }
  .glance {
    gap: 14px;
  }
  .cap {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .pair {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-family: var(--mono);
    font-size: 11.5px;
  }
  .pairs {
    display: flex;
    align-items: center;
    gap: 12px;
    color: var(--ink);
    text-decoration: none;
  }
  .row {
    display: grid;
    grid-template-columns: 12px minmax(0, 1fr) 110px 150px minmax(300px, 1.6fr) 120px 52px 104px;
    align-items: center;
    column-gap: 16px;
    min-height: 46px;
    padding: 0 16px;
    border-bottom: 1px solid var(--line);
    font-size: 12.5px;
    cursor: pointer;
  }
  .row:hover {
    background: var(--surface-hover);
  }
  .row.head {
    min-height: 30px;
    background: var(--surface);
    cursor: default;
  }
  .row.head:hover {
    background: var(--surface);
  }
  .name-cell {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .name {
    color: var(--ink);
    font-size: 13px;
    font-weight: 600;
    text-decoration: none;
  }
  .tag {
    padding: 0 6px;
    border: 1px solid var(--line);
    color: var(--muted);
    font-size: 10.5px;
  }
  .release {
    display: flex;
    align-items: center;
    min-width: 0;
    color: var(--ink);
    text-decoration: none;
  }
  .pipe {
    display: flex;
    align-items: center;
  }
  .link {
    width: 12px;
    height: 1px;
    background: var(--line);
  }
  .stage {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 20px;
    padding: 0 7px;
    border: 1px solid var(--line);
    background: var(--surface);
    font-family: var(--mono);
    font-size: 10.5px;
    white-space: nowrap;
  }
  .stage.ok {
    border-color: color-mix(in srgb, var(--success) 55%, var(--line));
  }
  .stage.bad {
    border-color: var(--error);
    background: var(--error-soft);
  }
  .stage.lag {
    color: var(--warning);
  }
  .decision {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: 12px;
    white-space: nowrap;
  }
  .spark {
    display: block;
    width: 100%;
    height: 24px;
  }
  .spark-base {
    stroke: var(--line);
    vector-effect: non-scaling-stroke;
  }
  .spark-area {
    fill: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .spark-line {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.2;
    vector-effect: non-scaling-stroke;
  }
  .right {
    justify-self: end;
    text-align: right;
  }
  .error,
  .empty,
  .archived {
    margin: 0;
    padding: 14px 16px;
    font-size: 12.5px;
  }
  .error {
    color: var(--error);
  }
  @media (max-width: 1100px) {
    .row {
      grid-template-columns: 12px minmax(0, 1fr) 90px minmax(260px, 1.6fr) 52px 104px;
    }
    .row > :nth-child(4),
    .row > :nth-child(6) {
      display: none;
    }
  }
  @media (max-width: 760px) {
    .row {
      grid-template-columns: 12px minmax(0, 1fr) 80px 104px;
    }
    .row > :nth-child(5),
    .row > :nth-child(7) {
      display: none;
    }
  }
</style>
