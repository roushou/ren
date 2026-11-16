<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import { page } from '$app/state';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { num, providerMark } from '#lib/api.ts';
  import { compactOrDash, SEPARATOR } from '#lib/display.ts';
  import {
    cacheShare,
    parseRange,
    usageMeasures,
    usagePoints,
    usageRanges,
    type UsageMeasure,
  } from '#lib/usage.ts';
  import UsageChart from '#lib/UsageChart.svelte';
  import Icon from '#lib/Icon.svelte';
  import RecordingName from '#lib/RecordingName.svelte';
  import Segmented, { type SegmentedOption } from '#lib/Segmented.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import SkeletonRows from '#lib/SkeletonRows.svelte';
  import type { Column } from '#lib/columns.ts';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  const arrival = createArrival(() => data.usage);
  const shown = $derived(arrival.current);
  /** The range's reads are on their way: the page draws its outline. */
  const pending = $derived(shown.state === 'pending');
  const snapshot = $derived(shown.state === 'ready' ? shown.snapshot : null);
  const current = $derived(snapshot?.value ?? null);
  const details = $derived(current?.details ?? new Map());
  const range = $derived(current?.range ?? parseRange(page.url.searchParams.get('range')));
  const figures: Column[] = [
    { label: 'Calls', numeric: true },
    { label: 'Errors', numeric: true },
    { label: 'Tokens in', numeric: true },
    { label: 'Out', numeric: true },
  ];
  const modelColumns: Column[] = [{ label: 'Model' }, ...figures];
  const recordingColumns: Column[] = [{ label: 'Recording', class: 'fill' }, ...figures];
  const userColumns: Column[] = [{ label: 'User' }, ...figures];
  const totals = $derived(current?.usage.totals);
  const points = $derived(current ? usagePoints(current.usage.hours, range, current.now) : []);
  let measure = $state<UsageMeasure>('input');
  const measureOptions: SegmentedOption<UsageMeasure>[] = usageMeasures.map((item) => ({
    value: item.measure,
    label: item.label,
  }));
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
  // Shown once a named user made calls in the range; all callers are listed then.
  const users = $derived(current?.usage.users ?? []);
  const named = $derived(users.some((u) => u.user !== null));
  // Recordings active in the range, most input first.
  const recordings = $derived(
    (current?.recordings ?? [])
      .filter(
        (s) =>
          !current || range === 'all' || s.last_seen >= current.now - (byHour ? 864e5 : 7 * 864e5),
      )
      .sort((a, b) => (b.tokens.input ?? 0) - (a.tokens.input ?? 0))
      .slice(0, 5),
  );
</script>

