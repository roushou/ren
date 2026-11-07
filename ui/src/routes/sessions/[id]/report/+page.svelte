<script lang="ts">
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { num, short, clock, providerMark, type CallId, type TurnId } from '#lib/api.ts';
  import { compact, date, span, sessionTone } from '#lib/display.ts';
  import { findingLine, turnRows, type FindingLine } from '#lib/activity.ts';
  import Icon from '#lib/Icon.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);
  const snapshot = createSnapshot(() => data.report);
  const current = $derived(snapshot.current.value);
  const report = $derived(current?.report);
  const session = $derived(current?.session);
  const rows = $derived(current ? turnRows(current.report, current.turns) : []);
  const findings = $derived(report ? report.findings.map(findingLine) : []);
  const tokens = (value: number | null) => (value === null ? '—' : compact(value));
  const callHref = (call: CallId) =>
    `/sessions/${encodeURIComponent(data.sessionId)}?call=${encodeURIComponent(call)}`;
  const turnHref = (turn: TurnId) => `/turns/${encodeURIComponent(turn)}`;
  const findingHref = (line: FindingLine) =>
    line.turn ? turnHref(line.turn) : line.call ? callHref(line.call) : null;
</script>

{#snippet scope()}
  <div class="scope-group scope-identity">
    <span class="session-dot tone-{sessionTone(data.sessionId)}"></span>{#if session}<span
        class="provider-mark tiny {session.provider}">{providerMark(session.provider)}</span
      ><strong class="mono" title="The session ID your client reported"
        >{short(session.native_id)}</strong
      >{:else}<strong class="mono">{short(data.sessionId)}</strong>{/if}
    <span class="muted">Context and cache report</span>
  </div>
  <span
    class="scope-group usage-note"
    title="Tokens are what providers report. They are not subscription allowance or charges."
    ><Icon name="info" size={12} /> Provider-reported tokens</span
  >
  <div class="scope-group end">
    <a class="text-button" href={`/sessions/${encodeURIComponent(data.sessionId)}`}
      >Session calls <Icon name="arrow" size={12} /></a
    ><a class="text-button" href="/sessions">All sessions <Icon name="arrow" size={12} /></a>
  </div>
{/snippet}

<div class="usage-page">
  <div class="console">
    {#if !report}
      <div class="empty-state">
        <div class="empty-icon"><Icon name="alert" size={26} /></div>
        <h2>The report could not be read</h2>
        <p>{snapshot.current.error?.message ?? 'Loading…'}</p>
      </div>
    {:else}
      {@const totals = report.totals}
      <section class="usage-tiles" aria-label="Totals">
        <div class="usage-tile">
          <span>Peak context</span><strong class="mono"
            >{compact(totals.peak_context_tokens)}</strong
          ><small>tokens in one call</small>
        </div>
        <div class="usage-tile">
          <span>Read from cache</span><strong class="mono"
            >{totals.cache_hit_percent === null ? '—' : `${totals.cache_hit_percent}%`}</strong
          ><small>{compact(totals.cache_read_tokens)} of {compact(totals.context_tokens)}</small>
        </div>
        <div class="usage-tile">
          <span>Written to cache</span><strong class="mono"
            >{compact(totals.cache_written_tokens)}</strong
          ><small>tokens</small>
        </div>
        <div class="usage-tile">
          <span>Output</span><strong class="mono">{compact(totals.output_tokens)}</strong><small
            >tokens</small
          >
        </div>
        <div class="usage-tile">
          <span>Calls</span><strong class="mono">{num(totals.calls)}</strong><small
            >{num(totals.calls_with_usage)} reported usage</small
          >
        </div>
      </section>

      <section class="panel usage-panel" aria-labelledby="report-findings-title">
        <header class="panel-heading">
          <h2 id="report-findings-title">Findings</h2>
          <span class="muted">Measured against fixed thresholds; causes are not inferred</span>
        </header>
        {#if findings.length}
          <ul class="finding-list">
            {#each findings as line, index (index)}
              {@const href = findingHref(line)}
              <li>
                <span class="chip {line.tone}">{line.title}</span>
                <span class="finding-detail">{line.detail}</span>
                {#if href}<a class="text-button" {href}
                    >{line.turn ? 'Open turn' : 'Open call'} <Icon name="arrow" size={12} /></a
                  >{/if}
              </li>
            {/each}
          </ul>
        {:else}
          <p class="quiet-empty">No cache rebuilds or large context steps in this session.</p>
        {/if}
      </section>

      <section class="panel usage-panel" aria-labelledby="report-turns-title">
        <header class="panel-heading">
          <h2 id="report-turns-title">Turns</h2>
          <span class="muted">Newest first · one user prompt each, as the client grouped them</span>
        </header>
        {#if rows.length}
          <div class="table-scroll">
            <table class="usage-list turn-table">
              <thead
                ><tr
                  ><th>Started</th><th>Prompt</th><th class="numeric">Calls</th><th class="numeric"
                    >Idle before</th
                  ><th class="numeric">Context</th><th class="numeric">From cache</th><th
                    class="numeric">Written</th
                  ><th class="numeric">Out</th></tr
                ></thead
              >
              <tbody>
                {#each rows as row (row.metrics.turn)}
                  {@const metrics = row.metrics}
                  <tr>
                    <td
                      ><span>{date(metrics.started_at)}</span><small
                        >{clock(metrics.started_at)}</small
                      ></td
                    >
                    <td class="turn-prompt">
                      <a class="row-link" href={turnHref(metrics.turn)}
                        >{row.summary?.prompt?.text ?? 'No prompt read'}</a
                      >
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
                      >{tokens(metrics.context_start_tokens)} → {tokens(
                        metrics.context_end_tokens,
                      )}</td
                    >
                    <td class="mono numeric"
                      >{metrics.cache_hit_percent === null
                        ? '—'
                        : `${metrics.cache_hit_percent}%`}</td
                    >
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
      {#if report.omitted_calls}<p class="quiet-empty">
          {num(report.omitted_calls)} older calls beyond the report's limit are not included.
        </p>{/if}
    {/if}
  </div>
</div>
