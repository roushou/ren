import test from 'node:test';
import assert from 'node:assert/strict';
import { readSnapshot, retainSnapshot } from '../src/lib/snapshots.ts';
import {
  gatewaySnapshot,
  recorderSnapshot,
  sessionsSnapshot,
  offPageSelection,
  trafficSnapshot,
} from '../src/lib/loaders.ts';
import { ApiVersionError, ManagementApi, SessionId } from '../src/lib/api.ts';
import {
  callFixture,
  callId,
  detailFixture,
  finishedFixture,
  healthFixture,
  storageFixture,
  storageViewFixture,
  recorderFixture,
  uuid,
  versioned,
} from './fixtures.ts';

const path = (input: Parameters<typeof fetch>[0]) => {
  assert.ok(typeof input === 'string');
  return input;
};

test('background failure retains only the matching snapshot and recovery clears the error', async () => {
  const previous = await readSnapshot('/traffic', async () => ['retained']);
  const failed = await readSnapshot('/traffic', async () => {
    throw new TypeError('offline');
  });
  const retained = retainSnapshot(previous, failed);
  assert.deepEqual(retained.value, ['retained']);
  assert.equal(retained.updated, previous.updated);
  assert.equal(retained.error?.kind, 'unreachable');
  assert.match(retained.error?.message ?? '', /Cannot reach TGV/);
  assert.equal(retainSnapshot(previous, { ...failed, key: '/sessions/other' }).value, null);
  const recovered = await readSnapshot('/traffic', async () => ['new']);
  assert.equal(retainSnapshot(previous, recovered).error, null);
  assert.deepEqual(retainSnapshot(previous, recovered).value, ['new']);
});

test('gateway reads health and storage together and accept degraded recorder health', async () => {
  const calls: string[] = [];
  const result = await gatewaySnapshot(async (input) => {
    calls.push(path(input));
    return path(input) === '/health'
      ? versioned(healthFixture({ storage_errors: 1 }), { status: 503 })
      : versioned(storageFixture());
  });
  assert.deepEqual(calls, ['/health', '/api/storage']);
  assert.ok(result.value);
  assert.equal(result.value.health.status, 'degraded');
  assert.equal(result.value.health.recorder.storage_errors, 1);
  assert.equal(result.value.storage.policy.max_calls, 100);
});

test('every response is version-checked and a mismatch is reported distinctly', async () => {
  const unversioned = async () => Response.json(healthFixture());
  for (const fetcher of [2, null, '1']
    .map((version) => async () => versioned(healthFixture(), undefined, version))
    .concat(unversioned)) {
    const result = await gatewaySnapshot(fetcher);
    assert.equal(result.value, null);
    assert.equal(result.error?.kind, 'incompatible');
    assert.match(result.error?.message ?? '', /requires version 1/);
  }
  // Error bodies (non-2xx) keep their status message instead.
  const busy = await recorderSnapshot(async () => versioned({ error: 'busy' }, { status: 503 }));
  assert.equal(busy.error?.kind, 'failed');
  assert.match(busy.error?.message ?? '', /busy/);
  const unavailable = await recorderSnapshot(async () =>
    versioned({ error: 'unavailable' }, { status: 503 }),
  );
  assert.match(unavailable.error?.message ?? '', /storage is unavailable/);
  await assert.rejects(
    new ManagementApi(async () => versioned(storageFixture(), undefined, 7)).storage(),
    ApiVersionError,
  );
});

test('responses are parsed at the boundary: labels keep unknown text, shapes are enforced', async () => {
  const metrics = await recorderSnapshot(async () =>
    versioned({ recorder: recorderFixture(), storage: storageViewFixture() }),
  );
  assert.equal(metrics.value?.storage.disk.blobs.page_size, 4096);
  const sessions = await sessionsSnapshot(async (input) => {
    assert.equal(path(input), '/api/sessions?limit=100');
    return versioned({
      sessions: [
        {
          id: uuid(7),
          provider: 'future_provider',
          source: 'codex_headers',
          native_id: 'native',
          first_seen: 1,
          last_seen: 2,
          call_count: 3,
          turn_count: 1,
        },
      ],
    });
  });
  assert.equal(sessions.value?.[0].provider, 'future_provider');
  // A daemon older than session totals reports none: null, never zero.
  assert.equal(sessions.value?.[0].error_count, null);
  assert.deepEqual(sessions.value?.[0].tokens, { input: null, output: null, cache_read: null });
  const malformed = await trafficSnapshot(
    async () => versioned({ calls: [{ ...callFixture(), id: 'not-a-uuid' }], next: null }),
    new URL('http://localhost/traffic'),
    null,
  );
  assert.equal(malformed.value, null);
  assert.equal(malformed.error?.kind, 'failed');
  assert.match(malformed.error?.message ?? '', /calls\[0\]\.id/);
});

test('the listing ignores the selection, and off-page selections load on their own', async () => {
  const older = finishedFixture(callId(9));
  const requested: string[] = [];
  const fetcher = async (input: RequestInfo | URL) => {
    requested.push(path(input));
    return path(input).startsWith('/api/calls?')
      ? versioned({ calls: [], next: null })
      : versioned(detailFixture(older));
  };
  const url = new URL(`http://localhost/traffic?call=${older.id}&model=fixture&unknown=ignored`);
  const listing = await trafficSnapshot(fetcher, url, null);
  assert.deepEqual(requested, ['/api/calls?limit=100&model=fixture']);
  assert.deepEqual(listing.value?.calls, []);

  requested.length = 0;
  const loaded = await offPageSelection(fetcher, older.id, []);
  assert.deepEqual(requested, [`/api/calls/${older.id}`]);
  assert.ok(loaded.kind === 'loaded' && loaded.call.id === older.id);
  // A listed call needs no read.
  assert.deepEqual(await offPageSelection(fetcher, older.id, [older]), { kind: 'none' });
  assert.deepEqual(await offPageSelection(fetcher, null, []), { kind: 'none' });

  const missing = await offPageSelection(
    async () => versioned({ error: 'not_found' }, { status: 404 }),
    older.id,
    [],
  );
  assert.ok(missing.kind === 'failed');
  assert.match(missing.message, /no longer available/);
  assert.equal((await offPageSelection(fetcher, 'nope', [])).kind, 'failed');
});

test('session pages read the session listing with only a cursor', async () => {
  const session = SessionId.parse(uuid(3));
  assert.ok(session);
  const requested: string[] = [];
  const page = await trafficSnapshot(
    async (input) => {
      requested.push(path(input));
      return path(input).startsWith('/api/sessions?')
        ? versioned({ sessions: [] })
        : versioned({ calls: [finishedFixture(callId(1))], next: '5:cursor' });
    },
    new URL('http://localhost/sessions/x?after=1%3Aabc&model=ignored'),
    session,
  );
  assert.deepEqual(requested, [
    '/api/sessions?limit=100',
    `/api/sessions/${session}/calls?limit=100&after=1%3Aabc`,
  ]);
  assert.equal(page.value?.next, '5:cursor');
  assert.equal(SessionId.parse('../evil'), null);
});
