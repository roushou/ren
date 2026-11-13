import test from 'node:test';
import assert from 'node:assert/strict';
import { readSnapshot, retainSnapshot } from '../src/lib/snapshots.ts';
import {
  gatewaySnapshot,
  recorderSnapshot,
  RecordingDetails,
  RecordingsReader,
  offPageSelection,
  searchSnapshot,
  trafficSnapshot,
} from '../src/lib/loaders.ts';
import { ApiVersionError, ManagementApi, RecordingId } from '../src/lib/api.ts';
import {
  callFixture,
  callId,
  detailFixture,
  finishedFixture,
  healthBody,
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
  assert.match(retained.error?.message ?? '', /Cannot reach Ren/);
  assert.equal(retainSnapshot(previous, { ...failed, key: '/recordings/other' }).value, null);
  const recovered = await readSnapshot('/traffic', async () => ['new']);
  assert.equal(retainSnapshot(previous, recovered).error, null);
  assert.deepEqual(retainSnapshot(previous, recovered).value, ['new']);
});

test('gateway reads health and storage together and accept degraded recorder health', async () => {
  const calls: string[] = [];
  const result = await gatewaySnapshot(async (input) => {
    calls.push(path(input));
    return path(input) === '/health'
      ? versioned(healthBody({ storage_errors: 1 }), { status: 503 })
      : versioned(storageFixture());
  });
  assert.deepEqual(calls, ['/health', '/api/storage']);
  assert.ok(result.value);
  assert.equal(result.value.health.status, 'degraded');
  assert.equal(result.value.health.recorder.storage_errors, 1);
  assert.equal(result.value.storage.policy.max_calls, 100);
});

test('the gateway names its environment', async () => {
  const read = (body: object) =>
    gatewaySnapshot(async (input) =>
      path(input) === '/health' ? versioned(body) : versioned(storageFixture()),
    );
  const development = await read({ ...healthBody(), instance: { environment: 'development' } });
  assert.equal(development.value?.health.instance.environment, 'development');
});

test('every response is version-checked and a mismatch is reported distinctly', async () => {
  const unversioned = async () => Response.json(healthBody());
  for (const fetcher of [2, null, '1']
    .map((version) => async () => versioned(healthBody(), undefined, version))
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
  const current = await new ManagementApi(async () => versioned(storageFixture())).storage();
  assert.equal(current.capture.body_limit_bytes, 4096);
});

test('responses are parsed at the boundary: labels keep unknown text, shapes are enforced', async () => {
  const metrics = await recorderSnapshot(async () =>
    versioned({ recorder: recorderFixture(), storage: storageViewFixture() }),
  );
  assert.equal(metrics.value?.metrics.storage.disk.blobs.page_size, 4096);
  // The usage read failed to parse (this fake answers every path alike): no oldest call, no failure.
  assert.equal(metrics.value?.oldest, null);
  const sessions = await new RecordingsReader(new RecordingDetails()).read(async (input) => {
    if (path(input).startsWith('/api/calls')) return versioned({ calls: [], next: null });
    if (path(input).endsWith('/turns?limit=5')) return versioned({ turns: [] });
    assert.equal(path(input), '/api/sessions?limit=100');
    return versioned({ sessions: [sessionRow(7, 2)] });
  });
  // A label this UI does not know is kept as text.
  assert.equal(sessions.value?.sessions[0]?.provider, 'future_provider');
  // Tokens no call reported are null, never zero.
  assert.deepEqual(sessions.value?.sessions[0]?.tokens, {
    input: null,
    output: null,
    cache_read: null,
  });
  // A session with no turn yet is read, and says so: no prompt, no folder.
  assert.deepEqual(sessions.value?.details.get(uuid(7) as never), {
    prompt: null,
    workdir: null,
  });
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
    if (path(input).startsWith('/api/sessions?')) return versioned({ sessions: [] });
    return path(input).startsWith('/api/calls?')
      ? versioned({ calls: [], next: null })
      : versioned(detailFixture(older));
  };
  const url = new URL(`http://localhost/traffic?call=${older.id}&model=fixture&unknown=ignored`);
  const listing = await trafficSnapshot(fetcher, url, null);
  // The listing (model calls by default), the sessions for the picker, and
  // how many calls the default hides; never the selected call.
  assert.deepEqual(requested, [
    '/api/calls?limit=100&model=fixture&kind=model',
    '/api/sessions?limit=100',
    '/api/calls?limit=100&model=fixture&kind=other',
  ]);
  assert.deepEqual(listing.value?.calls, []);
  assert.equal(listing.value?.hidden, 0);

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
  const session = RecordingId.parse(uuid(3));
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
  assert.equal(RecordingId.parse('../evil'), null);
});

