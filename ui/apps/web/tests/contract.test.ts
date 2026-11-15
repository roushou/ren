// The read API's JSON, as the daemon answers a fixed synthetic history
// (pinned by crates/ren-daemon/tests/contract.rs): every read the UI makes
// must decode it. When the daemon's answers change on purpose, regenerate
// with `REN_UPDATE_FIXTURES=1 cargo test -p ren-daemon --test contract`.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { CallId, ManagementApi, RecordingId, TurnId } from '../src/lib/api.ts';

const fixture = (name: string): unknown =>
  JSON.parse(readFileSync(new URL(`./fixtures/api/${name}.json`, import.meta.url), 'utf8'));

/** A `fetch` answering each read API path with its pinned answer. */
const pinned: typeof fetch = async (input) => {
  const url = new URL(String(input), 'http://ui.test');
  const path = url.pathname;
  const name = (
    [
      [/^\/api\/calls$/, 'calls'],
      [/^\/api\/calls\/[^/]+$/, 'call'],
      [/^\/api\/sessions$/, 'sessions'],
      [/^\/api\/sessions\/[^/]+\/calls$/, 'session_calls'],
      [/^\/api\/sessions\/[^/]+\/turns$/, 'turns'],
      [/^\/api\/sessions\/[^/]+\/report$/, 'report'],
      [/^\/api\/turns\/[^/]+$/, 'turn'],
      [/^\/api\/usage$/, 'usage'],
      [/^\/api\/search$/, 'search'],
      [/^\/api\/exposures$/, 'exposures'],
    ] as const
  ).find(([pattern]) => pattern.test(path))?.[1];
  assert.ok(name, `no fixture for ${path}`);
  return Response.json(fixture(name));
};

test('every read the UI makes decodes the daemon’s pinned answers', async () => {
  const api = new ManagementApi(pinned);
  const calls = await api.calls(new URLSearchParams());
  assert.equal(calls.calls.length, 2);
  const first = calls.calls[0];
  const callId = CallId.parse(first.id);
  assert.ok(callId);
  const detail = await api.call(callId);
  assert.equal(detail.call.id, first.id);

  const sessions = await api.sessions();
  const sessionId = RecordingId.parse(sessions.sessions[0].id);
  assert.ok(sessionId);
  assert.equal((await api.sessionCalls(sessionId, null)).calls.length, 2);
  const turns = await api.turns(sessionId);
  assert.equal(turns.turns[0].prompt?.text, 'Fix the parser');
  const turnId = TurnId.parse(turns.turns[0].turn);
  assert.ok(turnId);
  const turn = await api.turn(turnId);
  assert.equal(turn.calls.length, 2);
  assert.ok(turn.findings.some((finding) => finding.kind === 'secret_exposure'));
  const report = await api.report(sessionId);
  assert.equal(report.turns.length, 1);
  await api.usage({});

  const search = await api.search(new URLSearchParams({ q: 'parser' }));
  assert.ok(search.hits.length > 0);
  assert.match(search.hits[0].snippet, /«parser»/);
  const exposures = await api.exposures();
  assert.equal(exposures.exposures[0].hint, 'sk-ant-…a1b2');
});
