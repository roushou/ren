<!--
  Home (`/`): where the operator lands. A prompt to start something; what
  needs them anywhere (projects' decisions and System's warnings), one card
  each with what they need to decide; what each agent is doing now; and
  what ended or moved on since they last looked.
  Each part scrolls within its own height, so a long list never pushes
  the others away.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import Toolbar from '#lib/Toolbar.svelte';
  import ApprovalModal from '#lib/ApprovalModal.svelte';
  import { ApprovalAnswers } from '#lib/approvals.svelte.ts';
  import { ManagementApi, clock, type UsageCounts } from '#lib/api.ts';
  import { harnessLabel } from '#lib/activity.ts';
  import { compactOrDash } from '#lib/display.ts';
  import { Home } from '#lib/home.ts';
  import { ProjectBoard } from '#lib/projects.ts';
  import { RecorderReport } from '#lib/recorder.ts';
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { Runs, Sessions } from '#lib/runs.ts';
  import { Work } from '#lib/work.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const board = $derived(
    new ProjectBoard(data.projects, data.work, data.live, data.sessions, null, data.clients),
  );
  const answers = new ApprovalAnswers();
  /** The machine's recording, for System's card when storage is near its limit. */
  const gateway = createSnapshot(() => data.gateway);
  const crowded = $derived.by(() => {
    const read = gateway.current.value;
    return read
      ? new RecorderReport(
          read.health.recorder,
          read.storage?.usage ?? null,
          read.storage?.policy ?? null,
          null,
          Date.now(),
        ).crowded
      : null;
  });

  /** When the operator last marked what happened as seen: kept in this
   * browser, a day ago at first. It moves only when they say so. */
  const SEEN = 'ren.home.seen';
  let now = $state(Date.now());
  let seen = $state(Date.now() - 86_400_000);
  let spent = $state<UsageCounts | null>(null);
  function readSpent(since: number) {
    new ManagementApi()
      .usage({ from_ms: since })
      .then((usage) => (spent = usage.totals))
      .catch(() => (spent = null));
  }
  onMount(() => {
    try {
      const stored = Number(localStorage.getItem(SEEN));
      if (Number.isFinite(stored) && stored > 0) seen = stored;
    } catch {
      // Storage refused: a day ago stands.
    }
    readSpent(seen);
    const ticking = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(ticking);
  });
  /** Everything up to now is seen: the list starts again from here. */
  function markSeen() {
    seen = Date.now();
    try {
      localStorage.setItem(SEEN, String(seen));
    } catch {
      // It lasts the visit.
    }
    readSpent(seen);
  }

  const underWay = $derived(Home.underWay(board));
  const outcomes = $derived(Home.outcomes(board, seen));
  const counts = $derived(Home.counts(outcomes));
  const needs = $derived(
    board.approvals.length +
      board.signedOut.length +
      (crowded ? 1 : 0) +
      board.toPromote.length +
      board.previews.length +
      board.queued.length,
  );

  /** The prompt to start with, and the project it is for. */
  let prompt = $state('');
  /** The project picked, by name; empty for Scratch, unset for the first. */
  let chosen = $state<string | undefined>(undefined);
  const target = $derived(
    chosen === undefined ? (board.active[0]?.name ?? null) : chosen === '' ? null : chosen,
  );
  /** The newest prompts, to start from again. */
  const recent = $derived(
    [
      ...new Set(
        board.sessions.flatMap((body) =>
          body.stored?.first_prompt ? [body.stored.first_prompt] : [],
        ),
      ),
    ].slice(0, 3),
  );
  function start(event: SubmitEvent) {
    event.preventDefault();
    const text = prompt.trim();
    if (!text) return;
    const query = new URLSearchParams({ new: '1', prompt: text });
    void goto(`${Sessions.deskHref(target)}?${query}`);
  }
  const age = (at: number | null) => (at === null ? '' : Sessions.age(at, now));
</script>

