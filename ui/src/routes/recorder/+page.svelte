<script lang="ts">
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { num, bytes, type FileUsage } from '#lib/api.ts';
  import { assertNever } from '#lib/decode.ts';
  import { buildLabel, SEPARATOR } from '#lib/display.ts';
  import { RecorderReport } from '#lib/recorder.ts';
  import Icon from '#lib/Icon.svelte';
  import StateGlyph from '#lib/StateGlyph.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);
  const gateway = createSnapshot(() => data.gateway);
  const snapshot = createSnapshot(() => data.recorder);
  const policy = $derived(gateway.current.value?.storage.policy);
  // `/api/metrics` is fresher; `/health` keeps counters visible if storage reads fail.
  const recorder = $derived(
    snapshot.current.value?.metrics.recorder ?? gateway.current.value?.health.recorder,
  );
  const usage = $derived(
    snapshot.current.value?.metrics.storage ?? gateway.current.value?.storage.usage,
  );
  const report = $derived(
    recorder
      ? new RecorderReport(
          recorder,
          usage ?? null,
          policy ?? null,
          snapshot.current.value?.oldest ?? null,
          Date.now(),
        )
      : null,
  );
  const verdict = $derived(report?.verdict ?? null);
  /** The share of a queue limit in use, for its meter. */
  const share = (used: number, capacity: number) =>
    capacity > 0 ? Math.min(1, used / capacity) : 0;
  const free = (file: FileUsage) => file.freelist_count * file.page_size;
  const ratio = (numerator: number, denominator: number) =>
    denominator > 0 ? `${(numerator / denominator).toFixed(2)}×` : '—';
  type Unit = 'count' | 'bytes' | 'ms' | 'text';
  type Counter = { label: string; value: number | string | undefined; unit: Unit };
  function format(counter: Counter) {
    if (counter.value === undefined) return '—';
    switch (counter.unit) {
      case 'bytes':
        return typeof counter.value === 'number' ? bytes(counter.value) : counter.value;
      case 'ms':
        return typeof counter.value === 'number' ? `${num(counter.value)} ms` : counter.value;
      case 'count':
        return typeof counter.value === 'number' ? num(counter.value) : counter.value;
      case 'text':
        return String(counter.value);
      default:
        return assertNever(counter.unit);
    }
  }
  const counterGroups = $derived<{ title: string; counters: Counter[] }[]>([
    {
      title: 'Commits',
      counters: [
        { label: 'Commits', value: recorder?.commits, unit: 'count' },
        { label: 'Last commit latency', value: recorder?.last_commit_ms, unit: 'ms' },
        { label: 'Slowest commit', value: recorder?.max_commit_ms, unit: 'ms' },
        { label: 'Recorder lag', value: recorder?.lag_ms, unit: 'ms' },
        { label: 'Calls recovered at startup', value: recorder?.recovered_calls, unit: 'count' },
      ],
    },
    {
      title: 'Retention activity',
      counters: [
        { label: 'Pruned calls', value: recorder?.pruned_calls, unit: 'count' },
        { label: 'Pruned chunks', value: recorder?.pruned_chunks, unit: 'count' },
      ],
    },
    {
      title: 'Writes since startup',
      counters: [
        { label: 'Referenced bytes', value: recorder?.dedup.referenced_bytes, unit: 'bytes' },
        { label: 'New chunk bytes', value: recorder?.dedup.new_bytes, unit: 'bytes' },
        { label: 'Compressed bytes', value: recorder?.dedup.compressed_bytes, unit: 'bytes' },
      ],
    },
  ]);
  const diskCounters = $derived<Counter[]>([
    {
      label: 'Metadata · live',
      value: usage && RecorderReport.live(usage.disk.metadata),
      unit: 'bytes',
    },
    { label: 'Metadata · free pages', value: usage && free(usage.disk.metadata), unit: 'bytes' },
    { label: 'Blobs · live', value: usage && RecorderReport.live(usage.disk.blobs), unit: 'bytes' },
    { label: 'Blobs · free pages', value: usage && free(usage.disk.blobs), unit: 'bytes' },
    {
      label: 'Calls with derived activity',
      value: usage?.digested_calls ?? undefined,
      unit: 'count',
    },
    { label: 'Chunks', value: usage?.chunks, unit: 'count' },
    { label: 'Referenced body bytes', value: usage?.referenced_bytes, unit: 'bytes' },
    { label: 'Unique chunk bytes', value: usage?.unique_bytes, unit: 'bytes' },
    { label: 'Compressed chunk bytes', value: usage?.compressed_bytes, unit: 'bytes' },
    {
      label: 'Deduplication ratio',
      value: usage && ratio(usage.referenced_bytes, usage.unique_bytes),
      unit: 'text',
    },
    {
      label: 'Compression ratio',
      value: usage && ratio(usage.unique_bytes, usage.compressed_bytes),
      unit: 'text',
    },
  ]);
