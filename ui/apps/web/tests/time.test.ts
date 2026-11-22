import test from 'node:test';
import assert from 'node:assert/strict';
import { Clock, Elapsed } from '../src/lib/time.ts';

const { MINUTE, HOUR, DAY } = Elapsed;

test('how long ago reads short: now, seconds, minutes, hours, then its day', () => {
  const now = 10 * DAY;
  const day = () => 'Tue';
  assert.deepEqual(
    [now, now - 4_000, now - 20_000, now - 18 * MINUTE, now - 3 * HOUR, now - 2 * DAY].map((at) =>
      Elapsed.ago(at, now, day),
    ),
    ['now', 'now', '20s', '18m', '3h', 'Tue'],
  );
  // A time ahead of the clock is now, never a negative age.
  assert.equal(Elapsed.ago(now + 60_000, now, day), 'now');
});

test('a span reads at a glance, in days from two days on', () => {
  assert.deepEqual(
    [45_000, 12 * MINUTE, 83 * MINUTE, 2 * HOUR, 30 * HOUR, 3.5 * DAY, 7 * DAY].map(Elapsed.span),
    ['45 s', '12 min', '1 h 23 min', '2 h', '30 h', '3.5 days', '7 days'],
  );
});

test('a latency reads in milliseconds, then seconds', () => {
  assert.deepEqual([null, 87.4, 999, 1_250].map(Elapsed.latency), [
    '—',
    '87 ms',
    '999 ms',
    '1.25 s',
  ]);
});

test('a step’s time reads in tenths, then seconds, then minutes', () => {
  assert.deepEqual(
    [0, 420, 2_500, 9_949, 12_400, 59_400, 64_000, 3_725_000].map(Elapsed.stopwatch),
    ['0.0 s', '0.4 s', '2.5 s', '9.9 s', '12 s', '59 s', '1:04', '62:05'],
  );
});

test('a day within the week is its weekday, an older one its date', () => {
  const now = Date.UTC(2026, 10, 22, 12);
  assert.notEqual(Clock.day(now - DAY, now), Clock.day(now - 30 * DAY, now));
  assert.match(Clock.minute(now), /\d/);
});
