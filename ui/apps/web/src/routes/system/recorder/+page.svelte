<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { num, bytes, type FileUsage } from '#lib/api.ts';
  import { buildLabel } from '#lib/display.ts';
  import { RecorderReport } from '#lib/recorder.ts';
  import { Marks } from '#lib/marks.ts';
  import Glyph from '#lib/Glyph.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  const gateway = createArrival(() => data.gateway);
  const arrival = createArrival(() => data.recorder);
  const shown = $derived(arrival.current);
  /** The counters are on their way: the page draws its outline. */
  const pending = $derived(shown.state === 'pending');
  const snapshot = $derived(shown.state === 'ready' ? shown.snapshot : null);
  const policy = $derived(gateway.snapshot?.value?.storage.policy);
  // `/api/metrics` is fresher; `/health` keeps counters visible if storage reads fail.
  // While the metrics are on their way, their places are outlined instead.
  const recorder = $derived(
    pending
      ? undefined
      : (snapshot?.value?.metrics.recorder ?? gateway.snapshot?.value?.health.recorder),
  );
  const usage = $derived(
    pending
      ? undefined
      : (snapshot?.value?.metrics.storage ?? gateway.snapshot?.value?.storage.usage),
  );
  const report = $derived(
    recorder
      ? new RecorderReport(
          recorder,
          usage ?? null,
          policy ?? null,
          snapshot?.value?.oldest ?? null,
          Date.now(),
        )
      : null,
  );
  /** The limits' gauges, labelled, for their outline. */
  const gaugeOutline = $derived(RecorderReport.gauges(null, policy ?? null, null, Date.now()));
  const verdict = $derived(report?.verdict ?? null);
  /** The share of a queue limit in use, for its meter. */
  const share = (used: number, capacity: number) =>
    capacity > 0 ? Math.min(1, used / capacity) : 0;
  const free = (file: FileUsage) => file.freelist_count * file.page_size;
  const ratio = (numerator: number, denominator: number) =>
    denominator > 0 ? `${(numerator / denominator).toFixed(2)}×` : '—';
</script>

