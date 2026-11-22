<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import { page } from '$app/state';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { num, providerMark } from '#lib/api.ts';
  import { compactOrDash } from '#lib/display.ts';
  import {
    cacheShare,
    HourGrid,
    measureValue,
    parseRange,
    usageMeasures,
    usagePoints,
    usageRanges,
    type UsageMeasure,
  } from '#lib/usage.ts';
  import UsageChart from '#lib/UsageChart.svelte';
  import Icon from '#lib/Icon.svelte';
  import Glyph from '#lib/Glyph.svelte';
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
  const modelColumns: Column[] = [
    { label: 'Model' },
    { label: 'Calls', numeric: true },
    { label: 'Errors', numeric: true },
    { label: 'Share', numeric: true },
    { label: 'Tokens in', numeric: true },
    { label: 'Out', numeric: true },
  ];
  const recordingColumns: Column[] = [{ label: 'Session', class: 'fill' }, ...figures];
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
  const byHour = $derived(range !== 'all');
  function period(start: number) {
    const date = new Date(start);
    return byHour
      ? {
          short:
            range === '24h'
              ? date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
              : `${date.toLocaleDateString([], { weekday: 'short' })} ${date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`,
          long: `${date.toLocaleDateString([], { month: 'short', day: 'numeric' })}, ${date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`,
        }
      : {
          short: date.toLocaleDateString([], { month: 'short', day: 'numeric' }),
          long: date.toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' }),
        };
  }
  const share = $derived(totals ? cacheShare(totals) : null);
  /** The measure over the range, and its busiest point. */
  const measureTotal = $derived(
    points.reduce((sum, point) => sum + measureValue(point, measure), 0),
  );
  const peak = $derived(
    points.reduce<(typeof points)[number] | null>(
      (best, point) =>
        measureValue(point, measure) > (best ? measureValue(best, measure) : 0) ? point : best,
      null,
    ),
  );
  const measureShort = (value: number) =>
    measure === 'input' || measure === 'output' ? compactOrDash(value) : num(value);
  /** Calls by local hour of day over the last week. */
  const grid = $derived(current ? HourGrid.of(current.usage.hours, current.now) : null);
  const errorRate = $derived(totals && totals.calls ? totals.errors / totals.calls : null);
  const models = $derived(current?.usage.models ?? []);
  /** Each model's share of the range's tokens in. */
  const modelInput = $derived(models.reduce((sum, model) => sum + (model.tokens.input ?? 0), 0));
  const topModelInput = $derived(Math.max(1, ...models.map((m) => m.tokens.input ?? 0)));
  // Shown once a named user made calls in the range; all callers are listed then.
  const users = $derived(current?.usage.users ?? []);
  const named = $derived(users.some((u) => u.user !== null));
  // Recordings active in the range, most input first.
  const recordings = $derived(
    (current?.recordings ?? [])
      .filter(
        (s) =>
          !current ||
          range === 'all' ||
          s.last_seen >= current.now - (range === '24h' ? 864e5 : 7 * 864e5),
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
        href={`/monitor/usage?range=${item.range}`}
        class:chosen={range === item.range}
        aria-current={range === item.range ? 'page' : undefined}>{item.label}</a
      >{/each}
  </nav>
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
      <div class="usage-top">
        <section class="panel usage-panel" aria-labelledby="usage-chart-title">
          <header class="panel-heading">
            <div class="chart-title">
              <h2 id="usage-chart-title" class="cap">
                {measureLabel} per {byHour ? 'hour' : 'day'}
              </h2>
              {#if !pending && totals?.calls}<span class="big mono"
                  >{measureShort(measureTotal)}</span
                >{#if peak}<span class="muted mono"
                    >peak {measureShort(measureValue(peak, measure))} · {period(peak.start)
                      .long}</span
                  >{/if}{/if}
            </div>
            <Segmented label="Measure" options={measureOptions} bind:value={measure} />
          </header>
          {#if pending}
            <div class="chart-outline"><Skeleton size="block" width="100%" /></div>
          {:else if totals?.calls}
            <UsageChart
              {points}
              {measure}
              label={measureLabel}
              {period}
              cached={measure === 'input'
                ? points.map((point) => point.tokens.cache_read ?? 0)
                : null}
            />
            {#if measure === 'input'}<p class="chart-key muted">
                <span><span class="swatch cached"></span>read from cache</span>
                <span><span class="swatch fresh"></span>uncached input</span>
              </p>{/if}
          {:else}
            <p class="quiet-empty">No calls in this range.</p>
          {/if}
        </section>
        <div class="usage-side">
          <section class="usage-tiles" aria-label="Totals" aria-busy={pending}>
            <div class="usage-tile">
              <span>Calls</span><strong class="mono"
                >{#if pending}<Skeleton size="figure" width="5em" />{:else}{num(
                    totals?.calls,
                  )}{/if}</strong
              ><small
                >{#if !pending && totals && points.length}{num(
                    Math.round(totals.calls / points.length),
                  )} / {byHour ? 'hour' : 'day'}{/if}</small
              >
            </div>
            <div class="usage-tile">
              <span>Failed work calls</span><strong class="mono failed-figure"
                >{#if totals?.errors}<Glyph mark="failed" label={true} />{/if}{#if pending}<Skeleton
                    size="figure"
                    width="3em"
                  />{:else}{num(totals?.errors)}{/if}</strong
              ><small class="sub"
                >{#if pending}<Skeleton width="7em" />{:else}<span title="Share of calls"
                    >{errorRate === null ? '—' : `${(errorRate * 100).toFixed(1)}%`}</span
                  >{#if current?.bookkeepingErrors}<span
                      class="chip mono"
                      title="Failed bookkeeping calls"
                      ><Glyph mark="warning" />{num(
                        current.bookkeepingErrors,
                      )}{current.bookkeepingErrors >= 100 ? '+' : ''}</span
                    >{/if}{/if}</small
              >
            </div>
            <div class="usage-tile">
              <span>Tokens in</span><strong class="mono"
                >{#if pending}<Skeleton size="figure" width="4em" />{:else}{compactOrDash(
                    totals?.tokens.input ?? null,
                  )}{/if}</strong
              ><small class="sub" title="Read from cache"
                >{#if pending}<Skeleton width="8em" />{:else if share === null}—{:else}<svg
                    class="meter"
                    viewBox="0 0 100 4"
                    preserveAspectRatio="none"
                    aria-hidden="true"
                    ><rect class="track" width="100" height="4" /><rect
                      class="fill cached"
                      width={share * 100}
                      height="4"
                    /></svg
                  >{Math.round(share * 100)}%{/if}</small
              >
            </div>
            <div class="usage-tile">
              <span>Tokens out</span><strong class="mono"
                >{#if pending}<Skeleton size="figure" width="4em" />{:else}{compactOrDash(
                    totals?.tokens.output ?? null,
                  )}{/if}</strong
              ><small
                >{#if !pending && totals?.calls && totals.tokens.output !== null}{compactOrDash(
                    Math.round(totals.tokens.output / totals.calls),
                  )} / call{/if}</small
              >
            </div>
          </section>
          {#if grid && grid.busiest > 0}
            <section class="panel usage-panel" aria-labelledby="usage-hours-title">
              <header class="panel-heading">
                <h2 id="usage-hours-title" class="cap">Calls by hour of day</h2>
                <span class="muted mono">local time</span>
              </header>
              <div class="hour-grid">
                {#each grid.rows as row (row.day)}
                  <span class="muted mono day"
                    >{new Date(row.day).toLocaleDateString([], { weekday: 'short' })}</span
                  >
                  <svg viewBox="0 0 240 10" preserveAspectRatio="none" aria-hidden="true"
                    >{#each row.cells as calls, hour (hour)}<rect
                        class="cell"
                        x={hour * 10 + 0.5}
                        y="0"
                        width="9"
                        height="10"
                        fill-opacity={calls ? 0.15 + 0.85 * (calls / grid.busiest) : 0.04}
                        ><title>{hour}:00 · {calls} calls</title></rect
                      >{/each}</svg
                  >
                {/each}
                <span></span>
                <span class="hours muted mono"
                  ><span>00</span><span>06</span><span>12</span><span>18</span><span>23</span></span
                >
              </div>
            </section>
          {/if}
        </div>
      </div>
      <section class="panel usage-panel models" aria-labelledby="usage-models-title">
        <header class="panel-heading">
          <h2 id="usage-models-title" class="cap">By model · share of tokens in</h2>
          <span class="muted mono">{models.length} models</span>
        </header>
        {#if modelInput > 0}<svg
            class="model-share"
            viewBox="0 0 100 6"
            preserveAspectRatio="none"
            aria-label="Each model's share of tokens in"
            >{#each models as model, index (`${model.provider}-${model.model}`)}<rect
                class="share-{index % 5}"
                x={(models.slice(0, index).reduce((sum, one) => sum + (one.tokens.input ?? 0), 0) /
                  modelInput) *
                  100}
                width={((model.tokens.input ?? 0) / modelInput) * 100}
                height="6"
                ><title
                  >{model.model ?? 'No model reported'}: {Math.round(
                    ((model.tokens.input ?? 0) / modelInput) * 100,
                  )}%</title
                ></rect
              >{/each}</svg
          >{/if}
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
                  <td class="mono numeric"
                    >{modelInput
                      ? `${Math.round(((model.tokens.input ?? 0) / modelInput) * 100)}%`
                      : '—'}</td
                  >
                  <td class="mono numeric">{compactOrDash(model.tokens.input)}</td>
                  <td class="mono numeric">{compactOrDash(model.tokens.output)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}<p class="quiet-empty">No calls in this range.</p>{/if}
      </section>

      <div class="usage-columns usage-fill">
        <section class="panel usage-panel" aria-labelledby="usage-recordings-title">
          <header class="panel-heading">
            <h2 id="usage-recordings-title" class="cap">Busiest sessions</h2>
          </header>
          {#if recordings.length || pending}
            <table class="usage-list">
              {@render head(recordingColumns)}
              <tbody>
                {#if pending}<SkeletonRows columns={recordingColumns} rows={5} />{/if}
                {#each recordings as recording (recording.id)}
                  <tr>
                    <td class="fill"
                      ><a
                        class="row-link"
                        href={`/monitor/sessions/${encodeURIComponent(recording.id)}`}
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
          {:else}<p class="quiet-empty">No sessions were active in this range.</p>{/if}
        </section>

        {#if named}<section class="panel usage-panel" aria-labelledby="usage-users-title">
            <header class="panel-heading">
              <h2 id="usage-users-title" class="cap">By user</h2>
            </header>
            <table class="usage-list">
              {@render head(userColumns)}
              <tbody>
                {#each users as user (user.user)}
                  <tr>
                    <td
                      >{#if user.user}<a
                          class="row-link"
                          href={`/monitor/traffic?user=${encodeURIComponent(user.user)}`}
                          >{user.user}</a
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
  .usage-top {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 300px;
    gap: 1px;
    background: var(--line);
  }
  .usage-side {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .usage-side .usage-tiles {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .usage-side .usage-panel {
    flex: 1;
  }
  .chart-title {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 10px;
  }
  .cap {
    margin: 0;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px !important;
    font-weight: 400;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .big {
    font-size: 22px;
  }
  .models {
    border-top: 1px solid var(--line);
  }
  @media (max-width: 1000px) {
    .usage-top {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .failed-figure {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .chart-key {
    display: flex;
    gap: 14px;
    margin: 0;
    padding: 0 16px 10px;
    font-size: 11px;
  }
  .chart-key > span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .swatch {
    width: 8px;
    height: 8px;
  }
  .swatch.cached {
    background: color-mix(in srgb, var(--success) 70%, transparent);
  }
  .swatch.fresh {
    background: var(--chart);
  }
  .hour-grid {
    display: grid;
    grid-template-columns: 36px minmax(0, 1fr);
    gap: 3px 10px;
    align-items: center;
    padding: 12px 16px;
    font-size: 10.5px;
  }
  .hour-grid svg {
    width: 100%;
    height: 12px;
  }
  .cell {
    fill: var(--chart);
  }
  .hours {
    display: flex;
    justify-content: space-between;
  }
  .model-share {
    display: block;
    width: 100%;
    height: 6px;
  }
  .share-0 {
    fill: var(--chart);
  }
  .share-1 {
    fill: var(--accent);
  }
  .share-2 {
    fill: var(--role-tool);
  }
  .share-3 {
    fill: var(--success);
  }
  .share-4 {
    fill: var(--muted);
  }
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
