import test from 'node:test';
import assert from 'node:assert/strict';
import { readSnapshot, retainSnapshot } from '../src/lib/snapshots.ts';
import {
  gatewaySnapshot,
  recorderSnapshot,
  sessionsSnapshot,
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

test('gateway reads remain serial and accept degraded recorder health', async () => {
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
  const malformed = await trafficSnapshot(
    async () => versioned({ calls: [{ ...callFixture(), id: 'not-a-uuid' }], next: null }),
    new URL('http://localhost/traffic'),
    null,
  );
  assert.equal(malformed.value, null);
  assert.equal(malformed.error?.kind, 'failed');
  assert.match(malformed.error?.message ?? '', /calls\[0\]\.id/);
});

test('deep links fetch off-page calls and distinguish missing selection from a failed list', async () => {
  const older = finishedFixture(callId(9));
  const requested: string[] = [];
  const url = new URL(`http://localhost/traffic?call=${older.id}&model=fixture&unknown=ignored`);
  const loaded = await trafficSnapshot(
    async (input) => {
      requested.push(path(input));
      return path(input).startsWith('/api/calls?')
        ? versioned({ calls: [], next: null })
        : versioned(detailFixture(older));
    },
    url,
    null,
  );
  assert.deepEqual(requested, ['/api/calls?limit=100&model=fixture', `/api/calls/${older.id}`]);
  assert.equal(loaded.value?.selection.kind, 'loaded');
  const missing = await trafficSnapshot(
    async (input) =>
      path(input).startsWith('/api/calls?')
        ? versioned({ calls: [], next: null })
        : versioned({ error: 'not_found' }, { status: 404 }),
    url,
    null,
  );
  assert.equal(missing.error, null);
  const selection = missing.value?.selection;
  assert.ok(selection?.kind === 'failed');
  assert.match(selection.message, /no longer available/);
  const invalid = await trafficSnapshot(
    async () => versioned({ calls: [], next: null }),
    new URL('http://localhost/traffic?call=nope'),
    null,
  );
  assert.equal(invalid.value?.selection.kind, 'failed');
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