{#snippet scope()}
  <div class="scope-group">
    <span class="meta"
      >daemon build {buildLabel(gateway.snapshot?.value?.health.build ?? null)}</span
    >
  </div>
{/snippet}

<Toolbar {scope} />
<div class="recorder-console">
  {#if pending}
    <section class="rec-verdict state-edge" aria-label="How recording stands" aria-busy="true">
      <Skeleton width="22em" />
    </section>
  {:else if verdict}
    <section class="rec-verdict state-edge {verdict.tone}" aria-label="How recording stands">
      <div class="rec-state">
        <Glyph mark={Marks.ofTone(verdict.tone)} />
        <strong>{verdict.headline}</strong>
      </div>
      {#each report?.indicators ?? [] as one (one.key)}
        <div class="rec-indicator {one.tone}">
          <span class="cap">{one.label}</span>
          <span class="value mono"><Glyph mark={Marks.ofTone(one.tone)} />{one.value}</span>
          {#if one.share !== null}<svg
              class="meter"
              viewBox="0 0 100 4"
              preserveAspectRatio="none"
              aria-hidden="true"
              ><rect class="track" width="100" height="4" /><rect
                class="fill"
                width={Math.max(one.share * 100, one.share > 0 ? 2 : 0)}
                height="4"
              /></svg
            >{/if}
        </div>
      {/each}
      {#if usage}
        <div class="rec-indicator neutral">
          <span class="cap">Open</span>
          <span class="value mono">{num(usage.open_calls)}</span>
        </div>
      {/if}
    </section>
  {/if}
  {#if pending}
    <section class="rec-gauges" aria-label="Retention limits" aria-busy="true">
      {#each gaugeOutline as gauge (gauge.label)}
        <div class="rec-gauge">
          <div class="rec-gauge-head"><span>{gauge.label}</span></div>
          <div class="mono rec-gauge-value"><Skeleton size="figure" width="9em" /></div>
          <svg class="rec-meter" viewBox="0 0 100 6" preserveAspectRatio="none" aria-hidden="true"
            ><rect class="track" x="0" y="0" width="100" height="6" /></svg
          >
        </div>
      {/each}
    </section>
  {:else if report}
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
    <div class="rec-column">
      <section class="rec-block" aria-labelledby="body-title" aria-busy={pending}>
        <header class="rec-head">
          <h2 id="body-title" class="cap">What a body becomes on disk</h2>
          {#if usage}<span class="meta">{num(usage.chunks)} chunks</span>{/if}
        </header>
        {#if usage}
          {@const steps = [
            {
              label: 'Referenced body bytes',
              bytes: usage.referenced_bytes,
              note: '',
              tone: 'referenced',
            },
            {
              label: 'Unique chunk bytes',
              bytes: usage.unique_bytes,
              note: `deduplication ${ratio(usage.referenced_bytes, usage.unique_bytes)}`,
              tone: 'unique',
            },
            {
              label: 'Compressed chunk bytes',
              bytes: usage.compressed_bytes,
              note: `compression ${ratio(usage.unique_bytes, usage.compressed_bytes)}`,
              tone: 'compressed',
            },
          ]}
          <div class="funnel">
            {#each steps as step (step.label)}
              <span class="funnel-label">{step.label}</span>
              <svg viewBox="0 0 100 10" preserveAspectRatio="none" aria-hidden="true"
                ><rect class="funnel-track" width="100" height="10" /><rect
                  class="funnel-bar {step.tone}"
                  width={(step.bytes / Math.max(1, usage.referenced_bytes)) * 100}
                  height="10"
                /></svg
              >
              <span class="mono value">{bytes(step.bytes)}</span>
              {#if step.note}<span></span><span class="meta">{step.note}</span><span></span>{/if}
            {/each}
            <span></span><span class="meta ok"
              >{ratio(usage.referenced_bytes, usage.compressed_bytes)} smaller than what clients sent</span
            ><span></span>
          </div>
        {:else}<Skeleton width="100%" />{/if}
      </section>

      <section class="rec-block" aria-labelledby="disk-title" aria-busy={pending}>
        {#if usage}
          {@const parts = [
            {
              key: 'meta-live',
              label: 'Metadata · live',
              bytes: RecorderReport.live(usage.disk.metadata),
            },
            { key: 'meta-free', label: 'Metadata · free pages', bytes: free(usage.disk.metadata) },
            {
              key: 'blob-live',
              label: 'Blobs · live',
              bytes: RecorderReport.live(usage.disk.blobs),
            },
            { key: 'blob-free', label: 'Blobs · free pages', bytes: free(usage.disk.blobs) },
          ]}
          {@const whole = Math.max(
            1,
            parts.reduce((sum, part) => sum + part.bytes, 0),
          )}
          <header class="rec-head">
            <h2 id="disk-title" class="cap">Disk usage · {bytes(whole)}</h2>
          </header>
          <svg class="disk-bar" viewBox="0 0 100 10" preserveAspectRatio="none" aria-hidden="true"
            >{#each parts as part, index (part.key)}<rect
                class="disk-{part.key}"
                x={(parts.slice(0, index).reduce((sum, one) => sum + one.bytes, 0) / whole) * 100}
                width={(part.bytes / whole) * 100}
                height="10"><title>{part.label}: {bytes(part.bytes)}</title></rect
              >{/each}</svg
          >
          <div class="disk-key">
            {#each parts as part (part.key)}
              <div>
                <span class="meta"><span class="swatch disk-{part.key}"></span>{part.label}</span>
                <span class="mono">{bytes(part.bytes)}</span>
              </div>
            {/each}
          </div>
        {:else}
          <header class="rec-head"><h2 id="disk-title" class="cap">Disk usage</h2></header>
          <Skeleton width="100%" />
        {/if}
      </section>

      <section class="rec-block grow" aria-labelledby="retention-title">
        <header class="rec-head">
          <h2 id="retention-title" class="cap">Retention policy</h2>
          {#if policy}<Glyph
              mark={policy.enabled ? 'passed' : 'idle'}
              label={policy.enabled ? 'Enabled' : 'Disabled'}
            />{/if}
        </header>
        <div class="policy">
          <div>
            <span class="meta">History window</span>
            <span class="mono">{policy ? `${num(policy.max_age_seconds / 86400)} days` : '—'}</span>
            <span class="meta">from call start</span>
          </div>
          <div>
            <span class="meta">Call limit</span>
            <span class="mono">{num(policy?.max_calls)}</span>
            <span class="meta">retained calls</span>
          </div>
          <div>
            <span class="meta">Storage limit</span>
            <span class="mono">{policy ? bytes(policy.max_storage_bytes) : '—'}</span>
            <span class="meta">on disk</span>
          </div>
        </div>
        <div class="pruned">
          <div>
            <span class="meta">Pruned calls</span><span class="mono"
              >{num(recorder?.pruned_calls)}</span
            >
          </div>
          <div>
            <span class="meta">Pruned chunks</span><span class="mono"
              >{num(recorder?.pruned_chunks)}</span
            >
          </div>
          <div>
            <span class="meta">Calls with derived activity</span><span class="mono"
              >{num(usage?.digested_calls ?? undefined)}{#if usage}{' '}<span class="meta"
                  >of {num(usage.calls)}</span
                >{/if}</span
            >
          </div>
        </div>
      </section>
    </div>

    <div class="rec-column side">
      <section class="rec-block" aria-labelledby="queue-title" aria-busy={pending}>
        <header class="rec-head">
          <h2 id="queue-title" class="cap">Recorder health · queue</h2>
        </header>
        {#if recorder}
          {#each [{ label: 'Records', used: recorder.queue.items, capacity: recorder.queue.capacity, text: `${num(recorder.queue.items)} / ${num(recorder.queue.capacity)}` }, { label: 'Bytes', used: recorder.queue.bytes, capacity: recorder.queue.byte_capacity, text: `${bytes(recorder.queue.bytes)} / ${bytes(recorder.queue.byte_capacity)}` }] as meter (meter.label)}
            <div class="queue">
              <span>{meter.label}</span><span class="mono">{meter.text}</span>
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
                  width={Math.max(0.5, share(meter.used, meter.capacity) * 100)}
                /></svg
              >
            </div>
          {/each}
        {:else}<Skeleton width="100%" />{/if}
      </section>

      <section class="rec-block" aria-labelledby="commits-title" aria-busy={pending}>
        <header class="rec-head">
          <h2 id="commits-title" class="cap">
            Commits{#if recorder}{' · '}{num(recorder.commits)}{/if} · commit time
          </h2>
        </header>
        {#if recorder}
          {@const scale = Math.max(250, recorder.max_commit_ms)}
          <svg
            class="commit-scale"
            viewBox="0 0 100 10"
            preserveAspectRatio="none"
            aria-hidden="true"
            ><rect class="track" y="4" width="100" height="2" /><rect
              class="tick last"
              x={(recorder.last_commit_ms / scale) * 100 - 0.4}
              width="0.8"
              height="10"
            /><rect
              class="tick slowest"
              x={Math.min(99.2, (recorder.max_commit_ms / scale) * 100 - 0.4)}
              width="0.8"
              height="10"
            /></svg
          >
          <div class="scale-axis meta">
            <span>0</span><span>{Math.round(scale / 2)} ms</span><span>{scale} ms</span>
          </div>
          <dl class="rows">
            <div>
              <dt>Last commit</dt>
              <dd class="mono">{num(recorder.last_commit_ms)} ms</dd>
            </div>
            <div>
              <dt>Slowest commit</dt>
              <dd class="mono">{num(recorder.max_commit_ms)} ms</dd>
            </div>
            <div>
              <dt>Recorder lag</dt>
              <dd class="mono">{num(recorder.lag_ms)} ms</dd>
            </div>
            <div>
              <dt>Calls recovered at startup</dt>
              <dd class="mono">
                {num(recorder.recovered_calls)}
              </dd>
            </div>
          </dl>
        {:else}<Skeleton width="100%" />{/if}
      </section>

      <section class="rec-block grow" aria-labelledby="loss-title" aria-busy={pending}>
        <header class="rec-head">
          <h2 id="loss-title" class="cap">Loss and errors</h2>
          {#if report && !report.notes.some((note) => note.severity !== 'neutral')}<Glyph
              mark="passed"
              label="Nothing lost or failed"
            />{/if}
        </header>
        {#if report}
          {#each report.notes as note (note.label)}
            <div class="rec-note {note.severity}">
              <div>
                <strong>{note.label}</strong>
                <small>{note.detail}</small>
              </div>
              <span class="mono">{note.value}</span>
            </div>
          {/each}
          <dl class="rows">
            {#each report.quiet as label (label)}<div>
                <dt>{label}</dt>
                <dd class="mono">0</dd>
              </div>{/each}
          </dl>
        {:else}<Skeleton width="100%" />{/if}
      </section>
    </div>
  </div>
</div>

<style>
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
    font-family: var(--mono);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    text-align: right;
  }
  .muted {
    color: var(--muted);
  }
  .mono {
    font-family: var(--mono);
    font-variant-numeric: tabular-nums;
  }
  .disk-bar {
    display: block;
    width: calc(100% - 24px);
    height: 8px;
    margin: 4px 12px 6px;
    background: var(--surface-inset);
  }
  .disk-meta-live {
    fill: var(--chart);
    background: var(--chart);
  }
  .disk-meta-free {
    fill: color-mix(in srgb, var(--chart) 35%, transparent);
    background: color-mix(in srgb, var(--chart) 35%, transparent);
  }
  .disk-blob-live {
    fill: var(--accent);
    background: var(--accent);
  }
  .disk-blob-free {
    fill: color-mix(in srgb, var(--accent) 35%, transparent);
    background: color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .swatch {
    width: 8px;
    height: 8px;
  }
  .rec-verdict {
    display: grid;
    grid-template-columns: minmax(220px, 1.6fr) repeat(auto-fit, minmax(92px, 1fr));
    gap: 1px;
    border-bottom: 1px solid var(--control-border);
    background: var(--line);
    font-size: 13.5px;
  }
  .rec-verdict > div {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 5px;
    min-width: 0;
    padding: 10px 16px;
    background: var(--surface);
  }
  .rec-verdict > .rec-state {
    flex-direction: row;
    align-items: center;
    justify-content: flex-start;
    gap: 10px;
    padding-left: 18px;
  }
  .rec-indicator .value {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 15px;
  }
  .rec-indicator.success {
    --tone: var(--success);
  }
  .rec-indicator.warning {
    --tone: var(--warning);
    background: color-mix(in srgb, var(--warning) 8%, var(--surface));
  }
  .rec-indicator.error {
    --tone: var(--error);
    background: color-mix(in srgb, var(--error) 10%, var(--surface));
  }
  .rec-indicator.warning .value,
  .rec-indicator.error .value {
    color: var(--tone);
  }
  .rec-indicator .meter {
    width: 100%;
    height: 4px;
  }
  .rec-indicator .meter .track {
    fill: var(--surface-inset);
  }
  .rec-indicator .meter .fill {
    fill: var(--tone);
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
  .rec-note strong {
    font-weight: 500;
  }
  .rec-note.error strong,
  .rec-note small {
    color: var(--muted);
    font-size: 11px;
  }
  .cap {
    margin: 0;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    font-weight: 400;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .meta {
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11px;
  }
  .meta.ok {
    color: var(--success);
  }
  .storage-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 420px;
    gap: 1px;
    background: var(--line);
  }
  .rec-column {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .rec-block {
    padding: 12px 16px 14px;
    background: var(--surface);
  }
  .rec-block.grow {
    flex: 1;
  }
  .rec-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 10px;
  }
  .funnel {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr) 90px;
    column-gap: 14px;
    row-gap: 4px;
    align-items: center;
  }
  .funnel svg {
    width: 100%;
    height: 22px;
  }
  .funnel-label {
    font-size: 12.5px;
  }
  .funnel .value {
    text-align: right;
  }
  .funnel-track {
    fill: var(--surface-inset);
  }
  .funnel-bar.referenced {
    fill: var(--input-fresh);
  }
  .funnel-bar.unique {
    fill: var(--cache-read);
  }
  .funnel-bar.compressed {
    fill: var(--accent);
  }
  .disk-key {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 12px;
    margin-top: 8px;
  }
  .disk-key > div {
    display: flex;
    flex-direction: column;
  }
  .disk-key .mono {
    font-size: 14px;
  }
  .policy {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    border: 1px solid var(--line);
  }
  .policy > div {
    display: flex;
    flex-direction: column;
    padding: 8px 12px;
  }
  .policy > div + div {
    border-left: 1px solid var(--line);
  }
  .policy .mono,
  .pruned .mono {
    font-size: 14px;
  }
  .pruned {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px;
    margin-top: 14px;
  }
  .pruned > div {
    display: flex;
    flex-direction: column;
  }
  .queue {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    row-gap: 4px;
    margin-bottom: 10px;
    font-size: 12.5px;
  }
  .queue .rec-meter {
    grid-column: 1 / -1;
  }
  .commit-scale {
    width: 100%;
    height: 14px;
  }
  .commit-scale .track {
    fill: var(--line);
  }
  .commit-scale .tick.last {
    fill: var(--ink);
  }
  .commit-scale .tick.slowest {
    fill: var(--warning);
  }
  .scale-axis {
    display: flex;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .rows > div {
    display: flex;
    justify-content: space-between;
    padding: 3px 0;
    font-size: 12.5px;
  }
  .rows dt {
    color: var(--ink);
  }
  .rec-note {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 8px;
    padding: 8px 10px;
    border: 1px solid var(--line);
  }
  .rec-note > div {
    display: flex;
    flex-direction: column;
  }
  @media (max-width: 1100px) {
    .storage-grid {
      grid-template-columns: minmax(0, 1fr);
    }
    .funnel {
      grid-template-columns: 140px minmax(0, 1fr) 80px;
    }
  }
</style>