</script>

{#snippet scope()}
  <div class="scope-group">
    <span class="muted">Daemon build {buildLabel(gateway.current.value?.health.build ?? null)}</span
    >
  </div>
{/snippet}

<div class="recorder-console">
  {#if verdict}
    <section class="rec-verdict state-edge {verdict.tone}" aria-label="How recording stands">
      <strong>{verdict.headline}</strong>
      {#each verdict.facts as fact (fact)}<span class="muted">{SEPARATOR}{fact}</span>{/each}
      {#if usage?.open_calls}<span class="muted"
          >{SEPARATOR}{num(usage.open_calls)} call{usage.open_calls === 1 ? '' : 's'} open</span
        >{/if}
    </section>
  {/if}
  {#if report}
    <section class="rec-gauges" aria-label="Retention limits">
      {#each report.gauges as gauge (gauge.label)}
        <div class="rec-gauge" class:binding={gauge.binding}>
          <div class="rec-gauge-head">
            <span>{gauge.label}</span>
            {#if gauge.binding}<span
                class="rec-binding"
                title="Retention prunes for this limit first">prunes first</span
              >{/if}
            <span class="mono end"
              >{gauge.share === null ? '' : RecorderReport.percent(gauge.share)}</span
            >
          </div>
          <div class="mono rec-gauge-value">
            <strong>{gauge.used}</strong> <span class="muted">of {gauge.limit}</span>
          </div>
          <svg class="rec-meter" viewBox="0 0 100 6" preserveAspectRatio="none" aria-hidden="true"
            ><rect
              class="track"
              x="0"
              y="0"
              width="100"
              height="6"
            />{#if gauge.share !== null}<rect
                class="fill"
                class:high={gauge.share >= 0.9}
                class:mid={gauge.share >= 0.7 && gauge.share < 0.9}
                x="0"
                y="0"
                height="6"
                width={Math.min(1, gauge.share) * 100}
              />{/if}</svg
          >
        </div>
      {/each}
    </section>
  {/if}
  <div class="storage-grid">
    <section class="recorder-panel" aria-labelledby="retention-title">
      <header class="panel-heading">
        <div>
          <h2 id="retention-title">Retention policy</h2>
          <p>Configured at gateway startup</p>
        </div>
        <span class="badge" class:success={policy?.enabled}>
          {policy?.enabled ? 'Enabled' : policy ? 'Disabled' : 'Loading'}
        </span>
      </header>
      <dl class="settings-list">
        <div>
          <dt>History window <span>Measured from call start</span></dt>
          <dd>{policy ? `${num(policy.max_age_seconds / 86400)} days` : '—'}</dd>
        </div>
        <div>
          <dt>Call limit <span>Maximum retained calls</span></dt>
          <dd>{num(policy?.max_calls)}</dd>
        </div>
        <div>
          <dt>
            Storage budget <span>Live pages of the metadata and blob files (search included)</span>
          </dt>
          <dd>{policy ? bytes(policy.max_storage_bytes) : '—'}</dd>
        </div>
      </dl>
      <section class="counter-group" aria-label="Disk usage">
        <h3>Disk usage</h3>
        <dl class="health-list">
          {#each diskCounters as counter (counter.label)}
            <div>
              <dt>{counter.label}</dt>
              <dd class:zero={counter.value === 0}>{format(counter)}</dd>
            </div>
          {/each}
        </dl>
      </section>
      <footer class="panel-note">
        <Icon name="info" size={14} />
        <div>
          <p>
            Oldest finished calls are removed first. Open calls can temporarily exceed these limits.
          </p>
          <p>
            Live usage counts in-use database pages; free pages are reused. Deduplication compares
            referenced body bytes with unique chunk bytes.
          </p>
        </div>
      </footer>
    </section>
    <section class="recorder-panel" aria-labelledby="health-title">
      <header class="panel-heading">
        <div>
          <h2 id="health-title">Recorder health</h2>
          <p>Counters since gateway startup</p>
        </div>
      </header>
      {#if recorder}
        <section class="counter-group" aria-label="Queue">
          <h3>Queue</h3>
          <dl class="health-list">
            {#each [{ label: 'Records', used: recorder.queue.items, capacity: recorder.queue.capacity, text: `${num(recorder.queue.items)} / ${num(recorder.queue.capacity)}` }, { label: 'Bytes', used: recorder.queue.bytes, capacity: recorder.queue.byte_capacity, text: `${bytes(recorder.queue.bytes)} / ${bytes(recorder.queue.byte_capacity)}` }] as meter (meter.label)}
              <div class="rec-queue">
                <dt>{meter.label}</dt>
                <svg
                  class="rec-meter"
                  viewBox="0 0 100 6"
                  preserveAspectRatio="none"
                  aria-hidden="true"
                  ><rect class="track" x="0" y="0" width="100" height="6" /><rect
                    class="fill"
                    class:high={share(meter.used, meter.capacity) >= 0.9}
                    class:mid={share(meter.used, meter.capacity) >= 0.7 &&
                      share(meter.used, meter.capacity) < 0.9}
                    x="0"
                    y="0"
                    height="6"
                    width={share(meter.used, meter.capacity) * 100}
                  /></svg
                >
                <dd>{meter.text}</dd>
              </div>
            {/each}
          </dl>
        </section>
      {/if}
      {#if report}
        <section class="counter-group" aria-label="Loss and errors">
          <h3>Loss and errors</h3>
          {#each report.notes as note (note.label)}
            <div class="rec-note {note.severity}">
              <i></i>
              <div>
                <strong>{note.label}</strong>
                <small>{note.detail}</small>
              </div>
              <span class="mono">{note.value}</span>
            </div>
          {/each}
          {#if report.quiet.length}
            <details class="rec-quiet">
              <summary
                ><StateGlyph mark="yours" />{report.notes.some(
                  (note) => note.severity !== 'neutral',
                )
                  ? `${report.quiet.length} other counters at 0`
                  : `Nothing lost or failed since startup · ${report.quiet.length} counters at 0`}</summary
              >
              <ul>
                {#each report.quiet as label (label)}<li>
                    <span>{label}</span><span class="mono">0</span>
                  </li>{/each}
              </ul>
            </details>
          {/if}
        </section>
      {/if}
      {#each counterGroups as group (group.title)}
        <section class="counter-group" aria-label={group.title}>
          <h3>{group.title}</h3>
          <dl class="health-list">
            {#each group.counters as counter (counter.label)}
              <div>
                <dt>{counter.label}</dt>
                <dd class:zero={counter.value === 0}>
                  {format(counter)}
                </dd>
              </div>
            {/each}
          </dl>
        </section>
      {/each}
    </section>
  </div>
</div>

<style>
  /* Edge to edge like the other views: no margin, no outer frame. */
  .recorder-console {
    width: 100%;
    border-bottom: 1px solid var(--control-border);
    flex-shrink: 0;
  }
  dl {
    margin: 0;
  }
  dd {
    margin: 0;
    font-family: 'SFMono-Regular', Consolas, monospace;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    text-align: right;
  }
  .muted {
    color: var(--muted);
  }
  .mono {
    font-family: 'SFMono-Regular', Consolas, monospace;
    font-variant-numeric: tabular-nums;
  }
  /* The verdict: how recording stands, edged by the worst counter. */
  .rec-verdict {
    padding: 10px 16px 10px 18px;
    border-bottom: 1px solid var(--control-border);
    background: var(--surface);
    font-size: 13.5px;
  }
  .rec-verdict.error {
    --state: var(--error);
  }
  .rec-verdict.warning {
    --state: var(--warning);
  }
  .rec-verdict.success {
    --state: var(--success);
  }
  .rec-verdict strong {
    font-weight: 600;
  }
  /* Limits: how close retention is to pruning, the nearest one marked. */
  .rec-gauges {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 1px;
    border-bottom: 1px solid var(--control-border);
    background: var(--control-border);
  }
  .rec-gauge {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    padding: 10px 16px 12px;
    background: var(--surface);
  }
  .rec-gauge-head {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--muted);
    font-size: 11.5px;
  }
  .rec-gauge-head .end {
    margin-left: auto;
  }
  .rec-binding {
    padding: 0 5px;
    border: 1px solid currentColor;
    border-radius: 2px;
    color: var(--ink);
    font-size: 10.5px;
  }
  .rec-gauge-value {
    overflow: hidden;
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rec-gauge-value strong {
    font-size: 18px;
    font-weight: 500;
  }
  .rec-meter {
    display: block;
    width: 100%;
    height: 6px;
  }
  .rec-meter .track {
    fill: var(--line);
  }
  .rec-meter .fill {
    fill: var(--chart);
  }
  .rec-meter .fill.mid {
    fill: var(--warning);
  }
  .rec-meter .fill.high {
    fill: var(--error);
  }
  .rec-queue {
    grid-template-columns: 70px minmax(0, 1fr) auto !important;
    align-items: center !important;
  }
  /* Loss and errors: only what is not zero gets a row, with what it means. */
  .rec-note {
    display: grid;
    grid-template-columns: 4px minmax(0, 1fr) auto;
    align-items: center;
    gap: 0 12px;
    padding-right: 12px;
    border-bottom: 1px solid var(--line-subtle);
  }
  .rec-note > i {
    align-self: stretch;
    background: var(--line);
  }
  .rec-note.error > i {
    background: var(--error);
  }
  .rec-note.warning > i {
    background: var(--warning);
  }
  .rec-note > div {
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 7px 0;
  }
  .rec-note strong {
    font-weight: 500;
  }
  .rec-note.error strong,
  .rec-note.error .mono {
    color: var(--error);
  }
  .rec-note.warning strong,
  .rec-note.warning .mono {
    color: var(--warning);
  }
  .rec-note small {
    color: var(--muted);
    font-size: 11px;
  }
  .rec-quiet summary {
    display: flex;
    align-items: center;
    padding: 8px 12px;
    color: var(--muted);
    font-size: 12px;
    cursor: pointer;
  }
  .rec-quiet summary:hover {
    color: var(--ink);
  }
  .rec-quiet ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .rec-quiet li {
    display: flex;
    justify-content: space-between;
    padding: 6px 12px 6px 26px;
    border-top: 1px solid var(--line-subtle);
    color: var(--muted);
    font-size: 12px;
  }
  .storage-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .recorder-panel {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .recorder-panel + .recorder-panel {
    border-left: 1px solid var(--control-border);
  }
  .panel-heading {
    display: flex;
    align-items: start;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--control-border);
  }
  .panel-heading p {
    margin-top: 2px;
    color: var(--muted);
    font-size: 11px;
  }
  .settings-list > div,
  .health-list > div {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 16px;
    align-items: baseline;
    padding: 8px 12px;
    border-bottom: 1px solid var(--line-subtle);
  }
  .settings-list > div {
    padding-block: 12px;
  }
  .settings-list dt span {
    display: block;
    margin-top: 2px;
    font-size: 11px;
    color: var(--muted);
  }
  .counter-group h3 {
    margin: 0;
    padding: 5px 12px;
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--muted);
    background: var(--surface-subtle);
    border-bottom: 1px solid var(--line);
  }
  .counter-group + .counter-group {
    border-top: 1px solid var(--line);
  }
  .health-list > div:last-child {
    border-bottom: 0;
  }
  .panel-note {
    display: flex;
    gap: 8px;
    padding: 12px;
    margin-top: auto;
    border-top: 1px solid var(--line);
    background: var(--surface-subtle);
    color: var(--muted);
    font-size: 11px;
  }
  .panel-note :global(svg) {
    flex-shrink: 0;
    margin-top: 2px;
  }
  .panel-note p + p {
    margin-top: 6px;
  }
  @media (max-width: 720px) {
    .storage-grid {
      grid-template-columns: minmax(0, 1fr);
    }
    .recorder-panel + .recorder-panel {
      border-left: 0;
      border-top: 1px solid var(--control-border);
    }
    .rec-gauges {
      grid-template-columns: minmax(0, 1fr);
    }
    .panel-note {
      margin-top: 0;
      border-top: 0;
    }
  }
</style>
