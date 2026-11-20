import test from 'node:test';
import assert from 'node:assert/strict';
import { SearchResults } from '../src/lib/search.ts';
import type { SearchHit } from '../src/lib/api.ts';

const DAY = 86_400_000;
const dayStart = (ms: number) => Math.floor(ms / DAY) * DAY;
const hit = (call: number, recording: string | null, at: number, field: string): SearchHit =>
  ({
    call: `c${call}`,
    recording,
    turn: null,
    client: null,
    started_at: at,
    field,
    snippet: '',
  }) as never;

test('hits group by recording, newest group first, ticked across each group’s span', () => {
  const results = new SearchResults(
    [
      hit(1, 'r-a', 10 * DAY + 1_000, 'prompt'),
      hit(2, 'r-b', 12 * DAY, 'reply'),
      hit(3, 'r-a', 10 * DAY + 3_000, 'tool_output'),
      hit(4, null, 11 * DAY, 'reply'),
    ],
    dayStart,
  );
  assert.deepEqual(
    results.groups.map((group) => [group.recording, group.ticks.map((tick) => tick.at)]),
    [
      ['r-b', [0.5]],
      [null, [0.5]],
      ['r-a', [0, 1]],
    ],
  );
  assert.deepEqual(
    results.days.map((day) => [day.key / DAY, day.count]),
    [
      [10, 2],
      [11, 1],
      [12, 1],
    ],
  );
  assert.deepEqual(
    results.fields.map((field) => [field.key, field.count, field.share]),
    [
      ['reply', 2, 1],
      ['prompt', 1, 0.5],
      ['tool_output', 1, 0.5],
    ],
  );
  assert.deepEqual(results.recordings[0], { key: 'r-a', count: 2, share: 1 });
});
