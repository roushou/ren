import test from 'node:test';
import assert from 'node:assert/strict';
import { RecorderReport } from '../src/lib/recorder.ts';
import type { RetentionPolicy, StorageView } from '../src/lib/api.ts';
import { recorderFixture, storageViewFixture } from './fixtures.ts';

const omissions = recorderFixture().omissions;
const DAY = 86_400_000;
const NOW = 100 * DAY;
const policy: RetentionPolicy = {
  enabled: true,
  max_age_seconds: 7 * 86_400,
  max_calls: 10_000,
  max_storage_bytes: 1_000_000,
};
const storage = (calls: number, pages: number): StorageView => ({
  ...storageViewFixture(),
  calls,
  disk: {
    metadata: { page_size: 1000, page_count: pages, freelist_count: 0 },
    blobs: { page_size: 1000, page_count: 0, freelist_count: 0 },
  },
});

test('bytes past the capture policy are by design, never loss', () => {
  const policyOnly = recorderFixture({
    omissions: { ...omissions, policy_limit_bytes: 250 << 20, unattributed_bytes: 10 },
  });
  assert.deepEqual(RecorderReport.capture(policyOnly), { kind: 'healthy' });
  const report = new RecorderReport(policyOnly, null, null, null, NOW);
  assert.equal(report.verdict.headline, 'Recording healthy');
  assert.equal(report.verdict.tone, 'success');
  assert.deepEqual(
    report.notes.map((note) => [note.label, note.severity]),
    [
      ['Over the capture limit', 'neutral'],
      ['WebSocket bytes between calls', 'neutral'],
    ],
  );
  // Every loss and error counter is at zero, folded into one line.
  assert.equal(report.quiet.length, 6);
});

test('a full queue is loss, and errors outrank it', () => {
  const lost = recorderFixture({
    omissions: { ...omissions, queue_bytes_bytes: 4096, unrecorded_calls: 2 },
  });
  assert.deepEqual(RecorderReport.capture(lost), { kind: 'lost', calls: 2, bytes: 4096 });
  const report = new RecorderReport(lost, null, null, null, NOW);
  assert.equal(report.verdict.tone, 'warning');
  assert.deepEqual(
    report.indicators.map((one) => [one.key, one.value, one.tone]),
    [
      ['lost', '4.0 KiB', 'warning'],
      ['unrecorded', '2', 'warning'],
      ['errors', '0', 'success'],
      ['queue', '0%', 'success'],
      ['commit', `${report.recorder.last_commit_ms} ms`, 'success'],
      ['lag', `${report.recorder.lag_ms} ms`, 'success'],
    ],
  );
  const failing = recorderFixture({ storage_errors: 1, omissions: lost.omissions });
  const worst = new RecorderReport(failing, null, null, null, NOW);
  assert.equal(worst.verdict.headline, 'Recording failing');
  assert.deepEqual(
    worst.notes.map((note) => note.severity),
    ['error', 'warning', 'warning'],
  );
  assert.equal(
    RecorderReport.lossText({ kind: 'lost', calls: 1, bytes: 0 }),
    '1 call not recorded',
  );
});

test('the limit nearest its end is the one retention prunes for first', () => {
  // 70% of storage, 5% of calls, 3 of 7 days of history.
  const gauges = RecorderReport.gauges(storage(500, 700), policy, NOW - 3 * DAY, NOW);
  assert.deepEqual(
    gauges.map((gauge) => [gauge.label, gauge.used, gauge.limit, gauge.binding]),
    [
      ['Storage', '683.6 KiB', '976.6 KiB', true],
      ['Calls', '500', '10,000', false],
      ['History', '3 days', '7 days', false],
    ],
  );
  assert.equal(gauges[0]?.share, 0.7);
  // History binds once it is the nearest.
  const old = RecorderReport.gauges(storage(500, 100), policy, NOW - 6.5 * DAY, NOW);
  assert.deepEqual(
    old.map((gauge) => gauge.binding),
    [false, false, true],
  );
  // With retention off there are no limits, so nothing binds.
  const off = RecorderReport.gauges(storage(500, 700), { ...policy, enabled: false }, null, NOW);
  assert.deepEqual(
    off.map((gauge) => [gauge.limit, gauge.share, gauge.binding]),
    [
      ['no limit', null, false],
      ['no limit', null, false],
      ['no limit', null, false],
    ],
  );
});

