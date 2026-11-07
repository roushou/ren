<script lang="ts">
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { num, bytes, type FileUsage } from '#lib/api.ts';
  import { assertNever } from '#lib/decode.ts';
  import { buildLabel } from '#lib/display.ts';
  import Icon from '#lib/Icon.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);
  const gateway = createSnapshot(() => data.gateway);
  const snapshot = createSnapshot(() => data.recorder);
  const policy = $derived(gateway.current.value?.storage.policy);
  // `/api/metrics` is fresher; `/health` keeps counters visible if storage reads fail.
  const recorder = $derived(
    snapshot.current.value?.recorder ?? gateway.current.value?.health.recorder,
  );
  const usage = $derived(snapshot.current.value?.storage ?? gateway.current.value?.storage.usage);
  const live = (file: FileUsage) =>
    Math.max(0, file.page_count - file.freelist_count) * file.page_size;
  const free = (file: FileUsage) => file.freelist_count * file.page_size;
  const ratio = (numerator: number, denominator: number) =>
    denominator > 0 ? `${(numerator / denominator).toFixed(2)}×` : '—';
  const omitted = $derived(
    recorder
      ? recorder.omissions.policy_limit_bytes +
          recorder.omissions.queue_bytes_bytes +
          recorder.omissions.queue_slots_bytes +
          recorder.omissions.storage_failure_bytes
      : 0,
  );
  const loss = $derived(!!recorder && (omitted > 0 || recorder.omissions.unrecorded_calls > 0));
  const degraded = $derived(
    !!recorder && (recorder.storage_errors > 0 || recorder.retention_errors > 0),
  );
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
  const counterGroups = $derived<{ title: string; attention: boolean; counters: Counter[] }[]>([
    {
      title: 'Queue',
      attention: false,
      counters: [
        {
          label: 'Queued records',
          value: recorder && `${num(recorder.queue.items)} / ${num(recorder.queue.capacity)}`,
          unit: 'text',
        },
        {
          label: 'Queued bytes',
          value:
            recorder && `${bytes(recorder.queue.bytes)} / ${bytes(recorder.queue.byte_capacity)}`,
          unit: 'text',
        },
      ],
    },
    {
      title: 'Capture loss',
      attention: true,
      counters: [
        { label: 'Unrecorded calls', value: recorder?.omissions.unrecorded_calls, unit: 'count' },
        {
          label: 'Omitted · capture policy limit',
          value: recorder?.omissions.policy_limit_bytes,
          unit: 'bytes',
        },
        {
          label: 'Omitted · queue byte limit',
          value: recorder?.omissions.queue_bytes_bytes,
          unit: 'bytes',
        },
        {
          label: 'Omitted · queue slot limit',
          value: recorder?.omissions.queue_slots_bytes,
          unit: 'bytes',
        },
        {
          label: 'Omitted · storage failure',
          value: recorder?.omissions.storage_failure_bytes,
          unit: 'bytes',
        },
        {
          label: 'Not stored · WebSocket between calls',
          value: recorder?.omissions.unattributed_bytes,
          unit: 'bytes',
        },
      ],
    },
    {
      title: 'Errors',
      attention: true,
      counters: [
        { label: 'Storage errors', value: recorder?.storage_errors, unit: 'count' },
        { label: 'Retention errors', value: recorder?.retention_errors, unit: 'count' },
      ],
    },
    {
      title: 'Commits',
      attention: false,
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
      attention: false,
      counters: [
        { label: 'Pruned calls', value: recorder?.pruned_calls, unit: 'count' },
        { label: 'Pruned chunks', value: recorder?.pruned_chunks, unit: 'count' },
      ],
    },
    {
      title: 'Writes since startup',
      attention: false,
      counters: [
        { label: 'Referenced bytes', value: recorder?.dedup.referenced_bytes, unit: 'bytes' },
        { label: 'New chunk bytes', value: recorder?.dedup.new_bytes, unit: 'bytes' },
        { label: 'Compressed bytes', value: recorder?.dedup.compressed_bytes, unit: 'bytes' },
      ],
    },
  ]);
  const diskCounters = $derived<Counter[]>([
    { label: 'Metadata · live', value: usage && live(usage.disk.metadata), unit: 'bytes' },
    { label: 'Metadata · free pages', value: usage && free(usage.disk.metadata), unit: 'bytes' },
    { label: 'Blobs · live', value: usage && live(usage.disk.blobs), unit: 'bytes' },
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
    <span
      class="chip"
      class:success={!loss && !degraded && !!recorder}
      class:warning={loss || degraded}
      >{!recorder ? 'Loading' : loss || degraded ? 'Needs attention' : 'Healthy'}</span
    >
    <span class="muted">Daemon build {buildLabel(gateway.current.value?.health.build ?? null)}</span
    >
  </div>
{/snippet}

<div class="recorder-console">
  <dl class="storage-summary" aria-label="Storage usage">
    <div>
      <dt>Retained calls</dt>
      <dd>{num(usage?.calls)}</dd>
    </div>
    <div>
      <dt>Live disk usage</dt>
      <dd>
        {usage ? bytes(live(usage.disk.metadata) + live(usage.disk.blobs)) : '—'}
      </dd>
    </div>
    <div>
      <dt>Open calls</dt>
      <dd>{num(usage?.open_calls)}</dd>
    </div>
  </dl>
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
            Storage budget <span>Live pages of the metadata, blob, and search index files</span>
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
        <span
          class="badge"
          class:success={!loss && !degraded && !!recorder}
          class:warning={loss || degraded}
        >
          {!recorder ? 'Loading' : loss || degraded ? 'Attention' : 'Healthy'}
        </span>
      </header>
      {#each counterGroups as group (group.title)}
        <section class="counter-group" aria-label={group.title}>
          <h3>{group.title}</h3>
          <dl class="health-list">
            {#each group.counters as counter (counter.label)}
              <div>
                <dt>{counter.label}</dt>
                <dd
                  class:warning={group.attention &&
                    typeof counter.value === 'number' &&
                    counter.value > 0}
                  class:zero={counter.value === 0}
                >
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
  .storage-summary {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    border-bottom: 1px solid var(--control-border);
    background: var(--surface-subtle);
  }
  .storage-summary > div {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    padding: 12px;
    min-width: 0;
  }
  .storage-summary > div + div {
    border-left: 1px solid var(--line);
  }
  .storage-summary dt {
    font-size: 11px;
    color: var(--muted);
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
    .storage-summary > div {
      flex-direction: column;
      align-items: start;
      gap: 4px;
      padding: 10px;
    }
    .panel-note {
      margin-top: 0;
      border-top: 0;
    }
  }
</style>