{#snippet scope()}
  <div class="scope-group"><span class="scope-title">Home</span></div>
{/snippet}

<Toolbar {scope} />
<div class="home">
  {#if data.runsControl}
    <form class="start" onsubmit={start} aria-label="Start a session">
      <label class="prompt">
        <Icon name="spark" size={14} />
        <input
          bind:value={prompt}
          placeholder="What should an agent do?"
          aria-label="What should an agent do?"
          autocomplete="off"
        />
        <select
          value={target ?? ''}
          onchange={(event) => (chosen = event.currentTarget.value)}
          aria-label="Project"
          class="project"
        >
          {#each board.active as project (project.id)}<option value={project.name}
              >{project.name}</option
            >{/each}
          <option value="">Scratch</option>
        </select>
        <button class="button small primary" type="submit" disabled={!prompt.trim()}
          >Start <kbd>↵</kbd></button
        >
      </label>
      {#if recent.length}<p class="recent">
          <span class="muted mono">recent</span>
          {#each recent as text (text)}<button type="button" onclick={() => (prompt = text)}
              >{text}</button
            >{/each}
        </p>{/if}
    </form>
  {/if}

  <div class="parts">
    <div class="left">
      <section class="part" aria-labelledby="needs-title">
        <header>
          <h2 id="needs-title" class="cap">Needs you · {needs}</h2>
        </header>
        <div class="scroll cards">
          {#each board.approvals as waiting (`${waiting.session}:${waiting.approval}`)}
            {@const said = Runs.ask(waiting.ask)}
            {@const desk = Sessions.deskHref(waiting.project, { session: waiting.session })}
            <article class="card urgent">
              <Glyph mark="yours" label="Waits for you" />
              <div class="body">
                <strong>{Runs.question(waiting.ask, harnessLabel(waiting.harness))}</strong>
                <span class="from muted"
                  ><a href={desk}>{waiting.prompt ?? 'A session'}</a> ·
                  <span class="mono">{waiting.project ?? 'Scratch'}</span></span
                >
                {#if said.detail}<code class="code">{said.detail}</code>{/if}
              </div>
              <div class="acts">
                {#if waiting.ask.kind === 'questions'}
                  <a class="button small primary" href={desk}>Answer</a>
                {:else}
                  <button class="button small" onclick={() => answers.review(waiting)}
                    >Review…</button
                  >
                  <button
                    class="button small primary"
                    disabled={answers.answering}
                    onclick={() => void answers.answer(waiting, 'accept')}>Allow once</button
                  >
                {/if}
              </div>
            </article>
          {/each}
          {#each board.signedOut as client (client.harness)}
            <article class="card">
              <Glyph mark="warning" label="Needs a look" />
              <div class="body">
                <strong>{harnessLabel(client.harness)} is signed out on this host</strong>
                <span class="from muted"
                  >System · new {harnessLabel(client.harness)} sessions cannot start until it signs in</span
                >
              </div>
              <div class="acts">
                <a class="button small primary" href="/system/tools">Sign in</a>
              </div>
            </article>
          {/each}
          {#if crowded}
            <article class="card">
              <Glyph mark="warning" label="Needs a look" />
              <div class="body">
                <strong>Recorder storage is at {RecorderReport.percent(crowded.share ?? 0)}</strong>
                <span class="from muted"
                  >System · {crowded.used} of {crowded.limit} · the oldest calls go first at the limit</span
                >
              </div>
              <div class="acts"><a class="button small" href="/system/recorder">Recorder</a></div>
            </article>
          {/if}
          {#each board.toPromote as project (project.id)}
            {@const canary = project.release?.canary ?? null}
            {@const checks = canary?.checks ?? []}
            <article class="card">
              <Glyph mark="passed" label="Canary passed" />
              <div class="body">
                <strong>{project.name}'s canary passed: ready to promote</strong>
                <span class="from muted mono"
                  >{canary?.commit.slice(0, 12)} · checks {checks.filter((check) => check.passed)
                    .length}/{checks.length}{project.release?.behind
                    ? ` · production ↓${project.release.behind}`
                    : ''} · {project.release?.policy}</span
                >
              </div>
              <div class="acts">
                <a class="button small" href={ProjectBoard.releaseHref(project)}>Release</a>
                {#if project.factory === 'ren' && canary}<a
                    class="button small primary"
                    href="{ProjectBoard.releaseHref(project)}?promote={encodeURIComponent(
                      canary.commit,
                    )}">Promote…</a
                  >{/if}
              </div>
            </article>
          {/each}
          {#each board.previews as item (item.id)}
            {@const home = board.homeOf(item)}
            <article class="card">
              <Glyph mark="info" label="A preview to check" />
              <div class="body">
                <strong>{item.title}</strong>
                <span class="from muted"
                  >a preview to check · <span class="mono">{home ?? 'no project'}</span></span
                >
              </div>
              <div class="acts">
                {#if home}<a class="button small" href={Work.href(home, item.id)}>Work item</a>{/if}
                {#if item.preview}<a
                    class="button small primary"
                    href={item.preview.ui}
                    target="_blank"
                    rel="noopener">Open</a
                  >{/if}
              </div>
            </article>
          {/each}
          {#each board.queued as queued (queued.project)}
            <article class="card">
              <Glyph mark="queued" label="Queued" />
              <div class="body">
                <strong>{queued.count} queued to send</strong>
                <span class="from muted mono">{queued.project ?? 'no project'}</span>
              </div>
              <div class="acts">
                {#if queued.project}<a class="button small" href={ProjectBoard.href(queued.project)}
                    >Open</a
                  >{/if}
              </div>
            </article>
          {/each}
          {#if !needs}<p class="quiet muted">
              <Glyph mark="passed" label={true} /> Nothing needs you.
            </p>{/if}
        </div>
      </section>

      <section class="part" aria-labelledby="underway-title">
        <header>
          <h2 id="underway-title" class="cap">Under way · {underWay.length}</h2>
        </header>
        <div class="scroll">
          {#each underWay as row (row.session)}
            <a class="row live" href={row.href}>
              <Glyph mark={row.mark} label={row.doing} />
              <span class="title">{row.title}</span>
              <span class="doing mono muted">{row.doing}</span>
              <span class="chip mono">{row.project ?? 'Scratch'}</span>
              <span class="muted mono when">{age(row.since)}</span>
            </a>
          {:else}
            <p class="quiet muted">No session is running.</p>
          {/each}
        </div>
      </section>
    </div>

    <aside class="part since" aria-labelledby="since-title">
      <header>
        <h2 id="since-title" class="cap">Since you looked</h2>
        <span class="seen"
          ><span class="muted mono" title={new Date(seen).toLocaleString()}
            >{Home.looked(seen, now)}</span
          ><button
            class="button small"
            disabled={!outcomes.length}
            title="Mark everything here as seen"
            onclick={markSeen}>Mark seen</button
          ></span
        >
      </header>
      <dl class="tiles">
        <div>
          <dt class="cap">Sessions ended</dt>
          <dd class="mono"><Glyph mark="passed" />{counts.ended}</dd>
        </div>
        <div>
          <dt class="cap">Work landed</dt>
          <dd class="mono"><Glyph mark="passed" />{counts.landed}</dd>
        </div>
        <div>
          <dt class="cap">Shipped</dt>
          <dd class="mono">{counts.shipped}</dd>
        </div>
        <div>
          <dt class="cap">Ended short</dt>
          <dd class="mono">
            {#if counts.failed}<Glyph mark="failed" />{/if}{counts.failed}
          </dd>
        </div>
        <div class="wide">
          <dt class="cap">Tokens</dt>
          <dd class="mono">
            {spent
              ? `${compactOrDash(spent.tokens.input)} in · ${compactOrDash(spent.tokens.output)} out`
              : '—'}
          </dd>
        </div>
      </dl>
      <div class="scroll">
        {#each outcomes as one (one.key)}
          {#if one.href}<a class="row outcome" href={one.href}>
              <Glyph mark={one.mark} label={one.happened} />
              <span class="title">{one.title}</span>
              <span class="chip mono">{one.project ?? 'Scratch'}</span>
              <span class="muted mono when" title={clock(one.at)}>{age(one.at)}</span>
            </a>{:else}<div class="row outcome">
              <Glyph mark={one.mark} label={one.happened} />
              <span class="title">{one.title}</span>
              <span class="chip mono">{one.project ?? 'Scratch'}</span>
              <span class="muted mono when" title={clock(one.at)}>{age(one.at)}</span>
            </div>{/if}
        {:else}
          <p class="quiet muted">Nothing ended or moved on since then.</p>
        {/each}
      </div>
    </aside>
  </div>
</div>
<ApprovalModal {answers} />

<style>
  .home {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    background: var(--canvas);
  }
  .start {
    flex: none;
    padding: 12px 16px;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
  }
  .prompt {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 42px;
    padding: 0 8px 0 12px;
    border: 1px solid var(--control-border);
    border-radius: 3px;
    background: var(--surface-inset);
    color: var(--accent);
  }
  .prompt input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: none;
    color: var(--ink);
    font: inherit;
    font-size: 13.5px;
  }
  .project {
    max-width: 180px;
    font-size: 12px;
  }
  kbd {
    margin-left: 4px;
    font-family: var(--mono, monospace);
    font-size: 10px;
    opacity: 0.8;
  }
  .recent {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin: 8px 0 0;
    font-size: 11.5px;
  }
  .recent button {
    max-width: 260px;
    padding: 1px 8px;
    overflow: hidden;
    border: 1px solid var(--line);
    border-radius: 3px;
    background: none;
    color: var(--ink);
    font: inherit;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .recent button:hover {
    border-color: var(--control-border);
  }
  /* Each part has its own height and scrolls within it. */
  .parts {
    display: grid;
    flex: 1;
    grid-template-columns: minmax(0, 1fr) 380px;
    min-height: 0;
  }
  .left {
    display: grid;
    grid-template-rows: minmax(0, 1.15fr) minmax(0, 1fr);
    min-height: 0;
    border-right: 1px solid var(--line);
  }
  .left .part + .part {
    border-top: 1px solid var(--line);
  }
  .part {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .part > header {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    height: 36px;
    padding: 0 16px;
    border-bottom: 1px solid var(--line);
    font-size: 11px;
  }
  .cap {
    margin: 0;
    color: var(--muted);
    font-family: var(--mono, monospace);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .cards {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 16px;
  }
  .card {
    display: grid;
    flex: none;
    grid-template-columns: 12px minmax(0, 1fr) auto;
    align-items: start;
    column-gap: 12px;
    padding: 12px 14px;
    border: 1px solid var(--line);
    background: var(--surface);
  }
  .card > :global(.glyph) {
    margin-top: 4px;
  }
  .card.urgent {
    border-color: color-mix(in srgb, var(--warning) 55%, var(--line));
    box-shadow: inset 2px 0 var(--warning);
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
  }
  .body strong {
    font-size: 13px;
    font-weight: 600;
  }
  .from {
    font-size: 11.5px;
  }
  .code {
    padding: 6px 9px;
    overflow: hidden;
    border: 1px solid var(--line);
    background: var(--surface-inset);
    font-size: 11.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .acts {
    display: flex;
    gap: 6px;
  }
  .row {
    display: grid;
    align-items: center;
    column-gap: 12px;
    min-height: 38px;
    padding: 0 16px;
    border-bottom: 1px solid var(--line-subtle);
    color: var(--ink);
    font-size: 12.5px;
    text-decoration: none;
  }
  a.row:hover {
    background: var(--surface-hover);
  }
  .live {
    grid-template-columns: 12px minmax(0, 1.2fr) minmax(0, 1fr) auto 44px;
  }
  .outcome {
    grid-template-columns: 12px minmax(0, 1fr) auto 44px;
  }
  .title {
    overflow: hidden;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .doing {
    overflow: hidden;
    font-size: 11.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .when {
    font-size: 11px;
    text-align: right;
  }
  .since {
    background: var(--surface);
  }
  .seen {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .tiles {
    display: grid;
    flex: none;
    grid-template-columns: 1fr 1fr;
    margin: 0;
    border-bottom: 1px solid var(--line);
  }
  .tiles > div {
    padding: 10px 14px;
    border-right: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  .tiles > div:nth-child(2n) {
    border-right: 0;
  }
  .tiles > div.wide {
    grid-column: 1 / -1;
    border-right: 0;
    border-bottom: 0;
  }
  .tiles dt {
    font-size: 9.5px;
  }
  .tiles dd {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 4px 0 0;
    font-size: 17px;
  }
  .tiles .wide dd {
    font-size: 14px;
  }
  .quiet {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    padding: 14px 16px;
    font-size: 12.5px;
  }
  @media (max-width: 960px) {
    /* Stacked, the page scrolls and each list keeps a fixed height. */
    .parts,
    .left {
      display: flex;
      flex: none;
      flex-direction: column;
      border-right: 0;
    }
    .left .part {
      flex: none;
      height: 360px;
    }
    .parts > aside {
      border-top: 1px solid var(--line);
    }
    .home {
      overflow: auto;
    }
  }
</style>