test('the queue, commit time and lag are toned by their own limits', () => {
  const busy = recorderFixture({
    queue: { ...recorderFixture().queue, items: 95, capacity: 100 },
    max_commit_ms: RecorderReport.COMMIT_WARN_MS,
    lag_ms: RecorderReport.LAG_ERROR_MS,
  });
  const tones = Object.fromEntries(
    new RecorderReport(busy, null, null, null, NOW).indicators.map((one) => [one.key, one.tone]),
  );
  assert.equal(tones.queue, 'error');
  assert.equal(tones.commit, 'warning');
  assert.equal(tones.lag, 'error');
});

test('fullness reads a share against the queue limits', () => {
  assert.equal(RecorderReport.fullness(0.5), 'fine');
  assert.equal(RecorderReport.fullness(RecorderReport.QUEUE_WARN), 'warning');
  assert.equal(RecorderReport.fullness(0.95), 'error');
});

test('a capacity meter keeps its own colour until it is crowded', () => {
  assert.equal(RecorderReport.meterTone(0.5), 'chart');
  assert.equal(RecorderReport.meterTone(RecorderReport.QUEUE_WARN), 'warning');
  assert.equal(RecorderReport.meterTone(RecorderReport.QUEUE_ERROR), 'error');
});

test('a body steps down from referenced to unique to compressed bytes', () => {
  const steps = RecorderReport.bodySteps({
    ...storageViewFixture(),
    referenced_bytes: 4000,
    unique_bytes: 1250,
    compressed_bytes: 0,
  });
  assert.deepEqual(
    steps.map((step) => [step.tone, step.bytes, step.note?.ratio ?? null]),
    [
      ['referenced', 4000, null],
      ['unique', 1250, '3.20×'],
      ['compressed', 0, '—'],
    ],
  );
  assert.equal(RecorderReport.ratio(4000, 1250), '3.20×');
  assert.equal(RecorderReport.ratio(1, 0), '—');
});

test('disk usage splits each file into live and free pages', () => {
  const view = storageViewFixture();
  assert.equal(RecorderReport.free(view.disk.metadata), 2 * 4096);
  assert.deepEqual(
    RecorderReport.diskParts(view).map((part) => [part.key, part.bytes]),
    [
      ['meta-live', 8 * 4096],
      ['meta-free', 2 * 4096],
      ['blob-live', 8 * 4096],
      ['blob-free', 2 * 4096],
    ],
  );
});

test('the queue meters read records and bytes against their capacity', () => {
  const meters = RecorderReport.queueMeters(
    recorderFixture({ queue: { items: 256, capacity: 1024, bytes: 0, byte_capacity: 0 } }),
  );
  assert.deepEqual(
    meters.map((meter) => [meter.label, meter.share]),
    [
      ['Records', 0.25],
      ['Bytes', 0],
    ],
  );
  assert.equal(meters[0]?.text, '256 / 1,024');
});

test('only storage filling up needs a look: a full call limit is retention working', () => {
  const gauge = (key: 'storage' | 'calls' | 'history', share: number) =>
    ({ key, label: key, used: '', limit: '', share, binding: false }) as const;
  assert.equal(RecorderReport.limitFullness(gauge('storage', 0.95)), 'error');
  assert.equal(RecorderReport.limitFullness(gauge('storage', 0.75)), 'warning');
  assert.equal(RecorderReport.limitFullness(gauge('calls', 1)), 'fine');
  assert.equal(RecorderReport.limitFullness(gauge('history', 1)), 'fine');
});
