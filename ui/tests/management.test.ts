import test from 'node:test';
import assert from 'node:assert/strict';
import { gatewayOrigin } from '../src/lib/server/config.ts';
import { managementResponse } from '../src/lib/server/management.ts';

test('gateway configuration accepts origins and rejects credentials and URL components', () => {
  assert.equal(gatewayOrigin('http://127.0.0.1:8787/'), 'http://127.0.0.1:8787');
  assert.equal(gatewayOrigin('https://vps.example:8443'), 'https://vps.example:8443');
  for (const value of [
    '',
    '/api',
    'file:///tmp/db',
    'http://user:secret@localhost',
    'http://localhost/api',
    'http://localhost/?token=secret',
    'http://localhost/#fragment',
  ]) {
    assert.throws(() => gatewayOrigin(value), /Gateway URL/);
  }
});

test('management reads preserve filters and degraded status without forwarding credentials or cookies', async () => {
  const request = new Request('http://ui.test/api/calls?model=a%2Bb&limit=20', {
    headers: {
      authorization: 'Bearer browser-secret',
      cookie: 'session=private',
      'x-forwarded-host': 'elsewhere',
    },
  });
  const response = await managementResponse(
    'http://daemon.test:8787',
    request,
    async (url, init) => {
      assert.equal(String(url), 'http://daemon.test:8787/api/calls?model=a%2Bb&limit=20');
      assert.deepEqual(init?.headers, { accept: 'application/json' });
      assert.equal(init?.redirect, 'manual');
      assert.equal(init?.method, 'GET');
      assert.ok(init?.signal);
      return Response.json(
        { storage_errors: 1 },
        { status: 503, headers: { 'set-cookie': 'private', 'access-control-allow-origin': '*' } },
      );
    },
  );
  assert.equal(response.status, 503);
  assert.deepEqual(await response.json(), { storage_errors: 1 });
  assert.equal(response.headers.get('cache-control'), 'no-store');
  assert.equal(response.headers.get('set-cookie'), null);
  assert.equal(response.headers.get('access-control-allow-origin'), null);
});

test('UI bridge refuses inference, unknown endpoints and writes before making a request', async () => {
  const noFetch: typeof fetch = async () => {
    throw new Error('Must not contact daemon');
  };
  for (const path of [
    '/anthropic/v1/messages',
    '/codex/responses',
    '/api/unknown',
    '/api/calls/not-an-id',
    '/api/calls/11111111-1111-4111-8111-11111111111',
    '/api//evil.test',
    '/api/traffic',
    '/api/attempts',
    '/api/attempts/11111111-1111-4111-8111-111111111111',
    '/api/sessions/11111111-1111-4111-8111-111111111111/timeline',
    '/api/sessions/11111111-1111-4111-8111-111111111111/report/extra',
    '/api/turns',
    '/api/turns/not-an-id',
    '/api/calls/11111111-1111-4111-8111-111111111111/activity/extra',
    '/api/live/extra',
    '/api/calls/11111111-1111-4111-8111-111111111111/body/headers',
  ]) {
    const response = await managementResponse(
      'http://daemon.test',
      new Request(`http://ui.test${path}`),
      noFetch,
    );
    assert.equal(response.status, 404);
  }
  const response = await managementResponse(
    'http://daemon.test',
    new Request('http://ui.test/api/storage', { method: 'POST' }),
    noFetch,
  );
  assert.equal(response.status, 405);
});

test('redirects and unavailable daemons produce sanitized gateway errors', async () => {
  for (const fetcher of [
    async () => Response.redirect('https://elsewhere.test/secret'),
    async () => {
      throw new Error('Sensitive internal transport details');
    },
  ]) {
    const response = await managementResponse(
      'http://daemon.test',
      new Request('http://ui.test/health'),
      fetcher,
    );
    assert.equal(response.status, 502);
    assert.equal(response.headers.get('location'), null);
    assert.equal(response.headers.get('cache-control'), 'no-store');
    assert.doesNotMatch(await response.text(), /secret|Sensitive/);
  }
});

test('every allowlisted read uses the same bounded read bridge', async () => {
  const id = '11111111-1111-4111-8111-111111111111';
  for (const path of [
    '/health',
    '/api/metrics',
    '/api/storage',
    '/api/calls',
    `/api/calls/${id}`,
    `/api/calls/${id}/body/request`,
    `/api/calls/${id}/body/response`,
    '/api/sessions',
    `/api/sessions/${id}/calls`,
    `/api/sessions/${id}/turns`,
    `/api/sessions/${id}/report`,
    `/api/turns/${id}`,
    `/api/calls/${id}/activity`,
    '/api/search',
    '/api/exposures',
    '/api/usage',
  ]) {
    const response = await managementResponse(
      'http://daemon.test',
      new Request(`http://ui.test${path}`),
      async () => Response.json({ fixture: true }),
    );
    assert.equal(response.status, 200);
    assert.deepEqual(await response.json(), { fixture: true });
  }
});

test('the live feed streams unbuffered without a total timeout and forwards only a valid Last-Event-ID', async () => {
  for (const [sent, forwarded] of [
    ['42', '42'],
    ['42; drop table', undefined],
    ['-1', undefined],
    [null, undefined],
  ] as const) {
    const browser = new AbortController();
    let upstreamSignal: AbortSignal | undefined;
    let push: ((chunk: string) => void) | undefined;
    const request = new Request('http://ui.test/api/live?after=7', {
      signal: browser.signal,
      headers: {
        ...(sent === null ? {} : { 'last-event-id': sent }),
        cookie: 'session=private',
        authorization: 'Bearer browser-secret',
      },
    });
    const response = await managementResponse('http://daemon.test', request, async (url, init) => {
      assert.equal(String(url), 'http://daemon.test/api/live?after=7');
      const headers = init?.headers as Record<string, string>;
      assert.deepEqual(
        headers,
        forwarded === undefined
          ? { accept: 'text/event-stream' }
          : { accept: 'text/event-stream', 'last-event-id': forwarded },
      );
      assert.equal(init?.redirect, 'manual');
      upstreamSignal = init?.signal ?? undefined;
      const encoder = new TextEncoder();
      const body = new ReadableStream<Uint8Array>({
        start(controller) {
          push = (chunk) => controller.enqueue(encoder.encode(chunk));
        },
      });
      return new Response(body, {
        headers: { 'content-type': 'text/event-stream', 'set-cookie': 'private' },
      });
    });
    assert.equal(response.status, 200);
    assert.equal(response.headers.get('content-type'), 'text/event-stream');
    assert.equal(response.headers.get('cache-control'), 'no-store');
    assert.equal(response.headers.get('x-content-type-options'), 'nosniff');
    assert.equal(response.headers.get('set-cookie'), null);
    // Each event arrives before the upstream stream ends.
    const reader = response.body!.getReader();
    const decoder = new TextDecoder();
    // An SSE comment opens the stream at once, so EventSource reports it open.
    assert.equal(decoder.decode((await reader.read()).value), ': connected\n\n');
    push!('id: 43\nevent: observation\ndata: {}\n\n');
    assert.match(decoder.decode((await reader.read()).value), /id: 43/);
    push!('event: lagged\ndata: {"resume_from":43}\n\n');
    assert.match(decoder.decode((await reader.read()).value), /lagged/);
    // Only the browser request aborts the upstream read.
    assert.equal(upstreamSignal?.aborted, false);
    browser.abort();
    assert.equal(upstreamSignal?.aborted, true);
    await reader.cancel().catch(() => {});
  }
});
