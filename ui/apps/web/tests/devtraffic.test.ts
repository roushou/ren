import test from 'node:test';
import assert from 'node:assert/strict';
import { DevTraffic, trafficStatus } from '../src/lib/devtraffic.ts';
import { devTrafficOrigin } from '../src/lib/server/config.ts';
import { devTrafficResponse } from '../src/lib/server/devtraffic.ts';

const status = {
  settings: {
    flow: 'running',
    rate: 12,
    pace: 'normal',
    replies: { text: 60, tools: 25 },
    harnesses: { claude_code: 45 },
    senders: { alice: 30 },
  },
  sent: 3,
  in_flight: 1,
};

test('the traffic status is parsed at the boundary', () => {
  assert.equal(trafficStatus(status, 'status').settings.rate, 12);
  assert.throws(() =>
    trafficStatus(
      { ...status, settings: { ...status.settings, replies: { text: 101 } } },
      'status',
    ),
  );
  assert.throws(() => trafficStatus({ ...status, sent: -1 }, 'status'));
});

test('the dev traffic URL is a loopback HTTP origin only', () => {
  assert.equal(devTrafficOrigin('http://127.0.0.1:13001'), 'http://127.0.0.1:13001');
  for (const value of ['https://127.0.0.1:13001', 'http://10.0.0.2:13001', 'http://example.com']) {
    assert.throws(() => devTrafficOrigin(value));
  }
});

test('the proxy forwards only its three requests, JSON only, without cookies', async () => {
  const seen: { url: string; init?: RequestInit }[] = [];
  const fetcher = (async (url: URL, init?: RequestInit) => {
    seen.push({ url: String(url), init });
    return new Response('{}', { headers: { 'content-type': 'application/json' } });
  }) as unknown as typeof fetch;
  const origin = 'http://127.0.0.1:13001';
  const ask = (path: string, init?: RequestInit) =>
    devTrafficResponse(origin, new Request(`http://ui.test${path}`, init), fetcher);

  assert.equal((await ask('/dev/traffic/status', { headers: { cookie: 'a=b' } })).status, 200);
  assert.equal(seen[0].url, 'http://127.0.0.1:13001/api/status');
  assert.deepEqual(seen[0].init?.headers, {});
  const json = {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: '{"count":1}',
  };
  assert.equal((await ask('/dev/traffic/burst', json)).status, 200);
  assert.equal(seen[1].url, 'http://127.0.0.1:13001/api/burst');
  assert.equal(
    (
      await ask('/dev/traffic/burst', {
        method: 'POST',
        body: 'count=1',
        headers: { 'content-type': 'application/x-www-form-urlencoded' },
      })
    ).status,
    415,
  );
  assert.equal((await ask('/dev/traffic/status', json)).status, 405);
  assert.equal((await ask('/dev/traffic/elsewhere')).status, 404);
  assert.equal(seen.length, 2);
  // Without the dev slot's setting, nothing is forwarded.
  const off = await devTrafficResponse(
    undefined,
    new Request('http://ui.test/dev/traffic/status'),
    fetcher,
  );
  assert.equal(off.status, 404);
  assert.equal(seen.length, 2);
});

test('weight names read as words', () => {
  assert.equal(DevTraffic.label('claude_code'), 'claude code');
});
