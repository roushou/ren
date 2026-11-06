<script lang="ts">
  import { onMount } from 'svelte';
  import { goto, preloadCode, refreshAll } from '$app/navigation';
  import { page, navigating } from '$app/state';
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { Workbench, setWorkbench } from '#lib/workbench.svelte.ts';
  import { LiveConnection } from '#lib/live.svelte.ts';
  import { num, short, bytes, clock } from '#lib/api.ts';
  import { captureState } from '#lib/display.ts';
  import type { SnapshotError } from '#lib/snapshots.ts';
  import Icon from '#lib/Icon.svelte';
  import ThemeControl from '#lib/ThemeControl.svelte';
  import '#lib/style.css';
  import type { LayoutProps } from './$types';
  let { data, children }: LayoutProps = $props();
  const workbench = setWorkbench(new Workbench());
  const gateway = createSnapshot(() => data.gateway);
  const health = $derived(gateway.current.value?.health);
  const storage = $derived(gateway.current.value?.storage);
  const view = $derived(
    page.route.id === '/recorder'
      ? 'recorder'
      : page.route.id === '/sessions'
        ? 'sessions'
        : 'traffic',
  );
  const sessionId = $derived(page.params.id ?? null);
  const session = $derived(page.data.traffic?.value?.session);
  const sessionCount = $derived(page.data.sessions?.value?.length ?? 0);
  const selected = $derived(workbench.selected);
  const pageTitle = $derived(
    page.error
      ? 'Page unavailable'
      : view === 'recorder'
        ? 'Recorder & storage'
        : view === 'sessions'
          ? 'Sessions'
          : sessionId
            ? 'Session activity'
            : 'Traffic',
  );
  const gatewayOrigin = $derived(page.data.gatewayUrl);
  const omissions = $derived(health?.recorder.omissions);
  const omittedBytes = $derived(
    omissions
      ? omissions.policy_limit_bytes +
          omissions.queue_bytes_bytes +
          omissions.queue_slots_bytes +
          omissions.storage_failure_bytes
      : 0,
  );
  const loss = $derived(!!omissions && (omittedBytes > 0 || omissions.unrecorded_calls > 0));
  const degraded = $derived(
    !!health &&
      (health.status !== 'ok' ||
        health.recorder.storage_errors > 0 ||
        health.recorder.retention_errors > 0),
  );
  const loading = $derived(workbench.manual || !!navigating.to);
  let refreshError = $state('');
  const failure = $derived<SnapshotError | null>(
    gateway.current.error ??
      page.data.traffic?.error ??
      page.data.sessions?.error ??
      page.data.recorder?.error ??
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
    for (const route of ['/traffic', '/sessions', '/recorder', '/sessions/[id]'] as const)
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

<svelte:head><title>TGV · {pageTitle}</title></svelte:head>
<div class="app-shell">
  <header class="workbench-header">
    <a class="brand" href="/traffic" aria-label="TGV home"
      ><Icon name="terminal" size={17} /><strong>TGV</strong></a
    >
    <nav class="workbench-tabs" aria-label="Workbench views">
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
        class:active={view === 'sessions' || !!sessionId}
        aria-current={view === 'sessions' || !!sessionId ? 'page' : undefined}
        ><Icon name="sessions" size={14} />Sessions</a
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
    <span class="gateway-address mono" title={gatewayOrigin}
      >{gatewayOrigin.replace(/^https?:\/\//, '')}</span
    >
    <ThemeControl />
  </header>
  <div class="workbench-toolbar">
    <h1>
      {pageTitle}{#if sessionId}<span class="mono heading-id"
          >{short(session?.native_id ?? sessionId)}</span
        >{/if}
    </h1>
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
          : loss
            ? omittedBytes > 0
              ? `${bytes(omittedBytes)} capture omitted`
              : `${num(omissions?.unrecorded_calls)} unrecorded calls`
            : 'No capture loss recorded'}</button
    >
    <span class="status-context mono"
      >{selected && view === 'traffic'
        ? `${short(selected.id)} · ${captureState(selected).toLowerCase()}`
        : view === 'recorder'
          ? `${num(storage?.usage.calls)} calls retained`
          : view === 'sessions'
            ? `${sessionCount}${sessionCount === 100 ? '+' : ''} sessions loaded`
            : `${workbench.callCount} calls on page · ${workbench.activeCount} in progress`}</span
    >
    <span
      class="snapshot-time mono"
      title={updated ? new Date(updated).toLocaleString() : undefined}
      >{loading ? 'Refreshing…' : updated ? `Snapshot ${clock(updated)}` : 'No snapshot'}</span
    >
  </footer>
</div>
