// Presentation only: the raw capture remains available independently.
export interface Card {
  label: string;
  text: string;
}
export interface View {
  cards: Card[];
  notices: string[];
}
type Obj = Record<string, any>;
const object = (v: unknown): v is Obj => !!v && typeof v === 'object' && !Array.isArray(v);
const list = (v: unknown): any[] => (Array.isArray(v) ? v : []);
const pretty = (v: unknown): string => {
  if (typeof v === 'string') return v;
  try {
    return JSON.stringify(v, null, 2) ?? '';
  } catch {
    return 'Value cannot be formatted; see raw capture.';
  }
};

/** Where one WebSocket message sits in the body, in bytes. */
export interface Boundary {
  offset: number;
  len: number;
}

/**
 * A readable view of a body preview. WebSocket bodies are messages back to back;
 * `messages` gives their byte boundaries, so each is read on its own: client
 * messages as documents, upstream messages as the event stream they are.
 */
export function bodyView(
  text: string,
  messages: Boundary[] = [],
  direction: 'request' | 'response' = 'response',
): View {
  try {
    return messages.length ? interpretMessages(text, messages, direction) : interpret(text);
  } catch {
    return {
      cards: [],
      notices: ['This payload could not be interpreted. Its original content is available in Raw.'],
    };
  }
}
function interpretMessages(
  text: string,
  messages: Boundary[],
  direction: 'request' | 'response',
): View {
  const bytes = new TextEncoder().encode(text);
  const decoder = new TextDecoder();
  const parts = [...messages]
    .sort((a, b) => a.offset - b.offset)
    .filter((m) => m.offset + m.len <= bytes.length)
    .map((m) => decoder.decode(bytes.subarray(m.offset, m.offset + m.len)));
  const beyond = messages.length - parts.length;
  let view: View;
  if (direction === 'request') {
    const views = parts.map(interpret);
    view = {
      cards: views.flatMap((v) => v.cards),
      notices: [...new Set(views.flatMap((v) => v.notices))],
    };
  } else {
    // Each upstream message is one Responses event: read them as an event stream.
    const stream = parts
      .map(
        (part) =>
          part
            .split(/\r\n?|\n/)
            .map((line) => `data: ${line}`)
            .join('\n') + '\n\n',
      )
      .join('');
    view = interpret(stream);
  }
  if (beyond)
    view.notices.push(
      `${beyond} message${beyond === 1 ? '' : 's'} beyond the preview; see Raw or export the body.`,
    );
  return view;
}