test('search reads nothing without a query and keeps only a known field', async () => {
  const empty = await searchSnapshot(async () => {
    throw new Error('Must not search without a query');
  }, new URL('http://ui.test/search?q=%20%20&field=prompt'));
  assert.deepEqual([empty.value?.q, empty.value?.field, empty.value?.page], ['', 'prompt', null]);

  const calls: string[] = [];
  const fetcher: typeof fetch = async (input) => {
    calls.push(path(input));
    return versioned({ hits: [], indexed_calls: 3 });
  };
  const known = await searchSnapshot(
    fetcher,
    new URL('http://ui.test/search?q=a%26b&field=tool_output'),
  );
  assert.deepEqual(
    [known.value?.q, known.value?.field, known.value?.page],
    ['a&b', 'tool_output', { hits: [], indexed_calls: 3 }],
  );
  const unknown = await searchSnapshot(
    fetcher,
    new URL('http://ui.test/search?q=a&field=everything'),
  );
  assert.equal(unknown.value?.field, null);
  // Each search also reads the session listing, to name its hits' sessions.
  assert.deepEqual(calls, [
    '/api/search?q=a%26b&limit=50&field=tool_output',
    '/api/sessions?limit=100',
    '/api/search?q=a&limit=50',
    '/api/sessions?limit=100',
  ]);
  // The key encodes the query, so `&` in the text stays part of it.
  assert.equal(known.key, '/search?q=a%26b&field=tool_output');
  assert.equal(unknown.key, '/search?q=a&field=');
});

function sessionRow(n: number, lastSeen: number, calls = 3, turns = 1) {
  return {
    id: uuid(n),
    provider: 'future_provider',
    source: 'codex_headers',
    native_id: 'native',
    first_seen: 1,
    last_seen: lastSeen,
    call_count: calls,
    turn_count: turns,
    error_count: 0,
    tokens: { input: null, output: null, cache_read: null },
    archived_at: null,
  };
}

test('sessions are read in detail once per activity, and single-call ones not at all', async () => {
  const reader = new RecordingsReader(new RecordingDetails());
  const turnReads: string[] = [];
  let lastSeen = 2;
  const fetcher = async (input: RequestInfo | URL) => {
    const where = path(input);
    if (where.startsWith('/api/calls')) return versioned({ calls: [], next: null });
    if (where.endsWith('/turns?limit=5')) {
      turnReads.push(where);
      return versioned({ turns: [] });
    }
    // Session 8 is one call with no turn: a client's own check.
    return versioned({ sessions: [sessionRow(7, lastSeen), sessionRow(8, 2, 1, 0)] });
  };
  await reader.read(fetcher);
  await reader.read(fetcher);
  assert.deepEqual(turnReads, [`/api/sessions/${uuid(7)}/turns?limit=5`]);
  // New activity reads it again.
  lastSeen = 3;
  await reader.read(fetcher);
  assert.equal(turnReads.length, 2);
});

test('sessions are read a few at a time', async () => {
  let inFlight = 0;
  let most = 0;
  const fetcher = async (input: RequestInfo | URL) => {
    const where = path(input);
    if (where.startsWith('/api/calls')) return versioned({ calls: [], next: null });
    if (where.includes('/turns?')) {
      inFlight += 1;
      most = Math.max(most, inFlight);
      await new Promise((resolve) => setTimeout(resolve, 1));
      inFlight -= 1;
      return versioned({ turns: [] });
    }
    return versioned({
      sessions: Array.from({ length: 20 }, (_, n) => sessionRow(20 + n, 2)),
    });
  };
  const result = await new RecordingsReader(new RecordingDetails()).read(fetcher);
  assert.equal(result.value?.details.size, 20);
  assert.equal(most, RecordingDetails.parallel);
});
