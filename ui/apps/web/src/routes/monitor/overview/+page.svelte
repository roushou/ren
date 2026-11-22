<script lang="ts">
  /*
    Overview: is model traffic healthy, and what went wrong in the last day.
    Each provider with its failures by hour and what is left of its
    allowance; the day's failures folded by kind; what is live now; secrets
    seen in requests; and the day against the one before. Totals, hours,
    models and users are Usage's; the gateway's own health is the bottom
    edge's.
  */
  import Toolbar from '#lib/Toolbar.svelte';
  import { onMount } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import { page } from '$app/state';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { ManagementApi, num, providerName, type CallId } from '#lib/api.ts';
  import { span } from '#lib/display.ts';
  import { Board, LivePlace } from '#lib/board.ts';
  import { ago, DayChange, Failures, Providers } from '#lib/overview.ts';
  import { Allowances, type Allowance } from '#lib/allowance.ts';
  import { harnessLabel, searchFieldLabel, secretKindLabel } from '#lib/activity.ts';
  import ConnectClient from '#lib/ConnectClient.svelte';
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();

  const arrival = createArrival(() => data.overview);
  const shown = $derived(arrival.current);
  const pending = $derived(shown.state === 'pending');
  const overview = $derived(shown.state === 'ready' ? shown.snapshot : null);
  const current = $derived(overview?.value ?? null);

  // Elapsed times tick between refreshes.
  let now = $state(Date.now());
  onMount(() => {
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  const live = $derived(current?.live ?? null);
  const board = $derived(
    live
      ? new Board({
          recordings: live.recordings,
          calls: live.calls,
          turns: live.turns,
          now: Math.max(now, live.read),
          windowMs: LivePlace.windows['15m'].ms,
          focus: null,
          pins: [],
          only: null,
        })
      : null,
  );

  const providers = $derived(
    current
      ? Providers.rows({
          now: current.now,
          models: current.usage.models,
          failed: current.failed,
          recent: live?.calls ?? [],
        })
      : [],
  );
  const failures = $derived(
    current
      ? Failures.of({
          now: current.now,
          failed: current.failed,
          stopped: current.stopped,
          bookkeeping: current.bookkeeping,
        })
      : [],
  );
  const changes = $derived(current ? DayChange.rows(current.usage, current.previous) : []);
  const totals = $derived(current?.usage.totals ?? null);

  /** Each provider's allowance, read from its newest call's response head. */
  const api = new ManagementApi();
  /** `reading` until its call is read; null when its response stated no limit. */
  const allowances = new SvelteMap<CallId, Allowance | null | 'reading'>();
  $effect(() => {
    for (const row of providers) {
      const id = row.newest;
      if (!id || allowances.has(id)) continue;
      allowances.set(id, 'reading');
      api.call(id).then(
        (detail) =>
          allowances.set(id, Allowances.read(detail.attempts.at(-1)?.limit_headers ?? [])),
        () => allowances.set(id, null),
      );
    }
  });
  const allowanceOf = (id: CallId | null) => (id ? (allowances.get(id) ?? 'reading') : null);

  /** The model calls open now, newest first. */
  const open = $derived(
    (live?.calls ?? [])
      .filter((call) => call.state === 'open')
      .toSorted((a, b) => b.started_at - a.started_at)
      .slice(0, 5),
  );
  const clientOf = (recording: string | null) => {
    const found = live?.recordings.find((each) => each.id === recording);
    return found ? Board.clientName(Board.harness(found.source)) : 'Unattributed';
  };
  const lastCall = $derived(Math.max(0, ...(current?.recordings ?? []).map((s) => s.last_seen)));
  const secrets = $derived((current?.exposures ?? []).slice(0, 4));

  /** `Sat 21, 14:00`. */
  const when = (ms: number) => {
    const date = new Date(ms);
    const day = date.toLocaleDateString([], { weekday: 'short' });
    const time = date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    return `${day} ${date.getDate()}, ${time}`;
  };
  /** A provider's hour, drawn by how many failed in it. */
  const heat = (count: number) =>
    count === 0 ? 'none' : count < 3 ? 'few' : count < 10 ? 'some' : 'many';

  /** Nothing recorded in the last day: the page leads with how to connect a client. */
  const firstRun = $derived(!!current && current.usage.totals.calls === 0 && !board?.tiles.length);
  const OUTLINE_ROWS = 4;
</script>

<Toolbar>
  {#snippet scope()}
    <span class="ov-range">
      <span class="cap strong">Last 24 hours</span>
      {#if current}<span class="meta"
          >{when(current.now - 24 * 3_600_000)} – {when(current.now)}</span
        >{/if}
    </span>
    {#if totals}<span class="meta ov-sum"
        >{num(totals.calls)} calls · {num(totals.errors)} failed · {DayChange.share(
          totals.calls ? totals.errors / totals.calls : null,
        )}</span
      >{/if}
  {/snippet}
</Toolbar>
<div class="overview">
  {#if !pending && !current}
    <div class="empty-state" role="alert">
      <div class="empty-icon"><Icon name="alert" size={26} /></div>
      <h2>The overview could not be read</h2>
      <p>{overview?.error?.message ?? 'Loading…'}</p>
    </div>
  {:else if firstRun}
    <section class="ov-first" aria-labelledby="first-title">
      <h2 id="first-title">Connect a client to start recording</h2>
      <p class="muted">
        Nothing passed through Ren in the last 24 hours. Start a session through this gateway; its
        calls show up here, on Live, and in Traffic as they happen.
      </p>
      <ConnectClient layout="box" gatewayOrigin={page.data.gatewayUrl} />
      <span class="empty-footnote"
        ><Icon name="shield" size={12} /> Uses your client’s existing sign-in</span
      >
    </section>
  {:else}
    <div class="ov-grid">
      <div class="ov-column">
        <section class="ov-block" aria-labelledby="providers-title" aria-busy={pending}>
          <header class="ov-head">
            <h2 id="providers-title" class="cap">
              Providers{#if current}{' · '}{providers.length}{/if}
            </h2>
          </header>
          <div class="ov-providers" role="table" aria-label="Providers">
            <div class="ov-provider ov-columns cap" role="row">
              <span></span><span role="columnheader">Provider</span><span
                class="num"
                role="columnheader">Calls</span
              ><span class="num" role="columnheader">Failed</span><span
                class="num"
                role="columnheader">Rate</span
              ><span role="columnheader">By hour</span><span role="columnheader">Headroom</span
              ><span class="num" role="columnheader">Last failure</span>
            </div>
            {#if pending}
              {#each { length: OUTLINE_ROWS }, row (row)}
                <div class="ov-provider" role="row">
                  <span></span><Skeleton width="8em" /><span></span><span></span><span
                  ></span><Skeleton width="100%" /><span></span><span></span>
                </div>
              {/each}
            {:else}
              {#each providers as row (row.provider)}
                {@const allowance = allowanceOf(row.newest)}
                <a class="ov-provider" role="row" href={row.href}>
                  <Glyph mark={Providers.mark(row.state)} label={true} />
                  <span class="ov-name">
                    <span>{row.name}</span>
                    <span class="meta ellipsis">{row.models.join(' · ')}</span>
                  </span>
                  <span class="num">{num(row.calls)}</span>
                  <span class="num" class:bad={row.state !== 'healthy' && row.failed > 0}
                    >{row.failed ? num(row.failed) : '—'}</span
                  >
                  <span class="num" class:bad={row.state !== 'healthy' && row.failed > 0}
                    >{row.failed ? DayChange.share(row.rate) : '—'}</span
                  >
                  <span class="ov-hours" aria-label="Failures by hour, oldest first">
                    {#each row.hours as count, index (index)}<span
                        class="ov-hour {heat(count)}"
                        title="{count} failed"
                      ></span>{/each}
                  </span>
                  <span class="ov-allowance">
                    {#if allowance === 'reading'}
                      <Skeleton width="9em" />
                    {:else if allowance}
                      <svg viewBox="0 0 100 1" preserveAspectRatio="none" aria-hidden="true">
                        <rect class="track" width="100" height="1" />
                        <rect
                          class:low={allowance.left < 0.2}
                          class="left"
                          width={allowance.left * 100}
                          height="1"
                        />
                      </svg>
                      <span class="meta">{allowance.text}</span>
                    {:else}
                      <span class="meta">not reported</span>
                    {/if}
                  </span>
                  <span class="meta num" class:bad={row.state !== 'healthy'}
                    >{row.last ? `${row.last.ending} · ${ago(row.last.at, now)}` : '—'}</span
                  >
                </a>
              {:else}
                <p class="ov-none meta">No calls in the last 24 hours.</p>
              {/each}
            {/if}
          </div>
        </section>

        <section class="ov-block grow" aria-labelledby="failures-title" aria-busy={pending}>
          <header class="ov-head">
            <h2 id="failures-title" class="cap">
              Failures{#if totals}{' · '}{num(totals.errors)}{/if}
            </h2>
          </header>
          {#if pending}
            {#each { length: OUTLINE_ROWS }, row (row)}
              <div class="ov-failure">
                <span></span><Skeleton width="10em" /><Skeleton width="14em" /><span></span><span
                ></span>
              </div>
            {/each}
          {:else}
            {#each failures as group (group.key)}
              <a class="ov-failure {group.severity}" href={group.href}>
                <Glyph
                  mark={group.severity === 'error'
                    ? 'failed'
                    : group.severity === 'warning'
                      ? 'warning'
                      : 'info'}
                  label={true}
                />
                <span class="ov-failure-name"
                  ><span class="title">{group.name}</span><span class="meta">×{group.count}</span
                  ></span
                >
                <span class="meta ellipsis">{group.models}</span>
                <svg
                  class="ov-ticks"
                  viewBox="0 0 100 10"
                  preserveAspectRatio="none"
                  aria-label="When they happened over the day"
                >
                  <line x1="0" y1="9.5" x2="100" y2="9.5" />
                  {#each group.marks as mark, index (index)}<rect
                      x={mark * 100 - 0.4}
                      y="1"
                      width="0.8"
                      height="8.5"
                    />{/each}
                </svg>
                <span class="meta num">{ago(group.at, now)}</span>
              </a>
            {:else}
              <p class="ov-none meta">No failures in the last 24 hours.</p>
            {/each}
          {/if}
        </section>
      </div>

      <div class="ov-column side">
        <section class="ov-block" aria-labelledby="now-title" aria-busy={pending}>
          <header class="ov-head">
            <h2 id="now-title" class="cap">Now</h2>
            <a class="meta ov-link" href="/monitor/live">Live →</a>
          </header>
          <div class="ov-now">
            <div>
              <span class="ov-figure">{board ? num(board.inFlight) : '—'}</span><span class="meta"
                >in flight</span
              >
            </div>
            <div>
              <span class="ov-figure">{board ? num(board.tiles.length + board.overflow) : '—'}</span
              ><span class="meta">sessions active</span>
            </div>
            <div>
              <span class="ov-figure">{lastCall ? span(Math.max(0, now - lastCall)) : '—'}</span
              ><span class="meta">since last call</span>
            </div>
          </div>
          {#if open.length}
            <ul class="ov-open">
              {#each open as call (call.id)}
                <li>
                  <a href="/monitor/traffic?call={encodeURIComponent(call.id)}">
                    <Glyph mark="responding" />
                    <span class="ov-name-line"
                      ><span class="title">{clientOf(call.recording)}</span><span
                        class="meta ellipsis">{call.model ?? providerName(call.provider)}</span
                      ></span
                    >
                    <span class="meta">{span(Math.max(0, now - call.started_at))}</span>
                  </a>
                </li>
              {/each}
            </ul>
          {/if}
        </section>

        {#if secrets.length}
          <section class="ov-block flush" aria-labelledby="secrets-title">
            <header class="ov-head pad">
              <h2 id="secrets-title" class="cap">
                Secrets seen · {Failures.count(current?.exposures ?? [])}
              </h2>
            </header>
            <ul class="ov-secrets">
              {#each secrets as secret (secret.call + secret.fingerprint)}
                <li>
                  <a href="/monitor/traffic?call={encodeURIComponent(secret.call)}">
                    <Glyph mark="warning" label={true} />
                    <span class="ov-name">
                      <span>{secretKindLabel(secret.kind)}</span>
                      <span class="meta ellipsis"
                        >{secret.client ? `${harnessLabel(secret.client)} · ` : ''}in {searchFieldLabel(
                          secret.field,
                        ).toLowerCase()}</span
                      >
                    </span>
                    <span class="meta">{ago(secret.started_at, now)}</span>
                  </a>
                </li>
              {/each}
            </ul>
          </section>
        {/if}

        <section class="ov-block grow" aria-labelledby="compare-title" aria-busy={pending}>
          <header class="ov-head">
            <h2 id="compare-title" class="cap">Against the day before</h2>
          </header>
          {#each changes as change (change.label)}
            <div class="ov-change">
              <span>{change.label}</span>
              <span class="num">{change.value}</span>
              <span class="meta num" class:bad={change.worse}>{change.delta ?? '—'}</span>
            </div>
          {/each}
        </section>
      </div>
    </div>
  {/if}
</div>

<style>
  .overview {
    flex: 1;
    min-height: 0;
    overflow: auto;
    background: var(--canvas);
  }
  .ov-range {
    display: inline-flex;
    align-items: baseline;
    gap: 12px;
  }
  .ov-sum {
    margin-left: 16px;
  }
  .cap {
    margin: 0;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    font-weight: 400;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .cap.strong {
    color: var(--ink);
  }
  .meta {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11px;
  }
  .num {
    font-family: var(--mono);
    text-align: right;
  }
  .bad {
    color: var(--error);
  }
  .ellipsis {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .ov-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 360px;
    gap: 1px;
    min-height: 100%;
    background: var(--line);
  }
  .ov-column {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .ov-block {
    background: var(--surface);
    padding: 12px 16px 14px;
  }
  .ov-block.flush,
  .ov-column:not(.side) .ov-block {
    padding: 12px 0 6px;
  }
  .grow {
    flex: 1;
  }
  .ov-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  .ov-column:not(.side) .ov-head,
  .ov-head.pad {
    padding: 0 16px 8px;
  }
  .ov-link {
    color: var(--accent);
  }
  .ov-none {
    margin: 4px 16px 8px;
  }
  .ov-provider {
    display: grid;
    grid-template-columns: 14px 160px 70px 60px 56px minmax(0, 1fr) 190px 110px;
    column-gap: 12px;
    align-items: center;
    height: 44px;
    padding: 0 16px;
    border-top: 1px solid var(--line-subtle);
    color: inherit;
  }
  a.ov-provider:hover,
  .ov-failure:hover,
  .ov-open a:hover,
  .ov-secrets a:hover {
    background: var(--row-hover);
  }
  .ov-columns {
    height: 26px;
    font-size: 10px;
  }
  .ov-name {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-weight: 500;
  }
  .ov-name .meta {
    font-weight: 400;
  }
  .ov-hours {
    display: flex;
    gap: 2px;
    height: 14px;
  }
  .ov-hour {
    flex: 1;
    background: var(--surface-inset);
  }
  .ov-hour.few {
    background: color-mix(in srgb, var(--error) 35%, var(--surface-inset));
  }
  .ov-hour.some {
    background: color-mix(in srgb, var(--error) 65%, var(--surface-inset));
  }
  .ov-hour.many {
    background: var(--error);
  }
  .ov-allowance {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .ov-allowance svg {
    width: 100%;
    height: 4px;
  }
  .ov-allowance .track {
    fill: var(--surface-inset);
  }
  .ov-allowance .left {
    fill: var(--success);
  }
  .ov-allowance .left.low {
    fill: var(--warning);
  }
  .ov-failure {
    display: grid;
    grid-template-columns: 14px minmax(0, 260px) minmax(0, 1fr) 240px 90px;
    column-gap: 12px;
    align-items: center;
    height: 32px;
    padding: 0 16px;
    border-top: 1px solid var(--line-subtle);
    color: inherit;
  }
  .ov-failure-name {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .title {
    font-weight: 500;
  }
  .ov-failure.error .title {
    color: var(--error);
  }
  .ov-failure.warning .title {
    color: var(--warning);
  }
  .ov-ticks {
    width: 100%;
    height: 12px;
  }
  .ov-ticks line {
    stroke: var(--line);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  .ov-failure.error .ov-ticks rect {
    fill: var(--error);
  }
  .ov-failure.warning .ov-ticks rect {
    fill: var(--warning);
  }
  .ov-failure.neutral .ov-ticks rect {
    fill: var(--muted);
  }
  .ov-now {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px;
    margin-top: 10px;
  }
  .ov-now > div {
    display: flex;
    flex-direction: column;
  }
  .ov-figure {
    font-family: var(--mono);
    font-size: 22px;
    font-weight: 500;
  }
  .ov-open,
  .ov-secrets {
    margin: 12px 0 0;
    padding: 0;
    list-style: none;
  }
  .ov-open {
    border-top: 1px solid var(--line-subtle);
  }
  .ov-open a {
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
    height: 28px;
    border-bottom: 1px solid var(--line-subtle);
    color: inherit;
  }
  .ov-name-line {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
  }
  .ov-secrets {
    margin: 0;
  }
  .ov-secrets a {
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
    height: 40px;
    padding: 0 16px;
    border-top: 1px solid var(--line-subtle);
    color: inherit;
  }
  .ov-change {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 64px;
    gap: 8px;
    align-items: center;
    height: 28px;
    border-bottom: 1px solid var(--line-subtle);
  }
  .ov-first {
    max-width: 640px;
    margin: 48px auto;
    padding: 24px;
    background: var(--surface);
    border: 1px solid var(--line);
  }
  @media (max-width: 1100px) {
    .ov-grid {
      grid-template-columns: 1fr;
    }
    .ov-provider {
      grid-template-columns: 14px minmax(0, 1fr) 60px 50px minmax(0, 1fr) 100px;
    }
    .ov-provider > :nth-child(5),
    .ov-provider > :nth-child(7) {
      display: none;
    }
    .ov-failure {
      grid-template-columns: 14px minmax(0, 1fr) 120px 70px;
    }
    .ov-failure > :nth-child(3) {
      display: none;
    }
  }
</style>