function interpret(text: string): View {
  const view: View = { cards: [], notices: [] };
  let remaining = 128 * 1024;
  const add = (label: string, value: unknown) => {
    if (view.cards.length >= 100 || remaining <= 0) {
      if (!view.notices.includes('Readable view limited; use Raw to inspect the captured preview.'))
        view.notices.push('Readable view limited; use Raw to inspect the captured preview.');
      return;
    }
    const full = pretty(value);
    const shown = full.slice(0, remaining);
    remaining -= shown.length;
    view.cards.push({ label: label.slice(0, 200), text: shown });
    if (shown.length < full.length) view.notices.push('Content shortened in the readable view.');
  };
  function block(v: any, role: string) {
    if (typeof v === 'string') {
      add(role, v);
      return;
    }
    if (!object(v)) {
      add(role, v);
      return;
    }
    if (['text', 'input_text', 'output_text'].includes(v.type)) add(role, v.text);
    else if (v.type === 'thinking') add(`${role} · thinking`, v.thinking);
    else if (v.type === 'tool_use' || v.type === 'function_call')
      add(
        `Tool call · ${v.name ?? 'unnamed'} · ${v.call_id ?? v.id ?? ''}`,
        v.input ?? v.arguments,
      );
    else if (v.type === 'tool_result' || v.type === 'function_call_output')
      add(
        `Tool result · ${v.tool_use_id ?? v.call_id ?? ''}${v.is_error ? ' · error' : ''}`,
        v.content ?? v.output,
      );
    else if (v.type === 'message' || v.role) message(v);
    else add(`${role} · ${v.type ?? 'content'}`, v);
  }
  function message(v: any) {
    if (!object(v)) {
      block(v, 'Message');
      return;
    }
    const role = typeof v.role === 'string' ? v.role : 'assistant';
    if (typeof v.content === 'string') add(role, v.content);
    else if (Array.isArray(v.content)) for (const part of v.content) block(part, role);
    else
      block(
        { ...v, role: undefined, type: v.type === 'message' ? 'unknown message' : v.type },
        role,
      );
  }
  function document(v: any) {
    if (!object(v)) {
      add('Payload', v);
      return;
    }
    if ('system' in v) add('System', v.system);
    if ('instructions' in v) add('Instructions', v.instructions);
    for (const m of list(v.messages)) message(m);
    if (typeof v.input === 'string') add('User', v.input);
    else for (const item of list(v.input)) block(item, 'Input');
    for (const item of list(v.output)) block(item, 'Assistant');
    if ('content' in v) message(v);
    if (v.error) add('Provider error', v.error);
    if (v.tools) add('Available tools', v.tools);
    const rest = Object.fromEntries(
      Object.entries(v).filter(
        ([k]) =>
          ![
            'system',
            'instructions',
            'messages',
            'input',
            'output',
            'content',
            'tools',
            'error',
          ].includes(k),
      ),
    );
    if (Object.keys(rest).length) add('Other fields', rest);
  }
  try {
    document(JSON.parse(text));
    return view;
  } catch {
    /* SSE or plain text */
  }
  const normalized = text.replace(/^\uFEFF/, '').replace(/\r\n?/g, '\n');
  if (!/^(?:data:|event:|:)/m.test(normalized)) {
    add('Text / incomplete JSON', text);
    return view;
  }
  const frames = normalized.split('\n\n');
  if (frames.at(-1)?.trim())
    view.notices.push('Trailing SSE event is incomplete and was not interpreted.');
  frames.pop();
  const blocks = new Map<number, Obj>();
  const items = new Map<number, Obj>();
  let terminal: Obj | undefined;
  let seen = 0;
  for (const frame of frames) {
    if (++seen > 2000) {
      view.notices.push('SSE event limit reached; use Raw for the remaining preview.');
      break;
    }
    const lines = frame.split('\n');
    const data = lines
      .filter((l) => l.startsWith('data:'))
      .map((l) => l.slice(5).replace(/^ /, ''))
      .join('\n');
    if (!data || data === '[DONE]') continue;
    let event: Obj;
    try {
      event = JSON.parse(data);
    } catch {
      view.notices.push('An SSE event could not be parsed; preserved in Raw.');
      continue;
    }
    if (!object(event)) {
      add('SSE data', event);
      continue;
    }
    if (
      ['index', 'output_index', 'content_index'].some(
        (k) => k in event && (!Number.isInteger(event[k]) || event[k] < 0 || event[k] >= 2000),
      )
    ) {
      view.notices.push('An invalid SSE content index was skipped; preserved in Raw.');
      continue;
    }
    const type =
      event.type ??
      lines
        .find((l) => l.startsWith('event:'))
        ?.slice(6)
        .trim();
    if (type === 'message_start')
      for (const [i, b] of list(event.message?.content).entries()) blocks.set(i, b);
    else if (type === 'content_block_start') blocks.set(event.index, { ...event.content_block });
    else if (type === 'content_block_delta') {
      const b = blocks.get(event.index) ?? { type: 'text', text: '' };
      const d = event.delta ?? {};
      if (typeof d.text === 'string') b.text = (b.text ?? '') + d.text;
      if (typeof d.thinking === 'string') b.thinking = (b.thinking ?? '') + d.thinking;
      if (typeof d.partial_json === 'string')
        b.partial_json = (b.partial_json ?? '') + d.partial_json;
      blocks.set(event.index, b);
    } else if (type === 'response.output_item.added' || type === 'response.output_item.done') {
      if (object(event.item)) items.set(event.output_index, event.item);
    } else if (type === 'response.output_text.delta') {
      const item = items.get(event.output_index) ?? {
        type: 'message',
        role: 'assistant',
        content: [],
      };
      if (!Array.isArray(item.content)) item.content = [];
      const i = event.content_index ?? 0;
      item.content[i] ??= { type: 'output_text', text: '' };
      item.content[i].text += event.delta ?? '';
      items.set(event.output_index, item);
    } else if (type === 'response.function_call_arguments.delta') {
      const item = items.get(event.output_index) ?? { type: 'function_call', arguments: '' };
      item.arguments = (item.arguments ?? '') + (event.delta ?? '');
      items.set(event.output_index, item);
    } else if (['response.completed', 'response.failed', 'response.incomplete'].includes(type))
      terminal = event.response;
    else if (
      ![
        'ping',
        'message_stop',
        'message_delta',
        'content_block_stop',
        'response.created',
        'response.in_progress',
        'response.output_text.done',
        'response.content_part.added',
        'response.content_part.done',
        'response.function_call_arguments.done',
      ].includes(type)
    )
      add(type ?? 'Unknown SSE event', event);
  }
  for (const b of blocks.values()) {
    if (typeof b.partial_json === 'string') {
      try {
        b.input = JSON.parse(b.partial_json);
      } catch {
        b.input = b.partial_json;
        view.notices.push('Tool arguments are incomplete JSON.');
      }
    }
    block(b, 'Assistant');
  }
  if (terminal) {
    document(terminal);
    if (!list(terminal.output).length) for (const item of items.values()) block(item, 'Assistant');
  } else for (const item of items.values()) block(item, 'Assistant');
  view.notices = [...new Set(view.notices)];
  if (!view.cards.length) add('SSE capture', text);
  return view;
}
