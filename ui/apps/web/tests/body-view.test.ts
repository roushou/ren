import { test } from 'node:test';
import assert from 'node:assert/strict';
import { bodyView, toolFields } from '../src/lib/body-view.ts';
const sse = (...events: unknown[]) => events.map((e) => `data: ${JSON.stringify(e)}\n\n`).join('');
test('native requests show instructions, messages, tool inputs/results, and other fields as text', () => {
  const result = bodyView(
    JSON.stringify({
      system: '<script>alert(1)</script>',
      messages: [
        { role: 'user', content: 'hello' },
        {
          role: 'assistant',
          content: [{ type: 'tool_use', id: 't1', name: 'read', input: { path: 'a.txt' } }],
        },
        {
          role: 'user',
          content: [{ type: 'tool_result', tool_use_id: 't1', content: 'file text' }],
        },
      ],
      future_option: { enabled: true },
    }),
  );
  assert.equal(result.cards[0].text, '<script>alert(1)</script>');
  assert.ok(result.cards.some((c) => c.label.includes('Tool call') && c.text.includes('a.txt')));
  assert.ok(
    result.cards.some((c) => c.label.includes('Tool result') && c.text.includes('file text')),
  );
  assert.ok(result.cards.some((c) => c.text.includes('future_option')));
  const openai = bodyView(
    JSON.stringify({
      instructions: 'be concise',
      input: [
        { type: 'function_call', call_id: 't', name: 'read', arguments: '{"path":"a"}' },
        { type: 'function_call_output', call_id: 't', output: 'done' },
      ],
    }),
  );
  assert.deepEqual(
    openai.cards.map((c) => c.label.split(' · ')[0]),
    ['Instructions', 'Tool call', 'Tool result'],
  );
});
test('Anthropic SSE joins tool argument fragments without hiding unknown events', () => {
  const result = bodyView(
    sse(
      {
        type: 'content_block_start',
        index: 0,
        content_block: { type: 'tool_use', id: 'a', name: 'read', input: {} },
      },
      { type: 'content_block_delta', index: 0, delta: { partial_json: '{"path":' } },
      { type: 'content_block_delta', index: 0, delta: { partial_json: '"a.txt"}' } },
      { type: 'future_event', value: 42 },
    ),
  );
  assert.ok(
    result.cards.some(
      (c) =>
        c.kind === 'tool_call' &&
        c.detail === 'read' &&
        c.format === 'data' &&
        c.text.includes('a.txt'),
    ),
  );
  assert.ok(result.cards.some((c) => c.label === 'future_event'));
});
test('Responses terminal output replaces deltas; partial streams retain their observed content', () => {
  const delta = sse(
    {
      type: 'response.output_item.added',
      output_index: 0,
      item: { type: 'message', role: 'assistant', content: [] },
    },
    { type: 'response.output_text.delta', output_index: 0, content_index: 0, delta: 'hello' },
  );
  assert.equal(bodyView(delta).cards[0].text, 'hello');
  const result = bodyView(
    delta +
      sse({
        type: 'response.completed',
        response: {
          output: [
            {
              type: 'message',
              role: 'assistant',
              content: [{ type: 'output_text', text: 'hello world' }],
            },
          ],
        },
      }),
  );
  assert.deepEqual(
    result.cards.map((c) => c.text),
    ['hello world'],
  );
  assert.ok(bodyView(delta + 'data: {"type":').notices.some((n) => n.includes('incomplete')));
});
test('malformed data and hostile indices cannot crash or allocate huge readable arrays', () => {
  assert.ok(
    bodyView(
      sse({ type: 'response.output_text.delta', output_index: 0, content_index: 1e9, delta: 'x' }),
    ).notices.length,
  );
  assert.doesNotThrow(() =>
    bodyView(
      sse(
        { type: 'response.output_item.added', item: null },
        { type: 'response.output_text.delta', delta: {} },
      ),
    ),
  );
  const bounded = bodyView(
    JSON.stringify({
      messages: Array.from({ length: 500 }, () => ({ role: 'user', content: 'hello' })),
    }),
  );
  assert.equal(bounded.cards.length, 400);
  assert.ok(bounded.notices.length);
});

