import test from 'node:test';
import assert from 'node:assert/strict';
import { Questions, type Draft, type Question } from '../src/lib/questions.ts';
import { ShapeError } from '../src/lib/decode.ts';

const store = Questions.decode(
  {
    key: 'Which store?',
    header: 'Store',
    text: 'Which store?',
    detail: null,
    form: {
      form: 'choice',
      options: [
        { label: 'SQLite', description: 'One file' },
        { label: 'Postgres', description: null },
      ],
      pick: 'one',
      write_in: 'allowed',
    },
  },
  'question',
);

const features = Questions.decode(
  {
    key: 'features',
    header: null,
    text: 'Which features?',
    detail: null,
    form: {
      form: 'choice',
      options: [
        { label: 'Search', description: null },
        { label: 'Export', description: null },
      ],
      pick: 'many',
      write_in: 'refused',
    },
  },
  'question',
);

const token = Questions.decode(
  {
    key: 'token',
    header: 'Token',
    text: 'Which token?',
    detail: null,
    form: { form: 'text', lines: 'one', echo: 'hidden', placeholder: null, prefill: null },
  },
  'question',
);

test('a question decodes with its form, and an unknown form is kept to be skipped', () => {
  assert.equal(store.form.form, 'choice');
  const newer = Questions.decode(
    { key: 'k', header: null, text: 'Rate it', detail: null, form: { form: 'stars' } },
    'question',
  );
  assert.deepEqual(newer.form, { form: 'unrecognized', name: 'stars' });
  assert.deepEqual(Questions.draft(newer), { form: 'none' });
  assert.throws(
    () => Questions.decode({ key: 'k', header: null, form: { form: 'confirm' } }, 'q'),
    ShapeError,
    'a question has its text',
  );
});

test('a single choice keeps one answer: a pick, or the operator’s own words', () => {
  let draft: Draft = Questions.draft(store);
  draft = Questions.pick(store, draft, 'SQLite');
  draft = Questions.pick(store, draft, 'Postgres');
  assert.deepEqual(Questions.reply(store, draft), { as: 'chose', picked: ['Postgres'] });
  draft = Questions.write(store, draft, 'Both, behind a flag');
  assert.deepEqual(Questions.reply(store, draft), {
    as: 'chose',
    picked: [],
    written: 'Both, behind a flag',
  });
  draft = Questions.pick(store, draft, 'SQLite');
  assert.deepEqual(Questions.reply(store, draft), { as: 'chose', picked: ['SQLite'] });
  // Picking it again takes it back: nothing answered is a skip.
  draft = Questions.pick(store, draft, 'SQLite');
  assert.deepEqual(Questions.reply(store, draft), { as: 'skipped' });
});

test('a choice of many toggles each option', () => {
  let draft: Draft = Questions.draft(features);
  draft = Questions.pick(features, draft, 'Search');
  draft = Questions.pick(features, draft, 'Export');
  draft = Questions.pick(features, draft, 'Search');
  assert.deepEqual(Questions.reply(features, draft), { as: 'chose', picked: ['Export'] });
});

test('a hidden text is sent as a secret, and blank text skips', () => {
  assert.deepEqual(Questions.reply(token, { form: 'text', text: 'tok-acme' }), {
    as: 'secret',
    text: 'tok-acme',
  });
  assert.deepEqual(Questions.reply(token, { form: 'text', text: '  ' }), { as: 'skipped' });
});

test('the request carries a reply for every question, by key', () => {
  const questions: Question[] = [store, features];
  const drafts: Draft[] = [Questions.pick(store, Questions.draft(store), 'SQLite')];
  assert.ok(Questions.answered(questions, drafts));
  assert.deepEqual(Questions.request(questions, drafts), {
    replies: {
      'Which store?': { as: 'chose', picked: ['SQLite'] },
      features: { as: 'skipped' },
    },
  });
  assert.ok(!Questions.answered(questions, []), 'nothing answered declines instead');
  assert.equal(Questions.summary(questions), 'Which store? (and 1 more)');
});
