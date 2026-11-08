<script lang="ts">
  import { flushSync, onMount, tick } from 'svelte';
  import { goto, onNavigate, preloadCode, refreshAll } from '$app/navigation';
  import { page, navigating } from '$app/state';
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { Workbench, setWorkbench } from '#lib/workbench.svelte.ts';
  import { LiveConnection } from '#lib/live.svelte.ts';
  import { num, short, clock } from '#lib/api.ts';
  import { captureState } from '#lib/display.ts';
  import type { SnapshotError } from '#lib/snapshots.ts';
  import Icon from '#lib/Icon.svelte';
  import ThemeControl from '#lib/ThemeControl.svelte';
  import ConnectClient from '#lib/ConnectClient.svelte';
  import { RecorderReport } from '#lib/recorder.ts';
  import CopyButton from '#lib/CopyButton.svelte';
  import '#lib/style.css';
  import type { LayoutProps } from './$types';
  let { data, children }: LayoutProps = $props();
  const workbench = setWorkbench(new Workbench());
  // The scope bar renders the current page's snippet, but the page owns its
  // state. Leaving for another page, the snippet is dropped before that
  // page's data arrives, or it would render the old page against new data.
  // The new page shows its own scope once it mounts.
  onNavigate((navigation) => {
    if (navigation.from?.route.id === navigation.to?.route.id) return;
    workbench.scope = undefined;
    flushSync();
  });
  const gateway = createSnapshot(() => data.gateway);
  const health = $derived(gateway.current.value?.health);
  const storage = $derived(gateway.current.value?.storage);
  const view = $derived(
    page.route.id === '/'
      ? 'overview'
      : page.route.id === '/live'
        ? 'live'
        : page.route.id === '/recorder'
          ? 'recorder'
          : page.route.id === '/usage'
            ? 'usage'
            : page.route.id === '/sessions'
              ? 'sessions'
              : page.route.id === '/sessions/[id]/report'
                ? 'report'
                : page.route.id === '/turns/[id]'
                  ? 'turn'
                  : page.route.id === '/search'
                    ? 'search'
                    : 'traffic',
  );
  /** A session page's ID; reports and turns belong under Sessions too. */
  const sessionId = $derived(page.route.id === '/sessions/[id]' ? (page.params.id ?? null) : null);
  const underSessions = $derived(
    view === 'sessions' || view === 'report' || view === 'turn' || !!sessionId,
  );
  /** When the scope does not fit on one line it folds behind a button. */
  let scopeOpen = $state(false);
  let folded = $state(false);
  let toolbar = $state<HTMLElement>();
  let scopeElement = $state<HTMLElement>();
  /** The scope's width on one line, measured while it is shown inline. */
  let natural = 0;
  function fit() {
    if (!toolbar || !scopeElement) return;
    if (!folded) {
      // The content's own width: its groups and the 24px between each.
      const groups = [...scopeElement.children] as HTMLElement[];
      natural =
        groups.reduce((sum, group) => sum + group.offsetWidth, 0) +
        Math.max(0, groups.length - 1) * 24;
    }
    const actions = toolbar.querySelector<HTMLElement>('.page-actions')?.offsetWidth ?? 0;
    const padding = 24 + 12;
    folded = !!workbench.scope && natural > toolbar.clientWidth - actions - padding;
    if (!folded) scopeOpen = false;
  }
  $effect(() => {
    // A new view's scope is measured inline first.
    void workbench.scope;
    folded = false;
    void tick().then(fit);
  });
  onMount(() => {
    const observer = new ResizeObserver(() => fit());
    if (toolbar) observer.observe(toolbar);
    return () => observer.disconnect();
  });
  const sessionCount = $derived(page.data.sessions?.value?.sessions.length ?? 0);
  const selected = $derived(workbench.selected);
  const pageTitle = $derived(
    page.error
      ? 'Page unavailable'
      : view === 'overview'
        ? 'Overview'
        : view === 'live'
          ? 'Live'
          : view === 'recorder'
            ? 'Recorder & storage'
            : view === 'usage'
              ? 'Usage'
              : view === 'sessions'
                ? 'Sessions'
                : view === 'report'
                  ? 'Session report'
                  : view === 'turn'
                    ? 'Turn'
                    : view === 'search'
                      ? 'Search'
                      : sessionId
                        ? 'Session activity'
                        : 'Traffic',
  );
  const gatewayOrigin = $derived(page.data.gatewayUrl);
  // Bytes past the capture policy are left out by design: never loss.
  const capture = $derived(health ? RecorderReport.capture(health.recorder) : null);
  const loss = $derived(capture?.kind === 'lost');
  const degraded = $derived(!!health && (health.status !== 'ok' || capture?.kind === 'failing'));
  const loading = $derived(workbench.manual || !!navigating.to);
  let refreshError = $state('');
  const failure = $derived<SnapshotError | null>(
    gateway.current.error ??
      page.data.traffic?.error ??
      page.data.live?.error ??
      page.data.sessions?.error ??
      page.data.recorder?.error ??
      page.data.report?.error ??
      page.data.turn?.error ??
      page.data.search?.error ??
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
  // The views' code is small and static: fetch it once up front, so switching
  // views waits only for data (which hovering a tab also starts).
  onMount(() => {
    for (const route of [
      '/',
      '/live',
      '/traffic',
      '/sessions',
      '/usage',
      '/recorder',
      '/sessions/[id]',
      '/sessions/[id]/report',
      '/turns/[id]',
      '/search',
    ] as const)
      void preloadCode(route);
  });
  onMount(() => {
    live = new LiveConnection({ refresh: reload, reload });
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

<svelte:head><title>TGV - {pageTitle}</title></svelte:head>
<div class="app-shell">
  <header class="workbench-header">
    <a class="brand" href="/" aria-label="TGV overview"
      ><span class="flaps" aria-hidden="true"
        ><span class="flap">T</span><span class="flap">G</span><span class="flap">V</span></span
      ></a
    >
    <nav class="workbench-tabs" aria-label="Workbench views">
      <a
        href="/"
        data-sveltekit-preload-data="hover"
        class:active={view === 'overview'}
        aria-current={view === 'overview' ? 'page' : undefined}
        ><Icon name="monitor" size={14} />Overview</a
      >
      <a
        href="/live"
        data-sveltekit-preload-data="hover"
        class:active={view === 'live'}
        aria-current={view === 'live' ? 'page' : undefined}><Icon name="pulse" size={14} />Live</a
      >
      <a
        href="/traffic"
        data-sveltekit-preload-data="hover"
        class:active={view === 'traffic' && !sessionId}
        aria-current={view === 'traffic' && !sessionId ? 'page' : undefined}
        ><Icon name="logs" size={14} />Traffic</a
      >
      <a
        href="/sessions"
        data-sveltekit-preload-data="hover"
        class:active={underSessions}
        aria-current={underSessions ? 'page' : undefined}
        ><Icon name="sessions" size={14} />Sessions</a
      >
      <a
        href="/search"
        data-sveltekit-preload-data="hover"
        class:active={view === 'search'}
        aria-current={view === 'search' ? 'page' : undefined}
        ><Icon name="search" size={14} />Search</a
      >
      <a
        href="/usage"
        data-sveltekit-preload-data="hover"
        class:active={view === 'usage'}
        aria-current={view === 'usage' ? 'page' : undefined}><Icon name="chart" size={14} />Usage</a
      >
      <a
        href="/recorder"
        data-sveltekit-preload-data="hover"
        class:active={view === 'recorder'}
        aria-current={view === 'recorder' ? 'page' : undefined}
        ><Icon name="storage" size={14} />Recorder{#if loss || degraded}<span
            class="warning"
            aria-label="Recording needs attention">!</span
          >{/if}</a
      >
    </nav>
    <button
      class="gateway-address mono"
      popovertarget="connect-panel"
      title="Connect a client to {gatewayOrigin}"
      >{gatewayOrigin.replace(/^https?:\/\//, '')}<Icon name="chevron" size={11} /></button
    >
    <div id="connect-panel" class="connect-panel" popover="auto" aria-label="Connect a client">
      <header>
        <strong>Connect a client</strong>
        <span class="mono">{gatewayOrigin}</span><CopyButton
          text={gatewayOrigin}
          label="Copy the gateway address"
        />
      </header>
      <ConnectClient layout="list" {gatewayOrigin} />
      <p><Icon name="shield" size={12} /> Each client keeps its own sign-in; TGV forwards it.</p>
    </div>
    <ThemeControl />
  </header>
  <div class="workbench-toolbar" class:folded class:scope-open={scopeOpen} bind:this={toolbar}>
    <!-- The nav tab names the view; the bar shows its scope. -->
    <h1 class="sr-only">{pageTitle}</h1>
    {#if workbench.scope && folded}<button
        class="button small scope-toggle"
        aria-expanded={scopeOpen}
        aria-controls="scope"
        onclick={() => (scopeOpen = !scopeOpen)}><Icon name="filter" size={13} /> Scope</button
      >{/if}
    <div class="scope" id="scope" bind:this={scopeElement}>
      {#if workbench.scope}{@render workbench.scope()}{:else}<span class="scope-title"
          >{pageTitle}</span
        >{/if}
    </div>
    <div class="page-actions">
      <label class="toggle-control"
        ><input type="checkbox" bind:checked={workbench.automatic} /><span class="switch"
        ></span><span>Live updates</span></label
      >
      <button
        class="icon-button"
        onclick={() => void refresh()}
        disabled={loading}
        aria-label="Refresh workspace"
        title="Refresh workspace"
        ><span class:spinning={loading}><Icon name="refresh" size={14} /></span></button
      >
    </div>
  </div>

  {#if error}<div class="notice danger" role="alert">
      <Icon name="alert" size={14} /><span>{error} Displayed data may be out of date.</span><button
        class="text-button"
        onclick={() => void refresh()}>Retry</button
      >
    </div>{/if}
  {#if navigating.to}<div
      class="nav-progress"
      role="progressbar"
      aria-label="Loading view"
    ></div>{/if}
  <main class="workspace" class:navigating={!!navigating.to} aria-busy={!!navigating.to}>
    {@render children()}
  </main>
  <footer class="statusline" aria-label="Gateway status">
    <span class="read-only">READ ONLY</span>
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
    <span class="status-context mono"
      >{selected && view === 'traffic'
        ? `${short(selected.id)} · ${captureState(selected).toLowerCase()}`
        : view === 'recorder'
          ? `${num(storage?.usage.calls)} calls retained`
          : view === 'sessions'
            ? `${sessionCount}${sessionCount === 100 ? '+' : ''} sessions loaded`
            : view === 'traffic'
              ? `${workbench.callCount} calls on page · ${workbench.activeCount} in progress`
              : `${num(storage?.usage.calls)} calls retained`}</span
    >
    <span
      class="snapshot-time mono"
      title={updated ? new Date(updated).toLocaleString() : undefined}
      >{loading ? 'Refreshing…' : updated ? `Snapshot ${clock(updated)}` : 'No snapshot'}</span
    >
  </footer>
</div>
