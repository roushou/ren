<script lang="ts">
  import { onMount } from 'svelte';
  import { beforeNavigate, goto, preloadCode, refreshAll } from '$app/navigation';
  import { page, navigating } from '$app/state';
  import { createSettled, createSnapshot } from '#lib/snapshot.svelte.ts';
  import { Workbench, setWorkbench } from '#lib/workbench.svelte.ts';
  import { LiveConnection } from '#lib/live.svelte.ts';
  import { num, short, clock } from '#lib/api.ts';
  import { captureState } from '#lib/display.ts';
  import type { SnapshotError } from '#lib/snapshots.ts';
  import Icon, { type IconName } from '#lib/Icon.svelte';
  import ThemeControl from '#lib/ThemeControl.svelte';
  import ConnectClient from '#lib/ConnectClient.svelte';
  import AddressToggle from '#lib/AddressToggle.svelte';
  import { addressView } from '#lib/address.svelte.ts';
  import { GatewayPlace, type Network } from '#lib/place.ts';
  import { RecorderReport } from '#lib/recorder.ts';
  import { ReleaseLine } from '#lib/release.ts';
  import { PageEvidence } from '#lib/work.ts';
  import CopyButton from '#lib/CopyButton.svelte';
  import DevToolbar from '#lib/DevToolbar.svelte';
  import RedlineLayer from 'redline/RedlineLayer.svelte';
  import BriefingPanel from '#lib/BriefingPanel.svelte';
  import { RedlineDesk } from 'redline/desk';
  import { Recipients } from '#lib/briefing.svelte.ts';
  import { Views } from '#lib/view.ts';
  import { Arrival } from '#lib/arrival.ts';
  import type { RecordingsData } from '#lib/loaders.ts';
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
      evidence.error(args.map((arg) => (arg instanceof Error ? arg.message : String(arg))).join(' '));
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
  // The gateway says what kind of instance it is; the UI only shows it.
  const development = $derived(health?.instance.environment === 'development');
  /** The tag on the wordmark of an instance that is not production. */
  const marked = $derived.by(() => {
    switch (health?.instance.environment) {
      case 'development':
        return { tag: 'dev', what: 'A development gateway, not the installed service' };
      case 'canary':
        return { tag: 'canary', what: "The canary: main's tip on a copy of production's history" };
      case 'preview':
        return {
          tag: 'preview',
          what: "A preview: one workspace's branch on a copy of production's history",
        };
      default:
        return null;
    }
  });
  const storage = $derived(gateway.current.value?.storage);
  /** Where the release stands, once the factory has run against this gateway. */
  const release = $derived.by(() => {
    const view = gateway.current.value?.release;
    return view ? ReleaseLine.of(view) : null;
  });
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
  const recordingCount = $derived(
    reads.of<RecordingsData>(page.data.recordings)?.value?.recordings.length ?? 0,
  );
  const selected = $derived(workbench.selected);
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
    problem: () => error,
  };
  // The views' code is small and static: fetch it once up front, so switching
  // views waits only for data (which hovering a tab also starts).
  onMount(() => {
    for (const route of [
      '/',
      '/live',
      '/traffic',
      '/recordings',
      '/usage',
      '/recorder',
      '/recordings/[id]',
      '/recordings/[id]/report',
      '/turns/[id]',
      '/search',
      '/sessions',
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

<svelte:head><title>Ren - {pageTitle}</title></svelte:head>
<div class="app-shell">
  <header class="workbench-header">
    <a
      class="brand"
      class:marked
      href="/"
      aria-label={marked ? `Ren overview, ${marked.tag} gateway` : 'Ren overview'}
      title={marked?.what}
      ><span class="flaps" aria-hidden="true"
        ><span class="flap">R</span><span class="flap">E</span><span class="flap">N</span
        >{#if marked}<span class="instance-badge">{marked.tag}</span>{/if}</span
      ></a
    >
    <nav class="workbench-tabs" aria-label="Workbench views" bind:this={tabs}>
      <a
        href="/"
        data-sveltekit-preload-data="hover"
        class:active={tab === 'overview'}
        aria-current={tab === 'overview' ? 'page' : undefined}
        ><Icon name="monitor" size={14} />Overview</a
      >
      <a
        href="/live"
        data-sveltekit-preload-data="hover"
        class:active={tab === 'live'}
        aria-current={tab === 'live' ? 'page' : undefined}><Icon name="pulse" size={14} />Live</a
      >
      <a
        href="/sessions"
        class:active={tab === 'sessions'}
        aria-current={tab === 'sessions' ? 'page' : undefined}
        ><Icon name="spark" size={14} />Sessions</a
      >
      <a
        href="/traffic"
        data-sveltekit-preload-data="hover"
        class:active={tab === 'traffic'}
        aria-current={tab === 'traffic' ? 'page' : undefined}
        ><Icon name="logs" size={14} />Traffic</a
      >
      <a
        href="/recordings"
        data-sveltekit-preload-data="hover"
        class:active={tab === 'recordings'}
        aria-current={tab === 'recordings' ? 'page' : undefined}
        ><Icon name="sessions" size={14} />Recordings</a
      >
      <a
        href="/search"
        data-sveltekit-preload-data="hover"
        class:active={tab === 'search'}
        aria-current={tab === 'search' ? 'page' : undefined}
        ><Icon name="search" size={14} />Search</a
      >
      <a
        href="/usage"
        data-sveltekit-preload-data="hover"
        class:active={tab === 'usage'}
        aria-current={tab === 'usage' ? 'page' : undefined}><Icon name="chart" size={14} />Usage</a
      >
      <a
        href="/tools"
        class:active={tab === 'tools'}
        aria-current={tab === 'tools' ? 'page' : undefined}><Icon name="tool" size={14} />Tools</a
      >
      <a
        href="/work"
        class:active={tab === 'work'}
        aria-current={tab === 'work' ? 'page' : undefined}><Icon name="pencil" size={14} />Work</a
      >
      <a
        href="/recorder"
        data-sveltekit-preload-data="hover"
        class:active={tab === 'recorder'}
        aria-current={tab === 'recorder' ? 'page' : undefined}
        ><Icon name="storage" size={14} />Recorder{#if loss || degraded}<span
            class="warning"
            aria-label="Recording needs attention">!</span
          >{/if}</a
      >
    </nav>
    <button
      class="gateway-place mono"
      class:public={place.network === 'public'}
      popovertarget="connect-panel"
      title="Gateway on {place.node ??
        GatewayPlace.networkLabel(place.network)}, reached over {GatewayPlace.networkLabel(
        place.network,
      )}. {error ? 'The management API is not answering. ' : ''}Connect a client"
      ><span class="status-dot" class:error={!!error} class:neutral={!updated && !error}
      ></span>{#if place.node && place.network !== 'loopback'}<b>{place.node}</b><span
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
        <span class="mono">{shownOrigin}</span>{#if place.maskable}<AddressToggle />{/if}<CopyButton
          text={gatewayOrigin}
          label="Copy the gateway address"
        />
      </header>
      <ConnectClient layout="list" {gatewayOrigin} />
      <p><Icon name="shield" size={12} /> Each client keeps its own sign-in; Ren forwards it.</p>
    </div>
    <ThemeControl />
  </header>
  {#if navigating.to}<div
      class="nav-progress"
      role="progressbar"
      aria-label="Loading view"
    ></div>{/if}
  <main class="workspace" class:navigating={!!navigating.to} aria-busy={!!navigating.to}>
    {@render children()}
  </main>
  <footer class="statusline" class:redlining={redline.open} aria-label="Gateway status">
    <span
      class="read-only"
      title={page.data.runsControl
        ? 'History is read-only; this UI starts and drives sessions'
        : 'This UI only reads: give it the control token to drive sessions'}
      >{page.data.runsControl ? 'DRIVES SESSIONS' : 'READ ONLY'}</span
    >
    <span
      class="connection-state"
      title={error ||
        'Last successful management API snapshot; does not establish upstream availability.'}
      ><span class="status-dot" class:error={!!error} class:neutral={!updated}></span>{error
        ? incompatible
          ? 'API version mismatch'
          : 'Snapshot failed'
        : updated
          ? 'API reachable'
          : 'Connecting…'}</span
    >
    <button
      class="recorder-status"
      class:warning={loss || degraded}
      onclick={() => goto('/recorder')}
      title="Recorder counters since startup; open diagnostics"
      ><Icon name={loss || degraded ? 'alert' : 'storage'} size={13} />{!health
        ? 'Recorder unknown'
        : degraded
          ? 'Recorder errors recorded'
          : capture?.kind === 'lost'
            ? RecorderReport.lossText(capture)
            : 'No capture loss recorded'}</button
    >
    {#if release}<span
        class="release-status"
        class:warning={release.tone === 'warning'}
        class:error={release.tone === 'bad'}
        title={release.detail}>{release.text}</span
      >{/if}
    <span class="status-context mono"
      >{selected && (view === 'traffic' || view === 'recording')
        ? `${short(selected.id)} · ${captureState(selected, gateway.current.value?.storage.capture?.body_limit_bytes ?? null).toLowerCase()}`
        : view === 'recorder'
          ? `${num(storage?.usage.calls)} calls retained`
          : view === 'recordings'
            ? `${recordingCount}${recordingCount === 100 ? '+' : ''} recordings loaded`
            : view === 'traffic' || view === 'recording'
              ? `${workbench.callCount} calls on page · ${workbench.activeCount} in progress`
              : `${num(storage?.usage.calls)} calls retained`}</span
    >
    <span
      class="snapshot-time mono"
      title={updated ? new Date(updated).toLocaleString() : undefined}
      >{loading ? 'Refreshing…' : updated ? `Snapshot ${clock(updated)}` : 'No snapshot'}</span
    >
    {#if page.data.runsControl}<button
        class="redline-toggle"
        class:on={redline.open}
        data-redline
        aria-pressed={redline.open}
        onpointerenter={() => recipients.prepare()}
        onfocus={() => recipients.prepare()}
        onclick={() => redline.toggle()}
        title="Mark what's wrong on this page for an agent to fix (m)"
        ><Icon name="pencil" size={12} />Redline{#if redline.marks.length}<span
            class="redline-count">{redline.marks.length}</span
          >{/if}</button
      >{/if}
  </footer>
  {#if development && page.data.devTraffic}<DevToolbar />{/if}
  {#if page.data.runsControl}<RedlineLayer desk={redline} />{#if redline.open}<BriefingPanel
        desk={redline}
        {recipients}
        source={page.data.redlineSource}
        revision={health?.build.revision ?? null}
        {evidence}
      />{/if}{/if}
</div>