{#snippet head(columns: readonly Column[])}
  <thead
    ><tr
      >{#each columns as column (column.label)}<th class:numeric={column.numeric}>{column.label}</th
        >{/each}</tr
    ></thead
  >
{/snippet}

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

<Toolbar {scope} />
<div class="usage-page">
  <div class="console">
    {#if !current && !pending}
      <div class="empty-state">
        <div class="empty-icon"><Icon name="alert" size={26} /></div>
        <h2>Usage could not be read</h2>
        <p>{snapshot?.error?.message ?? 'Loading…'}</p>
      </div>
    {:else}
      <section class="usage-tiles" aria-label="Totals" aria-busy={pending}>
        <div class="usage-tile">
          <span>Calls</span><strong class="mono"
            >{#if pending}<Skeleton size="figure" width="5em" />{:else}{num(
                totals?.calls,
              )}{/if}</strong
          >
        </div>
        <div class="usage-tile">
          <span>Failed work calls</span><strong class="mono" class:error-count={!!totals?.errors}
            >{#if pending}<Skeleton size="figure" width="3em" />{:else}{num(
                totals?.errors,
              )}{/if}</strong
          ><small
            >{#if pending}<Skeleton width="7em" />{:else}{errorRate === null
                ? '—'
                : `${(errorRate * 100).toFixed(1)}% of calls`}{#if current?.bookkeepingErrors}{SEPARATOR}{num(
                  current.bookkeepingErrors,
                )}{current.bookkeepingErrors >= 100 ? '+' : ''} failed bookkeeping{/if}{/if}</small
          >
        </div>
        <div class="usage-tile">
          <span>Tokens in</span><strong class="mono"
            >{#if pending}<Skeleton size="figure" width="4em" />{:else}{compactOrDash(
                totals?.tokens.input ?? null,
              )}{/if}</strong
          ><small
            >{#if pending}<Skeleton width="8em" />{:else}{share === null
                ? '—'
                : `${Math.round(share * 100)}% read from cache`}{/if}</small
          >
        </div>
        <div class="usage-tile">
          <span>Tokens out</span><strong class="mono"
            >{#if pending}<Skeleton size="figure" width="4em" />{:else}{compactOrDash(
                totals?.tokens.output ?? null,
              )}{/if}</strong
          >
        </div>
      </section>

      <section class="panel usage-panel" aria-labelledby="usage-chart-title">
        <header class="panel-heading">
          <h2 id="usage-chart-title">{measureLabel} per {byHour ? 'hour' : 'day'}</h2>
          <Segmented label="Measure" options={measureOptions} bind:value={measure} />
        </header>
        {#if pending}
          <div class="chart-outline"><Skeleton size="block" width="100%" /></div>
        {:else if totals?.calls}
          <UsageChart {points} {measure} label={measureLabel} {period} />
        {:else}
          <p class="quiet-empty">No calls in this range.</p>
        {/if}
      </section>

      <div class="usage-columns usage-fill">
        <section class="panel usage-panel" aria-labelledby="usage-models-title">
          <header class="panel-heading"><h2 id="usage-models-title">By model</h2></header>
          {#if models.length || pending}
            <table class="usage-list">
              {@render head(modelColumns)}
              <tbody>
                {#if pending}<SkeletonRows columns={modelColumns} rows={6} />{/if}
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
                    <td class="mono numeric">{compactOrDash(model.tokens.input)}</td>
                    <td class="mono numeric">{compactOrDash(model.tokens.output)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else}<p class="quiet-empty">No calls in this range.</p>{/if}
        </section>

        <section class="panel usage-panel" aria-labelledby="usage-recordings-title">
          <header class="panel-heading">
            <h2 id="usage-recordings-title">Busiest recordings</h2>
            <span class="subtle-label">Totals over each recording's whole life</span>
          </header>
          {#if recordings.length || pending}
            <table class="usage-list">
              {@render head(recordingColumns)}
              <tbody>
                {#if pending}<SkeletonRows columns={recordingColumns} rows={5} />{/if}
                {#each recordings as recording (recording.id)}
                  <tr>
                    <td class="fill"
                      ><a class="row-link" href={`/recordings/${encodeURIComponent(recording.id)}`}
                        ><RecordingName {recording} detail={details.get(recording.id)} /></a
                      ></td
                    >
                    <td class="mono numeric">{num(recording.call_count)}</td>
                    <td
                      class="mono numeric"
                      class:error-count={!!recording.error_count}
                      class:zero={recording.error_count === 0}>{num(recording.error_count)}</td
                    >
                    <td class="mono numeric">{compactOrDash(recording.tokens.input)}</td>
                    <td class="mono numeric">{compactOrDash(recording.tokens.output)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else}<p class="quiet-empty">No recordings were active in this range.</p>{/if}
        </section>

        {#if named}<section class="panel usage-panel" aria-labelledby="usage-users-title">
            <header class="panel-heading">
              <h2 id="usage-users-title">By user</h2>
              <span class="subtle-label">Who made the calls, as the gateway identified them</span>
            </header>
            <table class="usage-list">
              {@render head(userColumns)}
              <tbody>
                {#each users as user (user.user)}
                  <tr>
                    <td
                      >{#if user.user}<a
                          class="row-link"
                          href={`/traffic?user=${encodeURIComponent(user.user)}`}>{user.user}</a
                        >{:else}<span
                          class="subtle-label"
                          title="Unlisted Tailscale logins, unidentified callers, and calls with no stored caller"
                          >Not a named user</span
                        >{/if}</td
                    >
                    <td class="mono numeric">{num(user.calls)}</td>
                    <td
                      class="mono numeric"
                      class:error-count={!!user.errors}
                      class:zero={!user.errors}>{num(user.errors)}</td
                    >
                    <td class="mono numeric">{compactOrDash(user.tokens.input)}</td>
                    <td class="mono numeric">{compactOrDash(user.tokens.output)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </section>{/if}
      </div>
    {/if}
  </div>
</div>

<style>
  /* The page fills the window: the bottom panels take whatever height is left. */
  .console {
    min-height: 100%;
  }
  .usage-fill {
    flex: 1 0 auto;
  }
  /* The chart's outline holds the chart's own height, so nothing moves when it arrives. */
  .chart-outline {
    height: 224px;
    padding: 12px;
  }
</style>
