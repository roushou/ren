<!--
  Above Traffic's list: the listed calls as lanes, one per recording, each
  call a bar from its start to its end (`CallTimeline`), and the figures
  that say how the page went. A lane's name narrows the list to its
  recording; a bar opens its call. It reads the listed page only, and says
  so.
-->
<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import { clock, num, type CallId, type RecordingId } from '#lib/api.ts';
  import { CallTimeline } from '#lib/timeline.ts';
  import { Steps } from '#lib/steps.ts';
  import type { CallView } from '#lib/api.ts';

  let {
    calls,
    now,
    name,
    selected,
    onrecording,
    href,
    oncall,
  }: {
    calls: readonly CallView[];
    /** When the page was read: open calls run to it. */
    now: number;
    /** A recording's name, as the list names it. */
    name: (recording: RecordingId) => string;
    selected: CallId | null;
    onrecording: (recording: RecordingId) => void;
    /** Where a call opens, for its bar's link. */
    href: (call: CallId) => string;
    /** A plain click on a bar: open its call in place. */
    oncall: (event: MouseEvent, call: CallId) => void;
  } = $props();

  const timeline = $derived(CallTimeline.of(calls, now));
  const figures = $derived(timeline.figures);
  /** Five ticks across the window. */
  const ticks = $derived(
    [0, 0.25, 0.5, 0.75, 1].map((share) => timeline.from + share * (timeline.to - timeline.from)),
  );
  const time = (ms: number | null) => (ms === null ? '—' : Steps.time(ms));
</script>

<section class="overview" aria-label="The listed calls over time">
  <div class="lanes">
    <div class="lanes-head">
      <span class="cap">Calls by session</span>
      <span class="muted">this page · a bar is one call, start to end</span>
    </div>
    {#each timeline.lanes as lane (lane.recording ?? '')}
      <div class="lane">
        <span class="lane-name">
          <Glyph mark={lane.live ? 'responding' : 'idle'} label={lane.live ? 'Live' : 'Idle'} />
          {#if lane.recording}{@const recording = lane.recording}<button
              class="lane-link mono"
              title="Show only this session's calls"
              onclick={() => onrecording(recording)}>{name(recording)}</button
            >{:else}<span class="mono muted">No session</span>{/if}
          <span class="mono muted count">{lane.bars.length}</span>
        </span>
        <svg class="lane-track" viewBox="0 0 100 10" preserveAspectRatio="none">
          {#each lane.bars as bar (bar.call.id)}{@const said = `${CallTimeline.mark(bar.call).label} · ${clock(bar.call.started_at)}`}<a
              href={href(bar.call.id)}
              aria-label={said}
              onclick={(event) => oncall(event, bar.call.id)}
              ><rect
                class="bar state-{bar.mark}"
                class:chosen={bar.call.id === selected}
                x={bar.left}
                y="2"
                width={bar.width}
                height="6"><title>{said}</title></rect
              ></a
            >{/each}
        </svg>
      </div>
    {/each}
    {#if timeline.folded}<p class="muted folded">
        and {timeline.folded} quieter recording{timeline.folded === 1 ? '' : 's'}
      </p>{/if}
    <div class="ticks mono muted" aria-hidden="true">
      {#each ticks as tick, index (index)}<span>{clock(tick)}</span>{/each}
    </div>
  </div>
  <dl class="figures">
    <div>
      <dt class="cap">Calls</dt>
      <dd class="mono big">{num(figures.calls)}</dd>
      <dd class="muted">
        {figures.perMinute === null ? '—' : `${figures.perMinute.toFixed(1)} / min`}
      </dd>
    </div>
    <div>
      <dt class="cap">Failed</dt>
      <dd class="mono big">
        {#if figures.failed}<Glyph mark="failed" label={true} />{/if}{num(figures.failed)}
      </dd>
      <dd class="muted">
        {figures.calls ? `${((figures.failed / figures.calls) * 100).toFixed(1)}%` : '—'}
      </dd>
    </div>
    <div>
      <dt class="cap">Duration p50</dt>
      <dd class="mono big">{time(figures.p50)}</dd>
      <dd class="muted">p95 {time(figures.p95)}</dd>
    </div>
    <div>
      <dt class="cap">Open</dt>
      <dd class="mono big">
        {#if figures.open}<Glyph mark="responding" label={true} />{/if}{num(figures.open)}
      </dd>
      <dd class="muted">{figures.probes} quota probe{figures.probes === 1 ? '' : 's'}</dd>
    </div>
  </dl>
</section>

<style>
  .overview {
    flex: none;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 320px;
    border-bottom: 1px solid var(--line);
    background: var(--surface);
    font-size: 12px;
  }
  .lanes {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 10px 16px;
  }
  .lanes-head {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 6px;
  }
  .cap {
    color: var(--muted);
    font-size: 10.5px;
    font-weight: 500;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .lane {
    display: grid;
    grid-template-columns: 260px minmax(0, 1fr);
    align-items: center;
    column-gap: 12px;
    height: 20px;
  }
  .lane-name {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .lane-link {
    min-width: 0;
    padding: 0;
    overflow: hidden;
    border: 0;
    background: none;
    color: var(--ink);
    font: inherit;
    font-size: 11.5px;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .lane-link:hover {
    text-decoration: underline;
  }
  .count {
    margin-left: auto;
    font-size: 11px;
  }
  .lane-track {
    width: 100%;
    height: 14px;
    background: var(--surface-inset);
  }
  .bar {
    fill: var(--muted);
    cursor: pointer;
  }
  .bar.state-passed {
    fill: color-mix(in srgb, var(--muted) 70%, transparent);
  }
  .bar.state-failed {
    fill: var(--error);
  }
  .bar.state-warning,
  .bar.state-unknown {
    fill: var(--warning);
  }
  .bar.state-responding {
    fill: var(--live);
  }
  .bar.state-info {
    fill: var(--role-tool);
  }
  .bar:hover,
  .bar.chosen {
    fill: var(--accent);
  }
  .folded {
    margin: 2px 0 0;
    font-size: 11px;
  }
  .ticks {
    display: flex;
    justify-content: space-between;
    margin-left: 272px;
    font-size: 10.5px;
  }
  .figures {
    display: grid;
    grid-template-columns: 1fr 1fr;
    margin: 0;
    border-left: 1px solid var(--line);
  }
  .figures {
    align-content: start;
  }
  .figures > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--line);
  }
  .figures > div:nth-child(odd) {
    border-right: 1px solid var(--line);
  }
  .figures > div:nth-child(n + 3) {
    border-bottom: 0;
  }
  dd {
    margin: 0;
    font-size: 11px;
  }
  dd.big {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--ink);
    font-size: 18px;
  }
  @media (max-width: 960px) {
    .overview {
      grid-template-columns: minmax(0, 1fr);
    }
    .figures {
      grid-template-columns: repeat(4, 1fr);
      border-top: 1px solid var(--line);
      border-left: 0;
    }
    .figures > div {
      border-right: 1px solid var(--line);
      border-bottom: 0;
    }
  }
  @media (max-width: 760px) {
    .lanes {
      display: none;
    }
  }
</style>
