<!--
  A recording's report beside the Recordings list: what it is, where to go
  next (its calls, its full report), its three headline figures, its
  context turn by turn against its peak, and its findings. The report is
  read when the preview opens.
-->
<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import FindingList from '#lib/FindingList.svelte';
  import { ManagementApi, num, short, type RecordingReport } from '#lib/api.ts';
  import { findingLine } from '#lib/activity.ts';
  import { compactOrDash } from '#lib/display.ts';
  import { Board } from '#lib/board.ts';
  import { RecordingList, type RecordingRow } from '#lib/recordings.ts';
  import { snapshotError } from '#lib/snapshots.ts';

  let {
    row,
    desk = null,
  }: {
    row: RecordingRow;
    /** Where the session Ren drives it as opens on its desk; null when Ren
     * does not drive it. */
    desk?: string | null;
  } = $props();
  const recording = $derived(row.recording);
  const href = $derived(`/monitor/sessions/${encodeURIComponent(recording.id)}`);

  type Read =
    | { state: 'reading' }
    | { state: 'read'; report: RecordingReport }
    | { state: 'failed'; reason: string };
  let read = $state<Read>({ state: 'reading' });
  $effect(() => {
    const id = recording.id;
    read = { state: 'reading' };
    const controller = new AbortController();
    new ManagementApi()
      .report(id, controller.signal)
      .then((report) => (read = { state: 'read', report }))
      .catch((error: unknown) => {
        if (!controller.signal.aborted)
          read = {
            state: 'failed',
            reason: snapshotError(error, 'The report could not be read.').message,
          };
      });
    return () => controller.abort();
  });
  const turns = $derived(read.state === 'read' ? RecordingList.turnContext(read.report) : []);
</script>

<div class="preview">
  <header>
    <h2>
      {#if row.now.kind === 'open'}<Glyph mark={row.now.mark} label={row.now.label} />{/if}
      <span>{row.detail?.prompt?.text.split('\n')[0] ?? `${row.client} recording`}</span>
    </h2>
    <p class="muted">
      {[
        row.client,
        row.detail?.workdir ? Board.folder(row.detail.workdir) : null,
        `session ${short(recording.native_id)}`,
      ]
        .filter((part) => part !== null)
        .join(' · ')}
    </p>
    <div class="actions">
      <a class="button primary small" {href}>Calls</a>
      <a class="button small" href="{href}/report">Report</a>
      {#if desk}<a class="button small" href={desk}>Open in desk</a>{/if}
    </div>
  </header>

  {#if read.state === 'reading'}
    <p class="muted pad">Reading its report…</p>
  {:else if read.state === 'failed'}
    <p class="error pad" role="alert">{read.reason}</p>
  {:else}
    {@const totals = read.report.totals}
    <dl class="figures">
      <div>
        <dt>Peak context</dt>
        <dd class="mono">{compactOrDash(totals.peak_context_tokens)}</dd>
      </div>
      <div>
        <dt>Cache hit</dt>
        <dd class="mono">
          {totals.cache_hit_percent === null ? '—' : `${Math.round(totals.cache_hit_percent)}%`}
        </dd>
      </div>
      <div>
        <dt>Output</dt>
        <dd class="mono">{compactOrDash(totals.output_tokens)}</dd>
      </div>
    </dl>

    <section>
      <h3>Context per turn</h3>
      {#if turns.length}
        <svg
          class="turns"
          viewBox="0 0 {turns.length * 10} 100"
          preserveAspectRatio="none"
          aria-hidden="true"
        >
          {#each turns as turn, index (index)}<rect
              class="turn-start"
              x={index * 10 + 1}
              y={100 - turn.start * 100}
              width="8"
              height={turn.start * 100}
            /><rect
              class="turn-grew"
              class:rebuilt={turn.rebuilt}
              x={index * 10 + 1}
              y={100 - Math.max(turn.start, turn.end) * 100}
              width="8"
              height={Math.abs(turn.end - turn.start) * 100}><title>{turn.title}</title></rect
            >{/each}
        </svg>
        <p class="legend muted">
          <span><span class="swatch turn-start"></span>start</span>
          <span><span class="swatch turn-grew"></span>grew</span>
          <span><span class="swatch rebuilt"></span>cache rebuilt</span>
          <span class="mono">{num(read.report.turns.length)} turns</span>
        </p>
      {:else}<p class="muted">No turn reported usage.</p>{/if}
    </section>

    <section>
      <h3>
        Findings
        {#if read.report.findings.length}<span class="muted mono"
            >{read.report.findings.length}</span
          >{/if}
      </h3>
      {#if read.report.findings.length}
        <FindingList
          findings={read.report.findings.map(findingLine)}
          recording={recording.id}
          page="recording"
        />
      {:else}<p class="muted"><Glyph mark="passed" label={true} /> Nothing stood out.</p>{/if}
    </section>
  {/if}
</div>

<style>
  .preview {
    display: flex;
    flex-direction: column;
    font-size: 12.5px;
  }
  header {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 80px 12px 16px;
    border-bottom: 1px solid var(--line);
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  h2 span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  header p {
    margin: 0;
    font-size: 11.5px;
  }
  .actions {
    display: flex;
    gap: 6px;
    margin-top: 4px;
  }
  .pad {
    padding: 12px 16px;
  }
  .figures {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    margin: 0;
    border-bottom: 1px solid var(--line);
  }
  .figures > div {
    padding: 10px 16px;
    border-right: 1px solid var(--line);
  }
  .figures > div:last-child {
    border-right: 0;
  }
  dt {
    color: var(--muted);
    font-size: 10.5px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  dd {
    margin: 4px 0 0;
    font-size: 17px;
  }
  section {
    padding: 12px 16px;
    border-bottom: 1px solid var(--line);
  }
  h3 {
    display: flex;
    gap: 8px;
    align-items: baseline;
    margin: 0 0 10px;
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  h3 .muted {
    font-size: 11px;
    letter-spacing: 0;
    text-transform: none;
  }
  .turns {
    display: block;
    width: 100%;
    height: 90px;
    background: var(--surface-inset);
  }
  .turn-start {
    fill: color-mix(in srgb, var(--muted) 45%, transparent);
    background: color-mix(in srgb, var(--muted) 45%, transparent);
  }
  .turn-grew {
    fill: var(--chart);
    background: var(--chart);
  }
  .turn-grew.rebuilt,
  .rebuilt {
    fill: var(--error);
    background: var(--error);
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    margin: 8px 0 0;
    font-size: 11px;
  }
  .legend > span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .swatch {
    width: 8px;
    height: 8px;
  }
  p {
    margin: 0;
  }
</style>
