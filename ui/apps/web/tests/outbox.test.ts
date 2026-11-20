import test from 'node:test';
import assert from 'node:assert/strict';
import { Outbox, type Prompting } from '../src/lib/outbox.ts';
import { RunCommandError, SessionId } from '../src/lib/runs.ts';

const session = SessionId.parse('00000000-0000-0000-0000-0000000000b1')!;
const other = SessionId.parse('00000000-0000-0000-0000-0000000000c2')!;
const prompt = (text: string, on = session): Prompting => ({
  session: on,
  text,
  images: [],
  delivery: 'start',
});

test('a prompt is on its way at once, then sent, then gone once echoed', () => {
  const { outbox, key } = new Outbox().sending(prompt('run the tests'), [], 'client');
  assert.deepEqual(
    outbox.of(session).map((item) => item.progress),
    [{ kind: 'sending', starts: 'client' }],
  );
  assert.equal(outbox.waiting(session), true);
  assert.equal(outbox.waiting(other), false);
  const sent = outbox.sent(key);
  assert.deepEqual(
    sent.of(session).map((item) => item.progress),
    [{ kind: 'sent' }],
  );
  assert.equal(sent.waiting(session), false);
  // The conversation shows it: the client echoed it.
  assert.deepEqual(sent.shown(session, ['run the tests']).items, []);
});

test('a refusal was not sent; no answer is unknown, never sent', () => {
  const { outbox, key } = new Outbox().sending(prompt('go'), [], 'nothing');
  assert.deepEqual(outbox.failed(key, new RunCommandError(409, null)).of(session)[0]?.progress, {
    kind: 'refused',
    reason: 'The command was refused (409).',
  });
  assert.equal(
    outbox.failed(key, new RunCommandError(502, null)).of(session)[0]?.progress.kind,
    'unknown',
  );
  assert.deepEqual(
    outbox.failed(key, new DOMException('late', 'TimeoutError')).items[0]?.progress,
    {
      kind: 'unknown',
      reason: 'No answer came in time.',
    },
  );
  assert.deepEqual(outbox.failed(key, new TypeError('Failed to fetch')).items[0]?.progress, {
    kind: 'unknown',
    reason: 'The connection failed.',
  });
  // An unknown one the client echoes after all was sent: it leaves.
  const unknown = outbox.failed(key, new TypeError('Failed to fetch'));
  assert.deepEqual(unknown.shown(session, ['go']).items, []);
});

test('an earlier prompt with the same text is not the echo', () => {
  const { outbox } = new Outbox().sending(prompt('continue'), ['continue', 'other'], 'nothing');
  assert.equal(outbox.shown(session, ['continue', 'other']).items.length, 1);
  assert.equal(outbox.shown(session, ['continue', 'other', 'continue']).items.length, 0);
});

test('two prompts with one text take their echoes in order', () => {
  const first = new Outbox().sending(prompt('again'), [], 'nothing');
  const second = first.outbox.sending(prompt('again'), [], 'nothing');
  const once = second.outbox.shown(session, ['again']);
  assert.deepEqual(
    once.items.map((item) => item.key),
    [second.key],
  );
  assert.deepEqual(once.shown(session, ['again', 'again']).items, []);
});

test("one session's conversation leaves another's prompts alone", () => {
  const mine = new Outbox().sending(prompt('hello'), [], 'nothing');
  const theirs = mine.outbox.sending(prompt('hello', other), [], 'nothing');
  const after = theirs.outbox.shown(session, ['hello']);
  assert.deepEqual(
    after.items.map((item) => item.session),
    [other],
  );
  // Nothing changed: the same outbox.
  assert.equal(after.shown(session, ['hello']), after);
  assert.deepEqual(after.without(theirs.key).items, []);
});
