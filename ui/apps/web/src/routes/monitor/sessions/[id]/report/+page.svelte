<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import SessionHead from '#lib/SessionHead.svelte';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { num, clock } from '#lib/api.ts';
  import { compact, compactOrDash, date, percentOrDash, span } from '#lib/display.ts';
  import { ContextChart, findingLine, rowPrompt, turnHref, turnRows } from '#lib/activity.ts';
  import FindingList from '#lib/FindingList.svelte';
  import Glyph from '#lib/Glyph.svelte';
  import { Marks } from '#lib/marks.ts';
  import Icon from '#lib/Icon.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import SkeletonRows from '#lib/SkeletonRows.svelte';
  import type { Column } from '#lib/columns.ts';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  const arrival = createArrival(() => data.report);
  const desks = createArrival(() => data.desks);
  /** Where the session opens on its desk, when Ren drives it. */
  const desk = $derived.by(() => {
    return desks.snapshot?.value?.get(data.recordingId) ?? null;
  });
  const arrived = $derived(arrival.current);
  /** The report is on its way: the page draws its outline. */
  const pending = $derived(arrived.state === 'pending');
  const snapshot = $derived(arrived.state === 'ready' ? arrived.snapshot : null);
  const current = $derived(snapshot?.value ?? null);
  const report = $derived(current?.report);
  const totals = $derived(report?.totals);
  const recording = $derived(current?.recording);
  const rows = $derived(current ? turnRows(current.report, current.turns) : []);
  const findings = $derived(report ? report.findings.map(findingLine) : []);
  const chart = $derived(new ContextChart(rows));
  /** The recording's peak context: each turn's range is drawn against it. */
  const peak = $derived(Math.max(1, totals?.peak_context_tokens ?? 0));
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

