<script lang="ts">
  /*
    The session's inspector, beside its conversation: context turn by turn,
    what its calls add up to, where its time went, the files it touched, and
    the work item it works on. The report is read as the session opens and
    again each time a turn ends (`version`).
  */
  import { ManagementApi, type RecordingId, type RecordingReport } from '#lib/api.ts';
  import type { SessionId } from '#lib/runs.ts';
  import { SessionInspector } from '#lib/inspector.ts';
  import { span } from '#lib/display.ts';
  import { Work, WorkApi, type WorkItem } from '#lib/work.ts';
  import type { Step } from '#lib/steps.ts';
  import Glyph from '#lib/Glyph.svelte';

  let {
    session,
    recording,
    steps,
    project,
    version,
  }: {
    session: SessionId;
    /** The recording its calls were recorded under; null before the first. */
    recording: RecordingId | null;
    steps: readonly Step[];
    /** The project it belongs to, by name; null in Scratch. */
    project: string | null;
    /** Changes when a turn ends, to read the report again. */
    version: number;
  } = $props();

  const api = new ManagementApi();
  const work = new WorkApi(fetch);

  let report = $state.raw<{ recording: RecordingId; report: RecordingReport } | null>(null);
  $effect(() => {
    void version;
    const target = recording;
    if (!target) return;
    const abort = new AbortController();
    api.report(target, abort.signal).then(
      (read) => (report = { recording: target, report: read }),
      () => {
        // Without it, the panel shows what the steps say.
      },
    );
    return () => abort.abort();
  });
  const read = $derived(report && report.recording === recording ? report.report : null);

  let item = $state.raw<{ session: SessionId; item: WorkItem | null } | null>(null);
  $effect(() => {
    void version;
    const target = session;
    const abort = new AbortController();
    work.list('current', abort.signal).then(
      (items) =>
        (item = { session: target, item: items.find((each) => each.session === target) ?? null }),
      () => {
        // Without it, no work item shows.
      },
    );
    return () => abort.abort();
  });
  const workItem = $derived(item && item.session === session ? item.item : null);

  const context = $derived(read ? SessionInspector.context(read) : null);
  const totals = $derived(read ? SessionInspector.totals(read) : null);
  const time = $derived(SessionInspector.time(steps, read));
  const files = $derived(SessionInspector.files(steps));

  /** Bars drawn in a 100-high box, one unit wide each with a gap; a short
   * session's few bars keep their width instead of filling the box. */
  const GAP = 0.25;
  const MIN_BARS = 8;
  /** The time strip's parts, as x and width in a 100-wide box. */
  const strip = $derived.by(() => {
    let x = 0;
    return time.parts.map((part) => {
      const at = x;
      x += part.share * 100;
      return { kind: part.kind, x: at, width: part.share * 100 };
    });
  });
</script>

