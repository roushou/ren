<script lang="ts">
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { num, providerMark, providerName, short } from '#lib/api.ts';
  import { compact, sessionTone } from '#lib/display.ts';
  import {
    cacheShare,
    usageMeasures,
    usagePoints,
    usageRanges,
    type UsageMeasure,
  } from '#lib/usage.ts';
  import UsageChart from '#lib/UsageChart.svelte';
  import Icon from '#lib/Icon.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);
  const snapshot = createSnapshot(() => data.usage);
  const current = $derived(snapshot.current.value);
  const range = $derived(current?.range ?? '7d');
  const totals = $derived(current?.usage.totals);
  const points = $derived(current ? usagePoints(current.usage.hours, range, current.now) : []);
  let measure = $state<UsageMeasure>('input');
  const measureLabel = $derived(
    usageMeasures.find((item) => item.measure === measure)?.label ?? 'Tokens in',
  );
  const byHour = $derived(range === '24h');
  function period(start: number) {
    const date = new Date(start);
    return byHour
      ? {
          short: date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }),
          long: `${date.toLocaleDateString([], { month: 'short', day: 'numeric' })}, ${date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`,
        }
      : {
          short: date.toLocaleDateString([], { month: 'short', day: 'numeric' }),
          long: date.toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' }),
        };
  }
  const share = $derived(totals ? cacheShare(totals) : null);
  const errorRate = $derived(totals && totals.calls ? totals.errors / totals.calls : null);
  const models = $derived(current?.usage.models ?? []);
  const topModelInput = $derived(Math.max(1, ...models.map((m) => m.tokens.input ?? 0)));
  // Sessions active in the range, most input first.
  const sessions = $derived(
    (current?.sessions ?? [])
      .filter(
        (s) =>
          !current || range === 'all' || s.last_seen >= current.now - (byHour ? 864e5 : 7 * 864e5),
      )
      .sort((a, b) => (b.tokens.input ?? 0) - (a.tokens.input ?? 0))
      .slice(0, 5),
  );
  const tokens = (value: number | null | undefined) =>
    value === null || value === undefined ? '—' : compact(value);
</script>

{#snippet scope()}
  <nav class="scope-group segmented" aria-label="Time range">
    {#each usageRanges as item}<a
        href={`/usage?range=${item.range}`}
        class:chosen={range === item.range}
        aria-current={range === item.range ? 'page' : undefined}>{item.label}</a
      >{/each}
  </nav>
  <span
    class="scope-group end usage-note"
    title="Tokens are what providers report; input includes cached context. They are not subscription allowance or charges."
    ><Icon name="info" size={12} /> Provider-reported tokens</span
  >
{/snippet}

<div class="usage-page">
  <div class="console">
    {#if !current}
      <div class="empty-state">
        <div class="empty-icon"><Icon name="alert" size={26} /></div>
        <h2>Usage could not be read</h2>
        <p>{snapshot.current.error?.message ?? 'Loading…'}</p>
      </div>
    {:else}
      <section class="usage-tiles" aria-label="Totals">
        <div class="usage-tile">
          <span>Calls</span><strong class="mono">{num(totals?.calls)}</strong>
        </div>
        <div class="usage-tile">
          <span>Errors</span><strong class="mono" class:error-count={!!totals?.errors}
            >{num(totals?.errors)}</strong
          ><small>{errorRate === null ? '—' : `${(errorRate * 100).toFixed(1)}% of calls`}</small>
        </div>
        <div class="usage-tile">
          <span>Tokens in</span><strong class="mono">{tokens(totals?.tokens.input)}</strong><small
            >{share === null ? '—' : `${Math.round(share * 100)}% read from cache`}</small
          >
        </div>
        <div class="usage-tile">
          <span>Tokens out</span><strong class="mono">{tokens(totals?.tokens.output)}</strong>
        </div>
      </section>

      <section class="panel usage-panel" aria-labelledby="usage-chart-title">
        <header class="panel-heading">
          <h2 id="usage-chart-title">{measureLabel} per {byHour ? 'hour' : 'day'}</h2>
          <div class="segmented" aria-label="Measure">
            {#each usageMeasures as item}<button
                class:chosen={measure === item.measure}
                aria-pressed={measure === item.measure}
                onclick={() => (measure = item.measure)}>{item.label}</button
              >{/each}
          </div>
        </header>
        {#if totals?.calls}
          <UsageChart {points} {measure} label={measureLabel} {period} />
        {:else}
          <p class="quiet-empty">No calls in this range.</p>
        {/if}
      </section>

      <div class="usage-columns">
        <section class="panel usage-panel" aria-labelledby="usage-models-title">
          <header class="panel-heading"><h2 id="usage-models-title">By model</h2></header>
          {#if models.length}
            <table class="usage-list">
              <thead
                ><tr
                  ><th>Model</th><th class="numeric">Calls</th><th class="numeric">Errors</th><th
                    class="numeric">Tokens in</th
                  ><th class="numeric">Out</th></tr
                ></thead
              >
              <tbody>
                {#each models as model (`${model.provider}-${model.model}`)}
                  <tr>
                    <td>
                      <span class="model-line"
                        ><span class="provider-mark tiny {model.provider}"
                          >{providerMark(model.provider)}</span
                        ><span class="model-name">{model.model ?? 'No model reported'}</span></span
                      >
                      <svg
                        class="usage-meter"
                        viewBox="0 0 100 4"
                        preserveAspectRatio="none"
                        aria-hidden="true"
                        ><rect class="usage-meter-track" width="100" height="4" /><rect
                          class="usage-bar"
                          width={(100 * (model.tokens.input ?? 0)) / topModelInput}
                          height="4"
                        /></svg
                      >
                    </td>
                    <td class="mono numeric">{num(model.calls)}</td>
                    <td
                      class="mono numeric"
                      class:error-count={!!model.errors}
                      class:zero={!model.errors}>{num(model.errors)}</td
                    >
                    <td class="mono numeric">{tokens(model.tokens.input)}</td>
                    <td class="mono numeric">{tokens(model.tokens.output)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else}<p class="quiet-empty">No calls in this range.</p>{/if}
        </section>

        <section class="panel usage-panel" aria-labelledby="usage-sessions-title">
          <header class="panel-heading">
            <h2 id="usage-sessions-title">Busiest sessions</h2>
            <span class="subtle-label">Totals over each session's whole life</span>
          </header>
          {#if sessions.length}
            <table class="usage-list">
              <thead
                ><tr
                  ><th>Session</th><th class="numeric">Calls</th><th class="numeric">Errors</th><th
                    class="numeric">Tokens in</th
                  ><th class="numeric">Out</th></tr
                ></thead
              >
              <tbody>
                {#each sessions as session (session.id)}
                  <tr>
                    <td
                      ><a class="row-link" href={`/sessions/${encodeURIComponent(session.id)}`}
                        ><span class="session-dot tone-{sessionTone(session.id)}"></span><span
                          class="mono">{short(session.native_id)}</span
                        ><small>{providerName(session.provider)}</small></a
                      ></td
                    >
                    <td class="mono numeric">{num(session.call_count)}</td>
                    <td
                      class="mono numeric"
                      class:error-count={!!session.error_count}
                      class:zero={session.error_count === 0}>{num(session.error_count)}</td
                    >
                    <td class="mono numeric">{tokens(session.tokens.input)}</td>
                    <td class="mono numeric">{tokens(session.tokens.output)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else}<p class="quiet-empty">No sessions were active in this range.</p>{/if}
        </section>
      </div>
    {/if}
  </div>
</div>
