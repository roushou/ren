<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import { Marks } from '#lib/marks.ts';
  import { harnessLabel } from '#lib/activity.ts';
  import Toasts from '#lib/layer/Toasts.svelte';
  import { toasts } from '#lib/layer/toasts.svelte.ts';
  import Popover from '#lib/layer/Popover.svelte';
  import CommandPalette from '#lib/layer/CommandPalette.svelte';
  import PendingPanel from '#lib/PendingPanel.svelte';
  import { ProjectBoard } from '#lib/projects.ts';
  import { onMount, untrack } from 'svelte';
  import { beforeNavigate, preloadCode, refreshAll } from '$app/navigation';
  import { page, navigating } from '$app/state';
  import { createSettled, createSnapshot } from '#lib/snapshot.svelte.ts';
  import { Workbench, setWorkbench } from '#lib/workbench.svelte.ts';
  import { LiveConnection } from '#lib/live.svelte.ts';
  import { clock } from '#lib/api.ts';
  import type { SnapshotError } from '#lib/snapshots.ts';
  import Icon, { type IconName } from '#lib/Icon.svelte';
  import ThemeControl from '#lib/ThemeControl.svelte';
  import ConnectClient from '#lib/ConnectClient.svelte';
  import AddressToggle from '#lib/AddressToggle.svelte';
  import { addressView } from '#lib/address.svelte.ts';
  import { GatewayPlace, type Network } from '#lib/place.ts';
  import { RecorderReport } from '#lib/recorder.ts';
  import { PageEvidence, Work } from '#lib/work.ts';
  import CopyButton from '#lib/CopyButton.svelte';
  import RedlineLayer from 'redline/RedlineLayer.svelte';
  import BriefingPanel from '#lib/BriefingPanel.svelte';
  import { RedlineDesk } from 'redline/desk';
  import { Recipients } from '#lib/briefing.svelte.ts';
  import { Views } from '#lib/view.ts';
  import { BottomEdge } from '#lib/edge.ts';
  import { Arrival } from '#lib/arrival.ts';
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
  const gateway = createSnapshot(() => data.gateway);
  const health = $derived(gateway.current.value?.health);
  /** The tag on the wordmark of an instance that is not production: the
   * gateway says what kind of instance it is; the UI only shows it. */
  const marked = $derived.by(() => {
    switch (health?.instance.environment) {
      case 'development':
        return { tag: 'dev', what: 'A development gateway, not the installed service' };
      case 'canary':
        return { tag: 'canary', what: "The canary: main's tip on a copy of production's history" };
      case 'preview':
        return {
          tag: 'preview',
          what: "A preview: one worktree's branch on a copy of production's history",
        };
      default:
        return null;
    }
  });
  const storage = $derived(gateway.current.value?.storage);
  /** The tab row, which scrolls sideways on a phone. */
  let tabs = $state<HTMLElement>();
  // Keep the current view's tab in sight when the row is narrower than its tabs.
  $effect(() => {
    void page.url.pathname;
    tabs
      ?.querySelector('[aria-current="page"]')
      ?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });
  const view = $derived(Views.of(page.route.id));
  /** The view being navigated to, when it is another one: its tab lights up
   * at once. */
  const target = $derived(
    navigating.to && navigating.to.route.id !== page.route.id
      ? Views.of(navigating.to.route.id)
      : null,
  );
  const tab = $derived(Views.tab(target ?? view));
  /** Home, or the place the page is in: the bar shows only it. */
  const where = $derived(Views.place(tab));
  /** The place's own entry: its name, and where that leads. */
  const here = $derived(Views.places.find((each) => each.place === where) ?? null);
  /** What waits on the operator, across every project: the top bar's Pending. */
  const board = $derived(
    new ProjectBoard(
      data.board.projects,
      data.board.work,
      data.board.live,
      data.board.sessions,
      null,
      data.board.clients,
    ),
  );
  /** What newly waits on the operator, said as a toast as it arrives:
   * unless the page that says it is the one on screen. The first board says
   * nothing; it is where the operator starts. */
  let lastBoard: ProjectBoard | null = null;
  $effect(() => {
    const now = board;
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
  /** How recording stands on this host, for System's mark on Home. */
  const hostReport = $derived(
    health
      ? new RecorderReport(
          health.recorder,
          storage?.usage ?? null,
          storage?.policy ?? null,
          null,
          Date.now(),
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
  /** The project an Inspect page is narrowed to, when it can be. */
  const scope = $derived(Views.scoped(tab) ? page.url.searchParams.get('project') : null);
  /** The same page, for every project. */
  const unscoped = $derived.by(() => {
    const params = new URLSearchParams(page.url.search);
    params.delete('project');
    params.delete('after');
    const search = params.toString();
    return search ? `${page.url.pathname}?${search}` : page.url.pathname;
  });
  /** The page's reads, as each comes in, for the status line and the error banner. */
  const reads = createSettled(() => [
    page.data.overview,
    page.data.traffic,
    page.data.live,
    page.data.recordings,
    page.data.usage,
    page.data.recorder,
    page.data.report,
    page.data.turn,
    page.data.search,
  ]);
  const pageTitle = $derived(page.error ? 'Page unavailable' : Views.title(view));
  const gatewayOrigin = $derived(page.data.gatewayUrl);
  // The header names where the gateway runs, not its address.
  const place = $derived(GatewayPlace.parse(gatewayOrigin));
  const shownOrigin = $derived(addressView.revealed ? place.origin : place.masked);
  const networkIcons = {
    loopback: 'monitor',
    tailnet: 'lock',
    private: 'lan',
    public: 'globe',
  } as const satisfies Record<Network, IconName>;
  // Bytes past the capture policy are left out by design: never loss.
  const capture = $derived(health ? RecorderReport.capture(health.recorder) : null);
  const loss = $derived(capture?.kind === 'lost');
  const degraded = $derived(!!health && (health.status !== 'ok' || capture?.kind === 'failing'));
  const loading = $derived(workbench.manual || !!navigating.to);
  let refreshError = $state('');
  const failure = $derived<SnapshotError | null>(
    gateway.current.error ??
      reads.of(page.data.traffic)?.error ??
      reads.of(page.data.live)?.error ??
      reads.of(page.data.recordings)?.error ??
      reads.of(page.data.recorder)?.error ??
      reads.of(page.data.report)?.error ??
      reads.of(page.data.turn)?.error ??
      reads.of(page.data.search)?.error ??
      null,
  );
  const error = $derived(refreshError || failure?.message || '');
  const incompatible = $derived(failure?.kind === 'incompatible');
  const updated = $derived(gateway.current.updated);
  /** Recorder notices the operator dismissed this visit, by their words. */
  let dismissed = $state<string[]>([]);
  /** Whether every strip shows, not just the worst. */
  let edgeOpen = $state(false);
  const strips = $derived(
    BottomEdge.strips({
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
    }),
  );
  async function reload() {
    // A navigation in flight loads fresh data itself.
    if (navigating.to) return;
    workbench.refreshing = true;
    try {
      await refreshAll();
      refreshError = '';
      workbench.refreshed++;
    } catch {
      refreshError = 'Unable to refresh this view.';
    } finally {
      workbench.refreshing = false;
    }
  }
  let live = $state<LiveConnection>();
  async function refresh() {
    workbench.manual = true;
    try {
      await (live ? live.refresh() : reload());
    } finally {
      workbench.manual = false;
    }
  }
  // Set before any page renders its toolbar, on the server too.
  workbench.controls = {
    refresh: () => void refresh(),
    loading: () => loading,
  };
  // The views' code is small and static: fetch it once up front, so switching
  // views waits only for data (which hovering a tab also starts).
  onMount(() => {
    for (const route of [
      '/',
      '/projects',
      '/monitor/overview',
      '/monitor/live',
      '/monitor/traffic',
      '/monitor/recordings',
      '/monitor/usage',
      '/system/recorder',
      '/monitor/recordings/[id]',
      '/monitor/recordings/[id]/report',
      '/monitor/turns/[id]',
      '/monitor/search',
      '/scratch',
    ] as const)
      void preloadCode(route);
  });
  onMount(() => {
    live = new LiveConnection({
      refresh: reload,
      reload,
      progress: (update) => workbench.stream(update),
    });
    return () => live?.destroy();
  });
  $effect(() => {
    // Follow the live feed only while automatic updates are on.
    live?.dispatch({ kind: workbench.automatic ? 'enable' : 'disable' });
  });
  $effect(() => {
    if (live) workbench.live = live.state;
  });
</script>

<svelte:window onkeydown={shortcut} />

<svelte:head><title>Ren - {pageTitle}</title></svelte:head>
<div class="app-shell">
  <header class="workbench-header">
    <a
      class="brand"
      class:marked
      class:active={view === 'home'}
      href="/"
      aria-current={view === 'home' ? 'page' : undefined}
      aria-label={marked ? `Home, ${marked.tag} gateway` : 'Home'}
      title={marked?.what ?? 'Home: what needs you, what is moving, what happened'}
      ><span class="wordmark" aria-hidden="true">REN</span>{#if marked}<span
          class="instance-badge"
          aria-hidden="true">{marked.tag}</span
        >{/if}</a
    >
    <nav class="workbench-context" aria-label="Where you are" bind:this={tabs}>
      {#if here === null}
        <!-- Home leads to the three places; inside one, only it shows. -->
        {#each Views.places as each (each.place)}
          <a class="place-link" href={each.href} data-sveltekit-preload-data="hover"
            >{each.label}{#if each.place === 'system' && hostAttention}<Glyph
                mark={hostAttention.mark}
                label={hostAttention.label}
              />{/if}</a
          >
        {/each}
      {:else}
        <a
          class="place-name"
          class:active={view === 'projects'}
          href={here.href}
          aria-current={view === 'projects' ? 'page' : undefined}
          >{here.label}{#if where === 'system' && place.node}<span class="mono muted"
              >{place.node}</span
            >{/if}</a
        >
      {/if}
      {#if where === 'projects' && (inProject || view === 'scratch')}
        <button class="project-pick" popovertarget="project-picker" title="Switch project"
          ><Icon name="branch" size={13} /><strong>{inProject ?? 'Scratch'}</strong><Icon
            name="chevron"
            size={10}
          /></button
        >
        <Popover id="project-picker" label="Projects" align="left">
          <ul class="project-list">
            {#each board.active as project (project.id)}
              <li>
                <a
                  href={ProjectBoard.href(project.name)}
                  aria-current={project.name === inProject ? 'page' : undefined}
                  ><Glyph
                    mark={board.approvals.some((waiting) => waiting.project === project.name)
                      ? 'yours'
                      : project.running
                        ? 'responding'
                        : 'idle'}
                    label={true}
                  /><span>{project.name}</span></a
                >
              </li>
            {/each}
            <li>
              <a href="/scratch" aria-current={view === 'scratch' ? 'page' : undefined}
                ><Glyph mark="idle" label="Sessions that work for no project" /><span>Scratch</span
                ></a
              >
            </li>
            <li class="all"><a href="/projects">All projects</a></li>
          </ul>
        </Popover>
      {/if}
      {#each view === 'scratch' ? [] : Views.pages(where, inProject) as link (link.tab)}
        <a
          href={Views.scopedHref(link, scope)}
          data-sveltekit-preload-data="hover"
          class:active={tab === link.tab}
          aria-current={tab === link.tab ? 'page' : undefined}
          >{link.label}{#if link.tab === 'recorder' && (loss || degraded)}<Glyph
              mark="warning"
              label="Recording needs attention"
            />{/if}</a
        >
      {/each}
    </nav>
    <button class="command-open" onclick={() => (commanding = true)}
      ><Icon name="search" size={13} /><span>Go to anything</span><kbd>⌘K</kbd></button
    >
    {#if where === 'projects'}
      <button
        class="pending-open"
        class:waiting={board.waiting > 0}
        popovertarget="pending-popover"
        aria-label={board.waiting ? `Pending: ${board.waiting}` : 'Pending: nothing waits on you'}
        ><Glyph mark={board.waiting ? 'yours' : 'idle'} />Pending{#if board.waiting}<span
            class="pending-count mono">{board.waiting}</span
          >{/if}</button
      >
      <Popover id="pending-popover" label="Pending" width="wide"><PendingPanel {board} /></Popover>
    {/if}
    {#if where === 'home' || where === 'system'}
      <button
        class="gateway-place mono"
        class:public={place.network === 'public'}
        popovertarget="connect-panel"
        title="Gateway on {place.node ??
          GatewayPlace.networkLabel(place.network)}, reached over {GatewayPlace.networkLabel(
          place.network,
        )}. {error ? 'The management API is not answering. ' : ''}Connect a client"
        ><Glyph
          mark={error ? 'failed' : updated ? 'passed' : 'waiting'}
        />{#if place.node && place.network !== 'loopback'}<b>{place.node}</b><span
            class="gateway-sep">·</span
          >{/if}<Icon name={networkIcons[place.network]} size={12} /><span
          >{place.network === 'loopback'
            ? 'this machine'
            : GatewayPlace.networkLabel(place.network)}</span
        ><Icon name="chevron" size={11} /></button
      >
      <div id="connect-panel" class="connect-panel" popover="auto" aria-label="Connect a client">
        <header>
          <strong>Connect a client</strong>
          <span class="mono">{shownOrigin}</span>{#if place.maskable}<AddressToggle
            />{/if}<CopyButton text={gatewayOrigin} label="Copy the gateway address" />
        </header>
        <ConnectClient layout="list" {gatewayOrigin} />
        <p><Icon name="shield" size={12} /> Each client keeps its own sign-in; Ren forwards it.</p>
      </div>
    {/if}
    <!-- Redline: mark what's wrong on this page for an agent to fix. A
         tool, so it sits with the bar's tools, not over the page. -->
    {#if page.data.runsControl}<button
        class="redline-toggle"
        class:on={redline.open}
        data-redline
        aria-pressed={redline.open}
        aria-label={redline.marks.length ? `Redline: ${redline.marks.length} marks` : 'Redline'}
        onpointerenter={() => recipients.prepare()}
        onfocus={() => recipients.prepare()}
        onclick={() => redline.toggle()}
        title="Redline: mark what's wrong on this page for an agent to fix (m)"
        ><Icon name="pencil" size={13} />{#if redline.marks.length}<span class="redline-count mono"
            >{redline.marks.length}</span
          >{/if}</button
      >{/if}
    <ThemeControl />
  </header>
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
  <!-- The bottom edge: silent while everything is well; one strip when
       something is not, the worst first. -->
  <div class="edge">
    {#each edgeOpen ? strips : strips.slice(0, 1) as strip, place (strip.kind)}
      {@const tone = BottomEdge.tone(strip)}
      <div class="strip {tone}" role={tone === 'bad' ? 'alert' : 'status'}>
        {#if strip.kind === 'unreachable'}<Glyph mark="failed" /><span
            ><b>Ren isn't answering.</b>
            {strip.readAt
              ? `Showing what was read at ${clock(strip.readAt)}.`
              : 'Nothing has been read yet.'}</span
          ><span class="strip-acts"
            >{#if loading}<span class="muted mono">retrying…</span>{/if}<button
              class="button small"
              disabled={loading}
              onclick={() => void refresh()}>Retry now</button
            ></span
          >
        {:else if strip.kind === 'incompatible'}<Glyph mark="failed" /><span
            ><b>This UI and Ren speak different API versions.</b> One was updated without the other.</span
          ><span class="strip-acts"
            ><button class="button small" onclick={() => location.reload()}>Reload</button></span
          >
        {:else if strip.kind === 'recorder'}<Glyph mark="warning" /><span
            ><b>Recording:</b> {strip.said}</span
          ><span class="strip-acts"
            ><button class="button small" onclick={() => (dismissed = [...dismissed, strip.said])}
              >Dismiss</button
            ><a class="button small" href="/system/recorder">Recorder →</a></span
          >
        {:else}<Glyph mark="idle" /><span
            ><b>Read only.</b> This UI can't start or steer sessions.</span
          ><span class="strip-acts muted">give it the control token to drive them</span>
        {/if}
        {#if place === 0 && strips.length > 1}<button
            class="strip-more mono"
            aria-expanded={edgeOpen}
            onclick={() => (edgeOpen = !edgeOpen)}
            >{edgeOpen ? 'fewer' : `+${strips.length - 1}`}</button
          >{/if}
      </div>
    {/each}
  </div>
  {#if page.data.runsControl}<RedlineLayer desk={redline} />{#if redline.open}<BriefingPanel
        desk={redline}
        {recipients}
        source={page.data.redlineSource}
        revision={health?.build.revision ?? null}
        {evidence}
      />{/if}{/if}
</div>