<Toolbar />
<div class="usage-page">
  <div class="console">
    <SessionHead
      recordingId={data.recordingId}
      {recording}
      detail={current?.detail ?? null}
      {desk}
      view="report"
      now={Date.now()}
    />
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
          >
        </div>
        <div class="usage-tile">
          <span>Read from cache</span><strong class="mono"
            >{@render value(
              totals && percentOrDash(totals.cache_hit_percent),
              '3em',
              'figure',
            )}</strong
          >
        </div>
        <div class="usage-tile">
          <span>Written to cache</span><strong class="mono"
            >{@render value(
              totals && compact(totals.cache_written_tokens),
              '4em',
              'figure',
            )}</strong
          >
        </div>
        <div class="usage-tile">
          <span>Output</span><strong class="mono"
            >{@render value(totals && compact(totals.output_tokens), '4em', 'figure')}</strong
          >
        </div>
        <div class="usage-tile">
          <span>Calls</span><strong class="mono"
            >{@render value(totals && num(totals.calls), '3em', 'figure')}</strong
          >{#if totals && totals.calls_with_usage < totals.calls}<small
              class="sub"
              title="Calls that reported usage"
              ><Glyph mark="warning" />{num(totals.calls_with_usage)}/{num(totals.calls)}</small
            >{/if}
        </div>
      </section>

      <div class="report-body">
        <div class="report-main">
          {#if pending}
            <section class="panel usage-panel" aria-labelledby="report-context-title">
              <header class="panel-heading">
                <h2 id="report-context-title" class="cap">Context per turn</h2>
              </header>
              <div class="context-chart">
                <div class="chart-outline"><Skeleton size="block" width="100%" /></div>
              </div>
            </section>
          {:else if chart.bars.length > 1}
            <section class="panel usage-panel" aria-labelledby="report-context-title">
              <header class="panel-heading">
                <h2 id="report-context-title" class="cap">Context per turn</h2>
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
                    <a href={turnHref(data.recordingId, bar.turn)} class:flagged={bar.flagged}
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

          <section class="panel usage-panel" aria-labelledby="report-turns-title">
            <header class="panel-heading">
              <h2 id="report-turns-title" class="cap">Turns · {rows.length}</h2>
            </header>
            {#if rows.length || pending}
              <div class="table-scroll">
                <table class="usage-list turn-table">
                  <thead
                    ><tr
                      >{#each turnColumns as column (column.label)}<th
                          class:numeric={column.numeric}>{column.label}</th
                        >{/each}</tr
                    ></thead
                  >
                  <tbody>
                    {#if pending}<SkeletonRows columns={turnColumns} rows={8} />{/if}
                    {#each rows as row (row.metrics.turn)}
                      {@const metrics = row.metrics}
                      {@const start = metrics.context_start_tokens ?? 0}
                      {@const end = metrics.context_end_tokens ?? start}
                      <tr>
                        <td
                          ><span>{date(metrics.started_at)}</span><small
                            >{clock(metrics.started_at)}</small
                          ></td
                        >
                        <td class="turn-prompt">
                          <a class="row-link" href={turnHref(data.recordingId, metrics.turn)}
                            >{rowPrompt(row)}</a
                          >
                          {#each row.findings as line, at (at)}<Glyph
                              mark={Marks.ofTone(line.tone)}
                              label={`${line.title}: ${line.detail}`}
                            />{/each}
                        </td>
                        <td class="mono numeric"
                          >{#if row.summary?.error_count}<span class="errors"
                              ><Glyph mark="failed" label="{row.summary.error_count} failed" />{row
                                .summary.error_count}</span
                            >{/if}{num(metrics.call_count)}</td
                        >
                        <td class="mono numeric"
                          >{metrics.idle_before_ms === null
                            ? '—'
                            : span(metrics.idle_before_ms)}</td
                        >
                        <td class="mono numeric"
                          ><span class="range"
                            ><svg viewBox="0 0 100 6" preserveAspectRatio="none" aria-hidden="true"
                              ><rect class="range-whole" width="100" height="6" /><rect
                                class="range-span"
                                x={(Math.min(start, end) / peak) * 100}
                                width={Math.max(1, (Math.abs(end - start) / peak) * 100)}
                                height="6"
                              /></svg
                            >{compactOrDash(metrics.context_start_tokens)} → {compactOrDash(
                              metrics.context_end_tokens,
                            )}</span
                          ></td
                        >
                        <td class="mono numeric"
                          >{#if metrics.cache_hit_percent !== null && metrics.cache_hit_percent < 50}<Glyph
                              mark="warning"
                              label="Most of this turn's context was not read from the cache"
                            />{/if}{percentOrDash(metrics.cache_hit_percent)}</td
                        >
                        <td class="mono numeric">{compact(metrics.cache_written_tokens)}</td>
                        <td class="mono numeric">{compact(metrics.output_tokens)}</td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {:else}
              <p class="quiet-empty">No turns: this client reports none.</p>
            {/if}
          </section>
        </div>
        <aside class="report-aside">
          <section class="panel usage-panel" aria-labelledby="report-findings-title">
            <header class="panel-heading">
              <h2 id="report-findings-title" class="cap">Findings · {findings.length}</h2>
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
              <FindingList {findings} recording={data.recordingId} page="recording" />
            {:else}
              <p class="quiet-empty">No cache rebuilds or large context steps in this session.</p>
            {/if}
          </section>
        </aside>
      </div>
      {#if report?.omitted_calls}<p class="quiet-empty">
          {num(report.omitted_calls)} older calls beyond the report's limit are not included.
        </p>{/if}
    {/if}
  </div>
</div>

<style>
  .cap {
    margin: 0;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px !important;
    font-weight: 400;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .report-body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(300px, 380px);
    align-items: start;
    gap: 0;
  }
  .report-main {
    min-width: 0;
  }
  .report-aside {
    position: sticky;
    top: 0;
    border-left: 1px solid var(--line);
  }
  @media (max-width: 1100px) {
    .report-body {
      grid-template-columns: minmax(0, 1fr);
    }
    .report-aside {
      position: static;
      border-left: 0;
    }
  }
  .errors {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    margin-right: 8px;
    color: var(--muted);
  }
  .range {
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
  }
  .range svg {
    width: 56px;
    height: 6px;
  }
  .range-whole {
    fill: var(--surface-inset);
  }
  .range-span {
    fill: var(--chart);
  }
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
    fill: var(--cache-read);
  }
  a:hover .bar {
    fill: color-mix(in srgb, var(--cache-read) 70%, var(--ink));
  }
  .flagged .bar {
    fill: var(--error);
  }
  .start {
    fill: var(--ink);
  }
  /* The chart's outline holds the chart's own height, so nothing moves when it arrives. */
  .chart-outline {
    height: 120px;
  }
</style>
