<script lang="ts">
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { num, short, clock } from '#lib/api.ts';
  import { compact, compactOrDash, date, percentOrDash, span, sessionTone } from '#lib/display.ts';
  import { ContextChart, findingLine, rowPrompt, turnHref, turnRows } from '#lib/activity.ts';
  import RecordingName from '#lib/RecordingName.svelte';
  import FindingList from '#lib/FindingList.svelte';
  import Icon from '#lib/Icon.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import SkeletonRows from '#lib/SkeletonRows.svelte';
  import type { Column } from '#lib/columns.ts';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);
  const arrival = createArrival(() => data.report);
  const arrived = $derived(arrival.current);
  /** The report is on its way: the page draws its outline. */
  const pending = $derived(arrived.state === 'pending');
  const snapshot = $derived(arrived.state === 'ready' ? arrived.snapshot : null);
  const current = $derived(snapshot?.value ?? null);
  const report = $derived(current?.report);
  const totals = $derived(report?.totals);
  const session = $derived(current?.session);
  const rows = $derived(current ? turnRows(current.report, current.turns) : []);
  const findings = $derived(report ? report.findings.map(findingLine) : []);
  const chart = $derived(new ContextChart(rows));
  /** The chart's height in SVG units, drawn at the same height in pixels. */
  const CHART = 120;
  const turnColumns: Column[] = [
    { label: 'Started' },
    { label: 'Prompt', class: 'turn-prompt' },
    { label: 'Calls', numeric: true },
    { label: 'Idle before', numeric: true },
    { label: 'Context', numeric: true },
    { label: 'From cache', numeric: true },
    { label: 'Written', numeric: true },
    { label: 'Out', numeric: true },
  ];
</script>

