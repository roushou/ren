<!--
  One call in the listing: when it started, how it ended, its model and the
  tools it asked for, its context and output, how long it took, and its HTTP
  status. A click anywhere on the row selects it.
-->
<script lang="ts">
  import ChipTag from '../ChipTag.svelte';
  import Glyph from '../Glyph.svelte';
  import Meter from '../Meter.svelte';
  import RecordingCell from './RecordingCell.svelte';
  import { short, title, providerName, providerMark, type CallView } from '../api.ts';
  import type { RecordingId, RecordingView } from '../api.ts';
  import {
    isActive,
    captureChip,
    compactOrDash,
    date,
    duration,
    httpChip,
    streamChip,
  } from '../display.ts';
  import type { RecordingDetail } from '../recordings.ts';
  import { Clock, Elapsed } from '../time.ts';
  import { CallTimeline } from '../timeline.ts';
  import type { Filters } from '../traffic.ts';
  import type { CallSelection } from './selection.svelte.ts';

  let {
    call: c,
    previous,
    selection,
    today,
    bodyLimit,
    longest,
    sessions,
  }: {
    call: CallView;
    /** The row above, in the same group. */
    previous: CallView | undefined;
    selection: CallSelection;
    /** Today's date as `date` writes it: a call from another day shows its date. */
    today: string;
    /** The daemon's per-body capture limit, to tell a trimmed body from lost bytes. */
    bodyLimit: number | null;
    /** The page's longest finished call: the duration bar is this call's share. */
    longest: number;
    /** The sessions rows name, on the traffic page; null on a session's own page. */
    sessions: {
      recordings: Map<RecordingId, RecordingView>;
      details: Map<RecordingId, RecordingDetail>;
      onfilter: (change: (filters: Filters) => void) => void;
    } | null;
  } = $props();
  const said = $derived(CallTimeline.mark(c));
  const http = $derived(httpChip(c.status));
  const stream = $derived(streamChip(c));
  const capture = $derived(captureChip(c, bodyLimit));
  const context = $derived(CallTimeline.context(c));
  const took = $derived(duration(c));
  const selected = $derived(selection.id === c.id);
</script>

<tr class="call-row" class:selected onclick={(event) => selection.selectRow(event, c.id)}>
  <td class="col-time mono" title={new Date(c.started_at).toLocaleString()}
    >{Clock.second(c.started_at)}{#if date(c.started_at) !== today}<small
        >{date(c.started_at)}</small
      >{/if}</td
  >
  <td class="col-mark"><Glyph mark={said.mark} label={said.label} /></td>
  <td class="col-call"
    ><a
      class="row-link mono"
      id={`call-${c.id}`}
      href={selection.href(c.id)}
      onclick={(event) => selection.select(event, c.id)}
      aria-label={`Inspect call ${c.id}`}
      aria-current={selected ? 'true' : undefined}>{short(c.id)}</a
    >{#if stream.tone !== 'success' && stream.tone !== 'live'}<ChipTag
        chip={stream}
      />{/if}{#if capture.tone === 'warning'}<span class="chip warning" title={capture.title}
        >{capture.label}</span
      >{/if}{#if sessions}<RecordingCell
        call={c}
        {previous}
        recordings={sessions.recordings}
        details={sessions.details}
        onfilter={sessions.onfilter}
      />{/if}</td
  >
  <td class="col-model"
    ><span class="model-line"
      ><span class="provider-mark tiny {c.provider}" title={providerName(c.provider)}
        >{providerMark(c.provider)}</span
      ><span class="model-name mono" title={c.model ?? title(c.endpoint)}
        >{c.model ?? (isActive(c) ? 'Awaiting observation' : title(c.endpoint))}</span
      ></span
    ></td
  >
  <td class="col-tools"
    >{#if c.tool_names.length}<span class="tool-chips"
        >{#each c.tool_names.slice(0, 2) as name, at (at)}<span class="chip tool-chip mono"
            >{name}</span
          >{/each}{#if c.tool_names.length > 2}<span class="chip mono"
            >+{c.tool_names.length - 2}</span
          >{/if}</span
      >{/if}</td
  >
  <td class="col-context numeric mono"
    >{#if context}<span
        class="figure"
        title="{context.tokens.toLocaleString()} tokens of prompt, {Math.round(
          context.cached * 100,
        )}% read from the cache"
        ><span class="cell-meter"><Meter share={context.cached} tone="success" /></span
        >{compactOrDash(context.tokens)}</span
      >{:else}<span class="muted">—</span>{/if}</td
  >
  <td class="col-out numeric mono">{compactOrDash(c.usage?.output_tokens ?? null)}</td>
  <td class="col-duration numeric mono"
    ><span class="figure"
      >{#if took !== null && longest > 0}<span class="cell-meter"
          ><Meter
            share={took / longest}
            least={0.02}
            tone={said.mark === 'failed'
              ? 'error'
              : said.mark === 'warning' || said.mark === 'unknown'
                ? 'warning'
                : 'muted'}
          /></span
        >{/if}{took === null ? '' : Elapsed.stopwatch(took)}</span
    ></td
  >
  <td class="col-http numeric mono"
    ><span class="status-code {http.tone}" title={http.title}>{http.label}</span></td
  >
</tr>

<style>
  /* The listing's column widths; its header carries the same. */
  .col-time {
    width: 88px;
    white-space: nowrap;
  }
  .col-mark {
    width: 26px;
    padding-right: 0;
  }
  .col-call {
    width: 32%;
  }
  .col-model {
    width: 16%;
  }
  .col-tools {
    width: 16%;
  }
  .col-context {
    width: 128px;
  }
  .col-out {
    width: 64px;
  }
  .col-duration {
    width: 112px;
  }
  .col-http {
    width: 56px;
  }
  .numeric {
    text-align: right;
  }
  .col-time small {
    display: block;
    color: var(--muted);
  }
  .col-call {
    overflow: hidden;
    white-space: nowrap;
  }
  /* One line a call: its ID, quiet, then the session it belongs to. */
  .col-call > .row-link {
    margin-right: 8px;
    color: var(--muted);
    font-size: 11px;
  }
  .col-call .chip {
    margin-left: 6px;
  }
  .model-line {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .model-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tool-chips {
    display: flex;
    gap: 4px;
    overflow: hidden;
  }
  .figure {
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
    width: 100%;
  }
  @media (max-width: 960px) {
    .col-tools,
    .col-out,
    .col-http {
      display: none;
    }
  }
  @media (max-width: 760px) {
    .col-context,
    .col-model {
      display: none;
    }
  }
  .cell-meter {
    flex: 1;
    max-width: 52px;
    --meter-track: var(--line);
  }
</style>
