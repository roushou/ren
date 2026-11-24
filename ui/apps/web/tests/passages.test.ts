import test from 'node:test';
import assert from 'node:assert/strict';
import type { UnknownLabel } from '../src/lib/decode.ts';
import { Reach, passages, type Passage } from '../src/lib/passages.ts';

function passage(asked: string, end: string, count: number, last_at: number): Passage {
  return {
    asked,
    end: end as Passage['end'],
    through: end === 'closed' ? '*.acme.dev:443' : null,
    listed_by: end === 'closed' ? 'project' : null,
    count,
    sent: 0,
    received: 0,
    first_at: 0,
    last_at,
  };
}

test('a session lists each place once per end, refusals first, then the newest', () => {
  const rows = Reach.rows([
    [
      passage('registry.acme.dev:443', 'closed', 2, 300),
      passage('example.com:443', 'unlisted', 1, 100),
    ],
    [
      passage('registry.acme.dev:443', 'closed', 3, 500),
      passage('cdn.acme.dev:443', 'timed_out', 1, 400),
    ],
  ]);
  assert.deepEqual(
    rows.map((row) => [row.asked, row.mark, row.count]),
    [
      ['example.com:443', 'failed', 1],
      ['registry.acme.dev:443', 'passed', 5],
      ['cdn.acme.dev:443', 'warning', 1],
    ],
  );
  assert.equal(rows[0]?.said, 'Refused: not listed');
  assert.equal(rows[1]?.said, 'Reached through *.acme.dev:443, listed by its project');
});

test('an end this UI does not know is kept and shown as unknown', () => {
  const [row] = Reach.rows([[passage('acme.dev:22', 'held_back', 1, 1)]]);
  assert.equal(row?.mark, 'unknown');
  assert.equal(row?.said, 'held_back');
  assert.equal(Reach.refused('held_back' as Passage['end']), false);
});

test('the read API body decodes, keeping an unknown end', () => {
  const body = {
    passages: [
      {
        asked: 'registry.acme.dev:443',
        end: 'closed',
        through: '*.acme.dev:443',
        listed_by: 'configuration',
        count: 2,
        sent: 150,
        received: 3000,
        first_at: 1,
        last_at: 2,
      },
      {
        asked: 'GET',
        end: 'later_end',
        through: null,
        listed_by: null,
        count: 1,
        sent: 0,
        received: 0,
        first_at: 3,
        last_at: 3,
      },
    ],
  };
  const read = passages(body, '').passages;
  assert.equal(read[0]?.through, '*.acme.dev:443');
  assert.equal(read[0]?.listed_by, 'configuration');
  assert.equal(read[1]?.end, 'later_end');
});

test('a list this UI does not know is named as stored', () => {
  assert.equal(Reach.list('workspace' as UnknownLabel), 'workspace');
  assert.equal(Reach.list('configuration'), "Ren's configuration");
});
