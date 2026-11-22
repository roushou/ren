<script lang="ts">
  import { Now } from '#lib/now.svelte.ts';
  import { Marks } from '#lib/marks.ts';
  import { harnessLabel } from '#lib/activity.ts';
  import Toasts from '#lib/layer/Toasts.svelte';
  import { toasts } from '#lib/layer/toasts.svelte.ts';
  import CommandPalette from '#lib/layer/CommandPalette.svelte';
  import { ProjectBoard } from '#lib/projects.ts';
  import { onMount, untrack } from 'svelte';
  import { beforeNavigate, invalidate, preloadCode } from '$app/navigation';
  import { page, navigating } from '$app/state';
  import { createArrival, createSettled } from '#lib/snapshot.svelte.ts';
  import { Workbench, setWorkbench } from '#lib/workbench.svelte.ts';
  import { LiveConnection } from '#lib/live.svelte.ts';
  import { ManagementApi } from '#lib/api.ts';
  import type { SnapshotError } from '#lib/snapshots.ts';
  import { RecorderReport } from '#lib/recorder.ts';
  import { PageEvidence, Work } from '#lib/work.ts';
  import RedlineLayer from 'redline/RedlineLayer.svelte';
  import BriefingPanel from '#lib/BriefingPanel.svelte';
  import { RedlineDesk } from 'redline/desk';
  import { Recipients } from '#lib/briefing.svelte.ts';
  import { Views } from '#lib/view.ts';
  import type { EdgeReading } from '#lib/edge.ts';
  import { Arrival, type Arriving } from '#lib/arrival.ts';
  import WorkbenchHeader from '#lib/shell/WorkbenchHeader.svelte';
  import BottomEdge from '#lib/shell/BottomEdge.svelte';
  import '@fontsource/inter/latin-400.css';
  import '@fontsource/inter/latin-500.css';
  import '@fontsource/inter/latin-600.css';
  import '@fontsource/jetbrains-mono/latin-400.css';
  import '@fontsource/jetbrains-mono/latin-500.css';
  import '#lib/style.css';
  import type { LayoutProps } from './$types';
  let { data, children }: LayoutProps = $props();
  const workbench = setWorkbench(new Workbench());
  /** Marking what's wrong on a page, for an agent to fix: needs the control token. */
  const redline = new RedlineDesk();
  /** Where Redline's briefs go: sessions in Ren's own source checkout. */
  const recipients = new Recipients(() => page.data.redlineSource);
  // Hydrated: from here on, pages that can take their data pending render
  // at once on a navigation, with their own outline (`Arrival`).
  onMount(() => Arrival.stream());
  /** What this page logged and read, carried by a work item marked on it. */
  const evidence = new PageEvidence();
  beforeNavigate(({ from, to }) => {
    if (from?.url.pathname !== to?.url.pathname) evidence.reset();
  });
  onMount(() => {
    const origin = window.location.origin;
    // The page's reads of the daemon's API, as the browser timed them.
    const reads = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        const read = PageEvidence.readOf(entry.name, origin);
        if (read) evidence.read(read);
      }
    });
    reads.observe({ type: 'resource', buffered: true });
    const failed = (event: ErrorEvent) => evidence.error(event.message);
    const rejected = (event: PromiseRejectionEvent) => evidence.error(String(event.reason));
    window.addEventListener('error', failed);
    window.addEventListener('unhandledrejection', rejected);
    // What the page itself logs as an error (Svelte's included), kept as text.
    const log = console.error;
    console.error = (...args: unknown[]) => {
      evidence.error(
        args.map((arg) => (arg instanceof Error ? arg.message : String(arg))).join(' '),
      );
      log(...args);
    };
    return () => {
      reads.disconnect();
      window.removeEventListener('error', failed);
      window.removeEventListener('unhandledrejection', rejected);
      console.error = log;
    };
  });
  const gateway = createArrival(() => data.gateway);
  const health = $derived(gateway.snapshot?.value?.health);
  const storage = $derived(gateway.snapshot?.value?.storage);
  const view = $derived(Views.of(page.route.id));
  /** The view being navigated to, when it is another one: its tab lights up
   * at once. */
  const target = $derived(
    navigating.to && navigating.to.route.id !== page.route.id
      ? Views.of(navigating.to.route.id)
      : null,
  );
  const tab = $derived.by(() => {
    const shown = target ?? view;
    return shown ? Views.tab(shown) : null;
  });
  /** Home, or the place the page is in: the bar shows only it. */
  const where = $derived(tab ? Views.place(tab) : 'home');
  /** The place's own entry: its name, and where that leads. */
  const here = $derived(Views.places.find((each) => each.place === where) ?? null);
  /** What waits on the operator, across every project: the top bar's Pending. */
  const boardRead = createArrival(() => data.board);
  const board = $derived(ProjectBoard.read(boardRead.snapshot?.value ?? null));
  /** What newly waits on the operator, said as a toast as it arrives:
   * unless the page that says it is the one on screen. The first board says
   * nothing; it is where the operator starts. */
  let lastBoard: ProjectBoard | null = null;
  $effect(() => {
    const now = board;
    // Not before the board is in: an empty board is not news.
    if (!boardRead.snapshot) return;
    const before = lastBoard;
    lastBoard = now;
    if (!before) return;
    const here = untrack(() => `${page.url.pathname}${page.url.search}`);
    for (const said of ProjectBoard.news(before, now))
      if (said.link?.href !== here) toasts.show(said);
  });
  /** Whether ⌘K's palette is open. */
  let commanding = $state(false);
  function shortcut(event: KeyboardEvent) {
    if (event.key.toLowerCase() === 'k' && (event.metaKey || event.ctrlKey) && !event.altKey) {
      event.preventDefault();
      commanding = true;
    }
  }
  /** The project whose pages the row shows, inside one. */
  const inProject = $derived.by(() => {
    const to = navigating.to ?? page;
    return to.route.id?.startsWith('/projects/[name]') ? (to.params?.name ?? null) : null;
  });
  const clock = new Now(60_000);
  /** How recording stands on this host, for System's mark on Home. */
  const hostReport = $derived(
    health
      ? new RecorderReport(
          health.recorder,
          storage?.usage ?? null,
          storage?.policy ?? null,
          null,
          clock.current,
        )
      : null,
  );
  const hostVerdict = $derived(hostReport?.verdict ?? null);
  /** What on the host needs a look, worst first; null when nothing does. */
  const hostAttention = $derived.by((): { mark: 'failed' | 'warning'; label: string } | null => {
    if (error) return { mark: 'failed', label: 'the gateway is not answering' };
    if (hostVerdict && hostVerdict.tone !== 'success')
      return {
        mark: Marks.ofTone(hostVerdict.tone) === 'failed' ? 'failed' : 'warning',
        label: hostVerdict.headline,
      };
    if (hostReport?.crowded) return { mark: 'warning', label: 'storage is near its limit' };
    if (board.signedOut.length)
      return {
        mark: 'warning',
        label: `${board.signedOut.map((client) => harnessLabel(client.harness)).join(', ')} signed out`,
      };
    return null;
  });
  /** The project a Monitor page is narrowed to, when it can be. */
  const scope = $derived(tab && Views.scoped(tab) ? page.url.searchParams.get('project') : null);
  /** The same page, for every project. */
  const unscoped = $derived.by(() => {
    const params = new URLSearchParams(page.url.search);
    params.delete('project');
    params.delete('after');
    const search = params.toString();
    return search ? `${page.url.pathname}?${search}` : page.url.pathname;
  });
  /** The page's reads, as each comes in, for the status line and the error banner. */
  /** The pages' own reads, by the name their loads give them: any that
   * failed is the bottom edge's to say. */
  const PAGE_READS = [
    'overview',
    'traffic',
    'live',
    'recordings',
    'usage',
    'recorder',
    'report',
    'search',
  ] as const;
  const reads = createSettled(() =>
    PAGE_READS.map((key) => page.data[key] as Arriving<unknown> | undefined),
  );
  const pageTitle = $derived(page.error ? 'Page unavailable' : view ? Views.title(view) : 'Ren');
  const gatewayOrigin = $derived(page.data.gatewayUrl);
  // Bytes past the capture policy are left out by design: never loss.
  const capture = $derived(health ? RecorderReport.capture(health.recorder) : null);
  const loss = $derived(capture?.kind === 'lost');
  const degraded = $derived(!!health && (health.status !== 'ok' || capture?.kind === 'failing'));
  let refreshError = $state('');
  const failure = $derived<SnapshotError | null>(
    gateway.snapshot?.error ??
      PAGE_READS.map(
        (key) => reads.of(page.data[key] as Arriving<unknown> | undefined)?.error,
      ).find((error) => error) ??
      null,
  );
  const error = $derived(refreshError || failure?.message || '');
  const incompatible = $derived(failure?.kind === 'incompatible');
  const updated = $derived(gateway.snapshot?.updated);
  /** Recorder notices the operator dismissed this visit, by their words. */
  let dismissed = $state<string[]>([]);
  /** What the bottom edge says its strips from. */
  const edge = $derived<EdgeReading>({
    failure: error ? (incompatible ? 'incompatible' : 'unreachable') : null,
    readAt: updated ?? null,
    recorder: !health
      ? null
      : degraded
        ? 'the recorder logged errors'
        : capture?.kind === 'lost'
          ? RecorderReport.lossText(capture)
          : null,
    dismissed: new Set(dismissed),
    readOnly: !page.data.runsControl,
  });
  async function reload() {
    // A navigation in flight loads fresh data itself.
    if (navigating.to) return;
    try {
      // Every load that read Ren reads it again; the server loads that only
      // hold settings (and the desk's and Packages', whose pages follow their
      // own feeds) do not. A refresh a click interrupts then leaves only
      // those reads to redo, not a round trip to this server first.
      await invalidate(() => true, true);
      refreshError = '';
      workbench.refreshed++;
    } catch {
      refreshError = 'Unable to refresh this view.';
    }
  }
  async function refresh() {
    workbench.manual = true;
    try {
      await (workbench.connection ? workbench.connection.refresh() : reload());
    } finally {
      workbench.manual = false;
    }
  }
  // The views' code is small and static: fetch it once up front, so switching
  // views waits only for data (which hovering a tab also starts).
  onMount(() => {
    for (const route of [
      '/',
      '/projects',
      '/monitor',
      '/monitor/live',
      '/monitor/traffic',
      '/monitor/sessions',
      '/monitor/usage',
      '/system/recorder',
      '/monitor/sessions/[id]',
      '/monitor/sessions/[id]/report',
      '/monitor/search',
      '/scratch/[[session]]',
    ] as const)
      void preloadCode(route);
  });
  onMount(() => {
    const connection = new LiveConnection({
      refresh: reload,
      reload,
      progress: (update) => workbench.stream(update),
    });
    workbench.connection = connection;
    return () => {
      workbench.connection = null;
      connection.destroy();
    };
  });
  $effect(() => {
    // Follow the live feed only while automatic updates are on.
    workbench.connection?.dispatch({ kind: workbench.automatic ? 'enable' : 'disable' });
  });
