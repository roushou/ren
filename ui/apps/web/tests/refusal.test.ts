import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { controlRefusals } from '../src/lib/api.ts';
import { proxyRefusals, refusals, Refusals } from '../src/lib/refusal.ts';
import { RunCommandError } from '../src/lib/runs.ts';
import { ToolsError } from '../src/lib/tools.ts';

test('a refused body’s code is read into the vocabulary, keeping one this UI does not know', () => {
  assert.equal(Refusals.read({ api_version: 1, error: 'busy' }), 'busy');
  assert.equal(Refusals.read({ error: 'runs_off' }), 'runs_off');
  assert.equal(Refusals.read({ error: 'a_newer_code' }), 'a_newer_code');
  assert.equal(Refusals.read({ error: 7 }), null);
  assert.equal(Refusals.read(null), null);
});

test('every code has its own words; an unknown one is named', () => {
  for (const code of refusals) {
    assert.doesNotMatch(Refusals.say(code), /^Refused: /, code);
  }
  assert.equal(Refusals.say('attachment_unrecognized'), 'Not a PNG, JPEG, GIF, or WebP image.');
  assert.equal(Refusals.say('no_images'), 'This client does not take images.');
  assert.equal(
    Refusals.say(Refusals.read({ error: 'something_new' }) ?? 'busy'),
    'Refused: something_new.',
  );
});

test('the UI server’s own codes are apart from the daemon’s, and are the ones it sends', () => {
  const daemon = new Set<string>(controlRefusals);
  for (const code of proxyRefusals) assert.ok(!daemon.has(code), code);
  // Every code the proxy refuses with is one it declares, and each it
  // declares is used (the type checker holds the first; this, the second).
  const proxy = readFileSync(new URL('../src/lib/server/control.ts', import.meta.url), 'utf8');
  const sent = new Set([...proxy.matchAll(/#refuse\([^,]+, '([a-z_]+)'\)/g)].map((m) => m[1]));
  assert.deepEqual([...sent].sort(), [...proxyRefusals].sort());
});

test('errors say the code’s words, or the status without one', () => {
  assert.equal(new RunCommandError(409, 'busy').message, 'A turn is already running.');
  assert.equal(new RunCommandError(502, null).message, 'The command was refused (502).');
  assert.equal(
    new ToolsError(422, 'tools_refused', 'node 18 is not offered').message,
    'node 18 is not offered.',
  );
  assert.equal(
    new ToolsError(403, 'login_not_allowed', null).message,
    'Your Tailscale login may not change tools here.',
  );
  assert.equal(new ToolsError(409, 'tools_busy', null).message, Refusals.say('tools_busy'));
});

test('only the redirect refusals let another address be pasted', () => {
  assert.equal(Refusals.redirect('redirect_elsewhere'), true);
  assert.equal(Refusals.redirect('not_awaiting_answer'), false);
  assert.equal(Refusals.redirect(null), false);
});
