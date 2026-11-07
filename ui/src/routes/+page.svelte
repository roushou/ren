<script lang="ts">
  import { onMount } from 'svelte';
  import { page } from '$app/state';
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { bytes, num, providerMark, providerName, short } from '#lib/api.ts';
  import { compactOrDash, sessionTone, streamChip } from '#lib/display.ts';
  import { activeSessions, ago, healthLine, problems } from '#lib/overview.ts';
  import { cacheShare, usagePoints } from '#lib/usage.ts';
  import UsageChart from '#lib/UsageChart.svelte';
  import CopyButton from '#lib/CopyButton.svelte';
  import Icon from '#lib/Icon.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);
  const overview = createSnapshot(() => data.overview);
  const gateway = createSnapshot(() => data.gateway);
  const current = $derived(overview.current.value);
  const health = $derived(
    healthLine(
      gateway.current.value?.health ?? null,
      gateway.current.value?.storage ?? null,
      gateway.current.error?.message ?? null,
      current?.exposures ?? null,
    ),
  );
  // Elapsed times tick between refreshes.
  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });
  const sessions = $derived(current ? activeSessions(current.sessions, now) : []);
  const lastCall = $derived(Math.max(0, ...(current?.sessions ?? []).map((s) => s.last_seen)));
  const attention = $derived(current ? problems(current.errors, current.stopped) : []);
  const totals = $derived(current?.usage.totals);
  const share = $derived(totals ? cacheShare(totals) : null);
  const points = $derived(current ? usagePoints(current.usage.hours, '24h', current.now) : []);
  const hour = (start: number) => {
    const date = new Date(start);
    const time = date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    return {
      short: time,
      long: `${date.toLocaleDateString([], { month: 'short', day: 'numeric' })}, ${time}`,
    };
  };
  const seconds = (ms: number) => {
    const s = Math.max(0, Math.round(ms / 1000));
    return s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${s % 60} s`;
  };
  let client = $state<'claude' | 'codex'>('claude');
  const origin = $derived(page.data.gatewayUrl);
  const local = $derived(origin === 'http://127.0.0.1:8787' || origin === 'http://localhost:8787');
  const command = $derived(`tgv run ${client}${local ? '' : ` --gateway ${origin}`}`);
</script>

{#snippet scope()}
  <div class="scope-group overview-health" aria-label="Health">
    {#each health as item}{#if item.href}<a
          class="chip {item.tone}"
          href={item.href}
          title={item.title}>{item.label}</a
        >{:else}<span class="chip {item.tone}" title={item.title}>{item.label}</span>{/if}{/each}
  </div>
{/snippet}

<div class="overview">
  <div class="console">
    {#if !current}
      <div class="empty-state" role="alert">
        <div class="empty-icon"><Icon name="alert" size={26} /></div>
        <h2>The overview could not be read</h2>
        <p>{overview.current.error?.message ?? 'Loading…'}</p>
      </div>
    {:else}
      <div class="overview-grid">
        <section class="panel usage-panel" aria-labelledby="live-title">
          <header class="panel-heading">
            <h2 id="live-title">
              <span class="status-dot" class:live={current.active.length > 0}></span> Live now
            </h2>
            <a class="text-button" href="/traffic?status=active"
              >In progress <Icon name="arrow" size={12} /></a
            >
          </header>
          {#if current.active.length}
            <ul class="overview-list">
              {#each current.active as call (call.id)}
                <li>
                  <a class="overview-row" href={`/traffic?call=${call.id}`}>
                    {#if call.session}<span class="session-dot tone-{sessionTone(call.session)}"
                      ></span>{/if}
                    <span class="provider-mark tiny {call.provider}"
                      >{providerMark(call.provider)}</span
                    >
                    <span class="model-name">{call.model ?? call.endpoint}</span>
                    <span class="overview-meta mono">
                      {call.response
                        ? `${bytes(call.response.stored_bytes)} so far · `
                        : ''}{seconds(now - call.started_at)}
                    </span>
                  </a>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="quiet-empty">
              Nothing in flight.{lastCall ? ` Last call ${ago(lastCall, now)}.` : ''}
            </p>
          {/if}
          <h3 class="overview-subhead">Sessions active in the last 15 minutes</h3>
          {#if sessions.length}
            <ul class="overview-list">
              {#each sessions as session (session.id)}
                <li>
                  <a class="overview-row" href={`/sessions/${encodeURIComponent(session.id)}`}>
                    <span class="session-dot tone-{sessionTone(session.id)}"></span>
                    <span class="mono">{short(session.native_id)}</span>
                    <small>{providerName(session.provider)}</small>
                    <span class="overview-meta mono"
                      >{num(session.call_count)} calls · {compactOrDash(session.tokens.input)} in · {ago(
                        session.last_seen,
                        now,
                      )}</span
                    >
                  </a>
                </li>
              {/each}
            </ul>
          {:else}<p class="quiet-empty">None.</p>{/if}
        </section>

        <section class="panel usage-panel" aria-labelledby="attention-title">
          <header class="panel-heading">
            <h2 id="attention-title">Needs attention</h2>
            <span class="subtle-label">Last 24 hours</span>
          </header>
          {#if attention.length}
            <ul class="overview-list">
              {#each attention as { call, reason } (call.id)}
                {@const stream = streamChip(call)}
                <li>
                  <a class="overview-row" href={`/traffic?call=${call.id}`}>
                    <span class="chip {reason.tone}">{reason.label}</span>
                    {#if stream.tone !== 'success'}<span
                        class="chip {stream.tone}"
                        title={stream.title}>{stream.label}</span
                      >{/if}
                    <span class="model-name">{call.model ?? call.endpoint}</span>
                    <span class="overview-meta mono">{ago(call.started_at, now)}</span>
                  </a>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="quiet-empty">
              <span class="chip success">All clear</span> No errors or stopped calls in the last 24 hours.
            </p>
          {/if}
        </section>
      </div>

      <section class="panel usage-panel" aria-labelledby="day-title">
        <header class="panel-heading">
          <h2 id="day-title">Last 24 hours</h2>
          <a class="text-button" href="/usage?range=24h">Usage <Icon name="arrow" size={12} /></a>
        </header>
        <div class="usage-tiles overview-tiles">
          <div class="usage-tile">
            <span>Calls</span><strong class="mono">{num(totals?.calls)}</strong>
          </div>
          <div class="usage-tile">
            <span>Errors</span><strong class="mono" class:error-count={!!totals?.errors}
              >{num(totals?.errors)}</strong
            >
          </div>
          <div class="usage-tile">
            <span>Tokens in</span><strong class="mono"
              >{compactOrDash(totals?.tokens.input ?? null)}</strong
            ><small>{share === null ? '—' : `${Math.round(share * 100)}% from cache`}</small>
          </div>
          <div class="usage-tile">
            <span>Tokens out</span><strong class="mono"
              >{compactOrDash(totals?.tokens.output ?? null)}</strong
            >
          </div>
        </div>
        {#if totals?.calls}<UsageChart
            {points}
            measure="input"
            label="Tokens in"
            period={hour}
          />{/if}
      </section>

      <section class="overview-connect" aria-label="Connect a client">
        <Icon name="terminal" size={14} />
        <span class="muted">Start a session through TGV:</span>
        <div class="segmented">
          <button class:chosen={client === 'claude'} onclick={() => (client = 'claude')}
            >Claude Code</button
          ><button class:chosen={client === 'codex'} onclick={() => (client = 'codex')}
            >Codex</button
          >
        </div>
        <code>{command}</code><CopyButton text={command} label="Copy launch command" />
      </section>
    {/if}
  </div>
</div>
