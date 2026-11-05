import test from 'node:test';
import assert from 'node:assert/strict';
import { emptyFilters, trafficParams, selectedCall } from '../src/lib/traffic.ts';
import { callFixture, finishedFixture, callId } from './fixtures.ts';

test('empty traffic, failed initial loads, and dismissed selections have no inspector', () => {
  assert.equal(selectedCall(undefined, []), undefined);
  assert.equal(selectedCall(null, []), undefined);
  assert.equal(selectedCall(callId(404), []), undefined);
  const previous = finishedFixture(callId(1));
  assert.equal(selectedCall(null, [previous], previous, previous), undefined);
  assert.equal(selectedCall(undefined, [], previous, previous), undefined);
});

test('selection retains off-page calls and finished live observations for the matching ID', () => {
  const open = callFixture(callId(1));
  const finished = finishedFixture(callId(1));
  const other = callFixture(callId(2));
  assert.equal(selectedCall(callId(1), [open], other), open);
  assert.equal(selectedCall(callId(1), [open], finished), finished);
  // An open observation never replaces the listed row.
  assert.equal(selectedCall(callId(1), [finished], open), finished);
  assert.equal(selectedCall(callId(1), [], other, open), open);
  assert.equal(selectedCall(callId(1), [], other, other), undefined);
});

test('traffic filters encode literals and local date bounds, and reject invalid ranges', () => {
  const filters = {
    ...emptyFilters(),
    q: ' a&b%_ ',
    provider: 'openai',
    has_tools: true,
    from: '2026-11-03T10:00',
    until: '2026-11-03T11:00',
  };
  const params = new URLSearchParams(trafficParams(filters));
  assert.equal(params.get('q'), 'a&b%_');
  assert.equal(params.get('provider'), 'openai');
  assert.equal(params.get('has_tools'), 'true');
  assert.equal(params.get('model'), null);
  assert.equal(Number(params.get('from_ms')), new Date(filters.from).getTime());
  assert.throws(() => trafficParams({ ...filters, until: '2026-11-03T09:00' }));
  assert.throws(() => trafficParams({ ...filters, from: 'invalid' }));
  assert.equal(trafficParams(emptyFilters()), '');
});

test('URL filters restore datetime-local values without changing their absolute timestamps', async () => {
  const { filtersFromParams } = await import('../src/lib/traffic.ts');
  const original = {
    ...emptyFilters(),
    q: 'a & b',
    has_tools: true,
    from: '2026-11-03T10:00',
    until: '2026-11-03T11:00',
  };
  assert.equal(
    trafficParams(filtersFromParams(new URLSearchParams(trafficParams(original)))),
    trafficParams(original),
  );
  assert.equal(filtersFromParams(new URLSearchParams('from_ms=invalid')).from, '');
});
