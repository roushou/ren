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
  assert.equal(report.verdict.facts[0], '2 calls and 4.0 KiB not captured');
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
      ['History', 'oldest 3 days', '7 days', false],
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
  assert.deepEqual([30 * 60_000, 5 * 3_600_000, 3.5 * DAY].map(RecorderReport.duration), [
    '30 min',
    '5 h',
    '3.5 days',
  ]);
  assert.deepEqual([0, 0.002, 0.07].map(RecorderReport.percent), ['0%', '<1%', '7%']);
});