</script>

<svelte:window onkeydown={shortcut} />

<svelte:head><title>Ren - {pageTitle}</title></svelte:head>
<div class="app-shell">
  <WorkbenchHeader
    {view}
    environment={health?.instance.environment ?? null}
    {where}
    {here}
    {tab}
    {inProject}
    {scope}
    {board}
    {hostAttention}
    recording={loss || degraded ? 'attention' : 'well'}
    {error}
    updated={updated ?? null}
    {gatewayOrigin}
    redline={page.data.runsControl ? { desk: redline, recipients } : null}
    oncommand={() => (commanding = true)}
    onrefresh={() => void refresh()}
  />
  {#if commanding}<CommandPalette
      projects={board.active}
      here={inProject}
      sources={{
        promotable: board.toPromote.flatMap((project) =>
          project.factory === 'ren' && project.release?.canary
            ? [{ project: project.name, commit: project.release.canary.commit }]
            : [],
        ),
        work: board.work.map((item) => {
          const home = board.homeOf(item);
          return { title: item.title, project: home, href: home ? Work.href(home, item.id) : null };
        }),
      }}
      onclose={() => (commanding = false)}
    />{/if}
  {#if scope}
    <div class="scope-row" role="status">
      <span>Project <a href={`/projects/${encodeURIComponent(scope)}`}>{scope}</a></span>
      <span class="muted">its sessions' runs only</span>
      <a class="clear" href={unscoped}>All projects</a>
    </div>
  {/if}
  {#if navigating.to}<div
      class="nav-progress"
      role="progressbar"
      aria-label="Loading view"
    ></div>{/if}
  <main class="workspace" class:navigating={!!navigating.to} aria-busy={!!navigating.to}>
    {@render children()}
  </main>
  <Toasts />
  <BottomEdge
    reading={edge}
    onretry={() => void refresh()}
    ondismiss={(said) => (dismissed = [...dismissed, said])}
  />
  {#if page.data.runsControl}<RedlineLayer desk={redline} />{#if redline.open}<BriefingPanel
        desk={redline}
        {recipients}
        source={page.data.redlineSource}
        revision={health?.build.revision ?? null}
        {evidence}
        renProject={ManagementApi.ren(board.projects)?.name ?? null}
      />{/if}{/if}
</div>

<style>
  .app-shell {
    position: relative;
    height: 100vh;
    height: 100dvh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  /* A Monitor page narrowed to one project, under the header. */
  .scope-row {
    display: flex;
    flex-shrink: 0;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 12px;
    padding: 6px 14px;
    font-size: 12px;
    background: var(--accent-soft);
    border-bottom: 1px solid var(--accent-border);
    color: var(--accent-text);
  }
  .scope-row a {
    color: var(--accent-text);
    font-weight: 600;
  }
  .scope-row .clear {
    margin-left: auto;
    font-weight: 500;
  }
  /* Loading: an indeterminate bar under the header while a view loads. It
     appears only after a short delay, so fast navigations do not flicker. */
  .nav-progress {
    position: absolute;
    top: 73px;
    left: 0;
    right: 0;
    z-index: 20;
    height: 2px;
    overflow: hidden;
    opacity: 0;
    animation: progress-in 0s linear 120ms forwards;
  }
  .nav-progress::before {
    content: '';
    position: absolute;
    inset: 0 auto 0 0;
    width: 35%;
    background: var(--accent);
    animation: progress-run 1.1s ease-in-out infinite;
  }
  @keyframes progress-in {
    to {
      opacity: 1;
    }
  }
  @keyframes progress-run {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(300%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .nav-progress::before {
      width: 100%;
      animation: none;
      opacity: 0.6;
    }
  }
</style>