<aside class="inspector" aria-label="Session inspector">
  <section class="block">
    <div class="head">
      <span class="cap">Context</span>
      {#if recording}<a
          class="meta calls"
          href="/monitor/sessions/{encodeURIComponent(recording)}"
          title="This session's model calls, in Monitor">Calls →</a
        >{/if}
    </div>
    {#if context && context.last !== null}
      <div class="figure">
        <span class="big">{context.last.toLocaleString()}</span><span class="meta">last call</span>
      </div>
      {#if context.cacheHit !== null}<span class="meta hit"
          >cache hit {Math.round(context.cacheHit)}%</span
        >{/if}
      <svg
        class="bars"
        viewBox="0 0 {Math.max(context.bars.length, MIN_BARS)} 100"
        preserveAspectRatio="none"
        role="img"
        aria-label="Context per turn"
      >
        {#each context.bars as bar, index (index)}
          {@const last = index === context.bars.length - 1}
          <g>
            <title>{bar.title}</title>
            <rect
              class="cached"
              x={index + GAP / 2}
              width={1 - GAP}
              y={100 - bar.cached * 96}
              height={bar.cached * 96}
            />
            <rect
              class="fresh"
              class:last
              x={index + GAP / 2}
              width={1 - GAP}
              y={100 - (bar.cached + bar.fresh) * 96}
              height={bar.fresh * 96}
            />
          </g>
        {/each}
      </svg>
      <div class="legend">
        <span class="meta"><span class="swatch cached"></span>cached</span>
        <span class="meta"><span class="swatch fresh"></span>fresh</span>
      </div>
    {:else}
      <p class="meta none">{recording ? '—' : 'No calls recorded yet'}</p>
    {/if}
  </section>

  {#if totals}
    <section class="stats">
      <div><span class="cap">Input</span><span class="stat">{totals.input}</span></div>
      <div><span class="cap">Output</span><span class="stat">{totals.output}</span></div>
      <div><span class="cap">Calls</span><span class="stat">{totals.calls}</span></div>
    </section>
  {/if}

  {#if time.total >= 1000}
    <section class="block">
      <div class="head">
        <span class="cap">Where time went</span><span class="meta">{span(time.total)}</span>
      </div>
      <svg class="time-strip" viewBox="0 0 100 1" preserveAspectRatio="none" aria-hidden="true">
        {#each strip as part (part.kind)}
          <rect
            class="time-{part.kind}"
            x={part.x}
            width={Math.max(0, part.width - 0.3)}
            height="1"
          />
        {/each}
      </svg>
      {#each time.parts as part (part.kind)}
        <div class="time-row">
          <span class="swatch time-{part.kind}"></span>
          <span>{part.label}</span>
          <span class="meta">{span(part.ms)}</span>
          <span class="meta share">{Math.round(part.share * 100)}%</span>
        </div>
      {/each}
    </section>
  {/if}

  {#if files.length}
    <section class="block files">
      <div class="head pad">
        <span class="cap">Files · {files.length}</span><span class="meta">read · edited</span>
      </div>
      <ul>
        {#each files as file (file.path)}
          <li title={file.path}>
            <span class="path">{file.path}</span>
            <span class="meta count">{file.reads || '—'}</span>
            <span class="meta count" class:edited={file.edits > 0}>{file.edits || '—'}</span>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  {#if workItem}
    <section class="block grow">
      <div class="head"><span class="cap">Work item</span></div>
      {#if project}
        <a class="title" href={Work.href(project, workItem.id)}>{workItem.title}</a>
      {:else}
        <span class="title">{workItem.title}</span>
      {/if}
      <ol class="way" aria-label="Its way to production">
        {#each Work.way(workItem) as step (step.step)}
          <li><Glyph mark={step.mark} label={true} /><span>{step.label}</span></li>
        {/each}
      </ol>
    </section>
  {:else}
    <div class="grow"></div>
  {/if}
</aside>

<style>
  .inspector {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-height: 0;
    overflow-y: auto;
    border-left: 1px solid var(--line);
    background: var(--line);
  }
  .block,
  .stats > div,
  .grow {
    background: var(--surface);
  }
  .block {
    display: flex;
    flex-direction: column;
    padding: 12px 14px;
  }
  .grow {
    flex: 1;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  .cap {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .meta {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11px;
  }
  .figure {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 6px 0 2px;
  }
  .big {
    font-family: var(--mono);
    font-size: 24px;
    font-weight: 500;
    letter-spacing: -0.02em;
  }
  .calls {
    color: var(--accent);
  }
  .calls:hover {
    color: var(--accent-hover);
  }
  .hit {
    color: var(--success);
  }
  .none {
    margin: 8px 0 0;
  }
  .bars {
    width: 100%;
    height: 72px;
    margin-top: 10px;
    border-bottom: 1px solid var(--control-border);
  }
  .bars .cached {
    fill: var(--cache-read);
  }
  .bars .fresh {
    fill: var(--muted);
  }
  .bars .fresh.last {
    fill: var(--accent);
  }
  .legend {
    display: flex;
    gap: 14px;
    margin-top: 8px;
  }
  .swatch {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 5px;
  }
  .legend .swatch.cached {
    background: var(--cache-read);
  }
  .legend .swatch.fresh {
    background: var(--muted);
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 1px;
  }
  .stats > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 14px;
  }
  .stat {
    font-family: var(--mono);
    font-size: 15px;
  }
  .time-strip {
    width: 100%;
    height: 8px;
    margin: 10px 0 8px;
  }
  .time-row {
    display: grid;
    grid-template-columns: 10px 1fr auto 44px;
    gap: 8px;
    align-items: center;
    height: 22px;
  }
  .time-row .swatch {
    margin: 0;
  }
  .share {
    text-align: right;
  }
  .time-model {
    fill: var(--time-model);
    background: var(--time-model);
  }
  .time-run {
    fill: var(--ink-soft);
    background: var(--ink-soft);
  }
  .time-read {
    fill: var(--cache-read);
    background: var(--cache-read);
  }
  .time-edit {
    fill: var(--time-edit);
    background: var(--time-edit);
  }
  .time-tool {
    fill: var(--role-thinking);
    background: var(--role-thinking);
  }
  .files {
    padding: 12px 0 6px;
  }
  .pad {
    padding: 0 14px 6px;
  }
  .files ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .files li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 22px 22px;
    gap: 6px;
    align-items: center;
    height: 24px;
    padding: 0 14px;
  }
  .path {
    overflow: hidden;
    font-family: var(--mono);
    font-size: 11.5px;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .count {
    text-align: right;
  }
  .count.edited {
    color: var(--accent);
  }
  .title {
    margin: 6px 0 8px;
    color: var(--ink);
    font-size: 12.5px;
    font-weight: 500;
  }
  a.title:hover {
    color: var(--accent);
  }
  .way {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .way li {
    display: flex;
    align-items: center;
    gap: 8px;
  }
</style>
