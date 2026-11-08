import test from 'node:test';
import assert from 'node:assert/strict';
import { LaunchCommand, launchClients } from '../src/lib/launch.ts';

test('every client tgv run launches is offered, Pi and opencode included', () => {
  assert.deepEqual(
    launchClients.map((client) => client.run),
    ['claude', 'codex', 'pi', 'opencode'],
  );
});

test('the default listener needs no --gateway', () => {
  for (const origin of ['http://127.0.0.1:8787', 'http://localhost:8787'])
    for (const { run } of launchClients)
      assert.equal(LaunchCommand.for(run, origin).text, `tgv run ${run}`);
});

test('any other gateway is named', () => {
  assert.equal(
    LaunchCommand.for('pi', 'https://vps.example.ts.net:8443').text,
    'tgv run pi --gateway https://vps.example.ts.net:8443',
  );
  // Another port on loopback is a different daemon.
  assert.equal(
    LaunchCommand.for('codex', 'http://127.0.0.1:9000').text,
    'tgv run codex --gateway http://127.0.0.1:9000',
  );
});