<!-- A tile's figure or note, or its outline while the report is on its way. -->
{#snippet value(
  text: string | undefined,
  width: string,
  size: 'text' | 'figure' = 'text',
)}{#if text === undefined}<Skeleton {size} {width} />{:else}{text}{/if}{/snippet}

{#snippet scope()}
  <div class="scope-group scope-identity">
    {#if session}<RecordingName {session} detail={current?.detail} />{:else}<span
        class="session-dot tone-{sessionTone(data.sessionId)}"
      ></span><strong class="mono">{short(data.sessionId)}</strong>{/if}
    <span class="muted">Context and cache report</span>
  </div>
  <span
    class="scope-group usage-note"
    title="Tokens are what providers report. They are not subscription allowance or charges."
    ><Icon name="info" size={12} /> Provider-reported tokens</span
  >
  <div class="scope-group end">
    <a class="text-button" href={`/recordings/${encodeURIComponent(data.sessionId)}`}
      >Recording calls <Icon name="arrow" size={12} /></a
    ><a class="text-button" href="/recordings">All recordings <Icon name="arrow" size={12} /></a>
  </div>
{/snippet}

<div class="usage-page">
  <div class="console">
    {#if !report && !pending}
      <div class="empty-state">
        <div class="empty-icon"><Icon name="alert" size={26} /></div>
        <h2>The report could not be read</h2>
        <p>{snapshot?.error?.message ?? 'Loading…'}</p>
      </div>
    {:else}
      <section class="usage-tiles" aria-label="Totals" aria-busy={pending}>
        <div class="usage-tile">
          <span>Peak context</span><strong class="mono"
            >{@render value(totals && compact(totals.peak_context_tokens), '4em', 'figure')}</strong
          ><small>tokens in one call</small>
        </div>
        <div class="usage-tile">
          <span>Read from cache</span><strong class="mono"
            >{@render value(
              totals && percentOrDash(totals.cache_hit_percent),
              '3em',
              'figure',
            )}</strong
          ><small
            >{@render value(
              totals && `${compact(totals.cache_read_tokens)} of ${compact(totals.context_tokens)}`,
              '7em',
            )}</small
          >
        </div>
        <div class="usage-tile">
          <span>Written to cache</span><strong class="mono"
            >{@render value(
              totals && compact(totals.cache_written_tokens),
              '4em',
              'figure',
            )}</strong
          ><small>tokens</small>
        </div>
        <div class="usage-tile">
          <span>Output</span><strong class="mono"
            >{@render value(totals && compact(totals.output_tokens), '4em', 'figure')}</strong
          ><small>tokens</small>
        </div>
        <div class="usage-tile">
          <span>Calls</span><strong class="mono"
            >{@render value(totals && num(totals.calls), '3em', 'figure')}</strong
          ><small
            >{@render value(
              totals && `${num(totals.calls_with_usage)} reported usage`,
              '7em',
            )}</small
          >
        </div>
      </section>

      {#if pending}
        <section class="panel usage-panel" aria-labelledby="report-context-title">
          <header class="panel-heading">
            <h2 id="report-context-title">Context per turn</h2>
          </header>
          <div class="context-chart">
            <div class="chart-outline"><Skeleton size="block" width="100%" /></div>
          </div>
        </section>
      {:else if chart.bars.length > 1}
        <section class="panel usage-panel" aria-labelledby="report-context-title">
          <header class="panel-heading">
            <h2 id="report-context-title">Context per turn</h2>
            <span class="muted"
              >Oldest to newest · each bar is the context a turn ended with, its tick where it
              started · the model's limit is not reported, so none is drawn</span
            >
          </header>
          <div class="context-chart">
            <span class="context-scale mono">{compact(chart.max)}</span>
            <svg
              viewBox="0 0 1000 {CHART}"
              preserveAspectRatio="none"
              aria-label="Context per turn"
            >
              {#each [0.25, 0.5, 0.75] as at (at)}<line
                  class="grid"
                  x1="0"
                  y1={CHART * at}
                  x2="1000"
                  y2={CHART * at}
                />{/each}
              {#each chart.bars as bar (bar.turn)}
                <a href={turnHref(bar.turn)} class:flagged={bar.flagged}
                  ><title>{bar.title}</title><rect
                    class="bar"
                    x={bar.x}
                    width={bar.width}
                    y={CHART * (1 - bar.end)}
                    height={CHART * bar.end}
                  /><rect
                    class="start"
                    x={bar.x}
                    width={bar.width}
                    y={CHART * (1 - bar.start) - 1}
                    height="2"
                  /></a
                >
              {/each}
            </svg>
          </div>
        </section>
      {/if}

      <section class="panel usage-panel" aria-labelledby="report-findings-title">
        <header class="panel-heading">
          <h2 id="report-findings-title">Findings</h2>
          <span class="muted">Measured against fixed thresholds; causes are not inferred</span>
        </header>
        {#if pending}
          <ul class="finding-list">
            {#each ['62%', '48%'] as width (width)}
              <li>
                <Skeleton width="7em" /><span class="finding-detail"><Skeleton {width} /></span>
              </li>
            {/each}
          </ul>
        {:else if findings.length}
          <FindingList {findings} session={data.sessionId} page="session" />
        {:else}
          <p class="quiet-empty">No cache rebuilds or large context steps in this recording.</p>
        {/if}
      </section>

      <section class="panel usage-panel" aria-labelledby="report-turns-title">
        <header class="panel-heading">
          <h2 id="report-turns-title">Turns</h2>
          <span class="muted">Newest first · one user prompt each, as the client grouped them</span>
        </header>
        {#if rows.length || pending}
          <div class="table-scroll">
            <table class="usage-list turn-table">
              <thead
                ><tr
                  >{#each turnColumns as column (column.label)}<th class:numeric={column.numeric}
                      >{column.label}</th
                    >{/each}</tr
                ></thead
              >
              <tbody>
                {#if pending}<SkeletonRows columns={turnColumns} rows={8} />{/if}
                {#each rows as row (row.metrics.turn)}
                  {@const metrics = row.metrics}
                  <tr>
                    <td
                      ><span>{date(metrics.started_at)}</span><small
                        >{clock(metrics.started_at)}</small
                      ></td
                    >
                    <td class="turn-prompt">
                      <a class="row-link" href={turnHref(metrics.turn)}>{rowPrompt(row)}</a>
                      {#each row.findings as line, index (index)}<span class="chip {line.tone}"
                          >{line.title}</span
                        >{/each}
                    </td>
                    <td class="mono numeric"
                      >{num(metrics.call_count)}{#if row.summary?.error_count}<small
                          class="error-count"
                        >
                          · {row.summary.error_count} err</small
                        >{/if}</td
                    >
                    <td class="mono numeric"
                      >{metrics.idle_before_ms === null ? '—' : span(metrics.idle_before_ms)}</td
                    >
                    <td class="mono numeric"
                      >{compactOrDash(metrics.context_start_tokens)} → {compactOrDash(
                        metrics.context_end_tokens,
                      )}</td
                    >
                    <td class="mono numeric">{percentOrDash(metrics.cache_hit_percent)}</td>
                    <td class="mono numeric">{compact(metrics.cache_written_tokens)}</td>
                    <td class="mono numeric">{compact(metrics.output_tokens)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {:else}
          <p class="quiet-empty">
            This session has no turns. Turns come from the client's own grouping; Pi sends none.
          </p>
        {/if}
      </section>
      {#if report?.omitted_calls}<p class="quiet-empty">
          {num(report.omitted_calls)} older calls beyond the report's limit are not included.
        </p>{/if}
    {/if}
  </div>
</div>

<style>
  .context-chart {
    position: relative;
    padding: 18px 16px 12px;
  }
  .context-chart svg {
    display: block;
    width: 100%;
    height: 120px;
  }
  .context-scale {
    position: absolute;
    top: 3px;
    left: 16px;
    color: var(--muted);
    font-size: 10.5px;
  }
  .grid {
    stroke: var(--line);
    stroke-dasharray: 2 3;
    vector-effect: non-scaling-stroke;
  }
  .bar {
    fill: color-mix(in srgb, var(--chart) 45%, transparent);
  }
  a:hover .bar {
    fill: var(--chart);
  }
  .flagged .bar {
    fill: color-mix(in srgb, var(--warning) 60%, transparent);
  }
  .start {
    fill: var(--ink);
  }
  /* The chart's outline holds the chart's own height, so nothing moves when it arrives. */
  .chart-outline {
    height: 120px;
  }
</style>
