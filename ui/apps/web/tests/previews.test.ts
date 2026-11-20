// The factory's previews: the daemon's pinned answer decodes, each preview
// is reached where a browser can reach it, and asks go to the control API.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { Previews, PreviewsApi, type Preview } from '../src/lib/previews.ts';

const pinned = JSON.parse(
  readFileSync(new URL('./fixtures/api/project_previews.json', import.meta.url), 'utf8'),
) as unknown;

test('the pinned answer decodes', async () => {
  const list = await new PreviewsApi(async () => Response.json(pinned)).list('ren');
  assert.equal(list.previews.length, 2);
  const pull = Previews.ofPull(list, 12);
  assert.ok(pull);
  assert.deepEqual(pull.state, { state: 'running', reach: 'tailnet' });
  assert.deepEqual(Previews.state(pull), ['running', 'ok']);
  assert.deepEqual(Previews.note(pull), ['preview', 'ok']);
  assert.equal(Previews.ofPull(list, 9), null);
  const [asked] = Previews.asked(list);
  assert.equal(asked?.commit, '3'.repeat(40));
  assert.deepEqual(asked?.preview?.state, { state: 'failed', detail: 'the build failed' });
});

test('a preview opens where this browser can reach it', () => {
  const running = (reach: 'loopback' | 'tailnet'): Preview => ({
    slot: 0,
    of: { kind: 'pull', number: 12 },
    commit: 'a'.repeat(40),
    state: { state: 'running', reach },
    ui_port: 18_101,
    published_ui_port: 9_451,
  });
  const tailnet = new URL('https://ren.example.ts.net:9443/projects/ren');
  assert.equal(Previews.url(running('tailnet'), tailnet), 'https://ren.example.ts.net:9451');
  // Not published: nothing on the tailnet answers it.
  assert.equal(Previews.url(running('loopback'), tailnet), null);
  assert.deepEqual(Previews.state(running('loopback')), ['host only', 'quiet']);
  assert.deepEqual(Previews.note(running('loopback')), ['preview host only', 'quiet']);
  // On the host itself, its loopback port does.
  const local = new URL('http://127.0.0.1:13000/projects/ren');
  assert.equal(Previews.url(running('loopback'), local), 'http://127.0.0.1:18101');
  const building: Preview = { ...running('tailnet'), state: { state: 'building' } };
  assert.equal(Previews.url(building, tailnet), null);
});

test('states and kinds a newer daemon sends are kept', async () => {
  const body = {
    api_version: 1,
    previews: [
      {
        slot: 0,
        of: { kind: 'branch' },
        commit: 'a'.repeat(40),
        state: { state: 'paused' },
        ui_port: 18_101,
        published_ui_port: 9_451,
      },
    ],
    requested: [],
  };
  const list = await new PreviewsApi(async () => Response.json(body)).list('ren');
  const [preview] = list.previews;
  assert.ok(preview);
  assert.deepEqual(preview.of, { kind: 'unknown' });
  assert.deepEqual(preview.state, { state: 'unknown', text: 'paused' });
  assert.deepEqual(Previews.state(preview), ['paused', 'warning']);
});

test('an ask goes to the control API, and a refusal says why', async () => {
  const sent: string[] = [];
  const api = new PreviewsApi(async (input, init) => {
    sent.push(`${init?.method} ${String(input)} ${String(init?.body)}`);
    return String(input).endsWith('withdraw')
      ? Response.json({ api_version: 1, error: 'not_found' }, { status: 404 })
      : Response.json({ api_version: 1, commits: [] }, { status: 202 });
  });
  await api.ask('b'.repeat(40));
  await assert.rejects(api.withdraw('b'.repeat(40)));
  assert.deepEqual(sent, [
    `POST /control/previews/ask {"commit":"${'b'.repeat(40)}"}`,
    `POST /control/previews/withdraw {"commit":"${'b'.repeat(40)}"}`,
  ]);
});