test('websocket bodies are read message by message using their byte boundaries', () => {
  const events = [
    JSON.stringify({ type: 'response.created', response: { id: 'resp_1' } }),
    JSON.stringify({ type: 'response.output_text.delta', delta: 'héllo' }),
    JSON.stringify({
      type: 'response.completed',
      response: { id: 'resp_1', status: 'completed' },
    }),
  ];
  const encoder = new TextEncoder();
  let offset = 0;
  const marks = events.map((event) => {
    const len = encoder.encode(event).length;
    const mark = { offset, len };
    offset += len;
    return mark;
  });
  // Deltas assemble across messages, as they do across SSE events.
  const response = bodyView(events.join(''), marks, 'response');
  assert.ok(
    response.cards.some(
      (card) => card.kind === 'assistant' && card.format === 'prose' && card.text === 'héllo',
    ),
  );
  assert.ok(!response.cards.some((card) => card.label.includes('incomplete JSON')));

  const create = JSON.stringify({
    type: 'response.create',
    instructions: 'Be brief.',
    input: 'hi',
  });
  const request = bodyView(create, [{ offset: 0, len: encoder.encode(create).length }], 'request');
  assert.deepEqual(
    request.cards.map((card) => [card.label, card.kind]),
    [
      ['Instructions', 'system'],
      ['User', 'user'],
      ['Other fields', 'other'],
    ],
  );

  // A message past the end of the preview is reported, not invented.
  const cut = bodyView(events[0], marks, 'response');
  assert.ok(cut.notices.some((notice) => notice.includes('2 messages beyond the preview')));
});

test('cards say what they hold: roles, thinking, tools, and errors', () => {
  const view = bodyView(
    JSON.stringify({
      system: [
        { type: 'text', text: 'You are terse.' },
        { type: 'text', text: 'Use tools.' },
      ],
      tools: [{ name: 'Bash', description: 'Run a command\nLong details' }, { name: 'Read' }],
      messages: [
        { role: 'user', content: 'list files' },
        {
          role: 'assistant',
          content: [
            { type: 'thinking', thinking: '' },
            { type: 'tool_use', id: 'toolu_1', name: 'Bash', input: { command: 'ls' } },
          ],
        },
        {
          role: 'user',
          content: [
            {
              type: 'tool_result',
              tool_use_id: 'toolu_1',
              is_error: true,
              content: [{ type: 'text', text: 'denied' }],
            },
          ],
        },
      ],
    }),
  );
  assert.deepEqual(
    view.cards.map((c) => [c.kind, c.label, c.detail ?? null, c.format]),
    [
      ['system', 'System', null, 'prose'],
      ['user', 'User', null, 'prose'],
      ['thinking', 'Thinking', null, 'prose'],
      ['tool_call', 'Tool call', 'Bash', 'data'],
      ['tool_error', 'Tool error', 'toolu_1', 'data'],
      ['tools', 'Available tools', '2', 'data'],
    ],
  );
  assert.equal(view.cards[0].text, 'You are terse.\n\nUse tools.');
  assert.equal(view.cards[4].text, 'denied');
  assert.equal(view.cards[5].text, 'Bash — Run a command\nRead');
  // A role this view does not know is kept, not mislabeled.
  const other = bodyView(JSON.stringify({ messages: [{ role: 'critic', content: 'hm' }] }));
  assert.deepEqual(
    other.cards.map((c) => [c.kind, c.label]),
    [['other', 'Critic']],
  );
});

test('tool call arguments read as labeled fields when they are an object', () => {
  const view = bodyView(
    JSON.stringify({
      content: [
        {
          type: 'tool_use',
          id: 'toolu_1',
          name: 'Bash',
          input: { command: 'ls -la\ncat x', description: 'List', timeout: 5 },
        },
        { type: 'tool_use', id: 'toolu_2', name: 'Echo', input: 'plain text' },
      ],
      role: 'assistant',
    }),
  );
  const [bash, echo] = view.cards.filter((c) => c.kind === 'tool_call');
  assert.deepEqual(toolFields(bash), [
    ['command', 'ls -la\ncat x'],
    ['description', 'List'],
    ['timeout', '5'],
  ]);
  assert.equal(toolFields(echo), null);
});
