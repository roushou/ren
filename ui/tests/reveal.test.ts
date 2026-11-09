import test from 'node:test';
import assert from 'node:assert/strict';
import { Reveal } from '../src/lib/reveal.ts';

test('a reveal catches up with each update at a steady rate', () => {
  const reveal = new Reveal();
  const first = 'x'.repeat(120);
  reveal.update(first);
  assert.equal(reveal.text, '');
  // 120 characters behind: a rate that catches up in CATCH_UP_MS.
  reveal.advance(Reveal.CATCH_UP_MS / 2);
  assert.equal(reveal.text.length, 60);
  reveal.advance(Reveal.CATCH_UP_MS / 2);
  assert.equal(reveal.text, first);
  assert.ok(reveal.done);
  // A small update still moves at the minimum rate, not instantly.
  reveal.update(first + 'yz');
  reveal.advance(1);
  assert.equal(reveal.text, first);
  reveal.advance(1000);
  assert.equal(reveal.text, first + 'yz');
});

test('text whose start was cut away is shown at once, and finish shows all', () => {
  const reveal = new Reveal('hello');
  assert.equal(reveal.text, 'hello');
  reveal.update('llo world');
  assert.equal(reveal.text, 'llo world');
  reveal.update('llo world, again and again');
  reveal.finish();
  assert.equal(reveal.text, 'llo world, again and again');
});
