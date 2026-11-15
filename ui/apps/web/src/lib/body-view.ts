// Presentation only: the raw capture remains available independently.

/** What a card holds, which decides its color, icon, and whether it starts folded. */
export type CardKind =
  | 'user'
  | 'assistant'
  | 'thinking'
  | 'system'
  | 'tool_call'
  | 'tool_result'
  | 'tool_error'
  | 'tools'
  | 'error'
  | 'other';

export interface Card {
  kind: CardKind;
  label: string;
  /** A tool's name or a call's identifier, shown beside the label. */
  detail?: string;
  /** Prose is read as Markdown; data keeps its layout. */
  format: 'prose' | 'data';
  text: string;
}

const prose = new Set<CardKind>(['user', 'assistant', 'thinking', 'system']);

/**
 * A tool call's arguments as labeled fields, when they are one flat-enough
 * object: string values keep their text (a shell command reads as written),
 * others stay JSON. Anything else is shown as-is.
 */
export function toolFields(card: Card): [string, string][] | null {
  if (card.kind !== 'tool_call') return null;
  let value: unknown;
  try {
    value = JSON.parse(card.text);
  } catch {
    return null;
  }
  if (!object(value)) return null;
  const entries = Object.entries(value);
  if (!entries.length || entries.length > 24) return null;
  return entries.map(([key, field]) => [
    key,
    typeof field === 'string' ? field : (JSON.stringify(field, null, 2) ?? String(field)),
  ]);
}

/** The card kind for a message role; roles this view does not know stay `other`. */
export function roleKind(role: string): CardKind {
  switch (role.toLowerCase()) {
    case 'user':
    case 'input':
      return 'user';
    case 'assistant':
    case 'model':
      return 'assistant';
    case 'system':
    case 'developer':
    case 'instructions':
      return 'system';
    default:
      return 'other';
  }
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
  // The preview holds at most 256 KiB, so the readable view can hold it all.
  let remaining = 512 * 1024;
  const add = (label: string, value: unknown, kind: CardKind = 'other', detail?: string) => {
    if (view.cards.length >= 400 || remaining <= 0) {
      if (!view.notices.includes('Readable view limited; use Raw to inspect the captured preview.'))
        view.notices.push('Readable view limited; use Raw to inspect the captured preview.');
      return;
    }
    const full = pretty(value);
    const shown = full.slice(0, remaining);
    remaining -= shown.length;
    view.cards.push({
      kind,
      label: label.slice(0, 200),
      ...(detail ? { detail: detail.slice(0, 200) } : {}),
      format: prose.has(kind) && typeof value === 'string' ? 'prose' : 'data',
      text: shown,
    });
    if (shown.length < full.length) view.notices.push('Content shortened in the readable view.');
  };
  const label = (role: string) => role.charAt(0).toUpperCase() + role.slice(1);
  function block(v: any, role: string) {
    if (typeof v === 'string' || !object(v)) {
      add(label(role), v, roleKind(role));
      return;
    }
    if (['text', 'input_text', 'output_text'].includes(v.type))
      add(label(role), v.text, roleKind(role));
    else if (v.type === 'thinking') add('Thinking', v.thinking, 'thinking');
    else if (v.type === 'redacted_thinking') add('Thinking', '', 'thinking');
    else if (v.type === 'tool_use' || v.type === 'function_call')
      add('Tool call', toolInput(v.input ?? v.arguments), 'tool_call', v.name ?? 'unnamed');
    else if (v.type === 'tool_result' || v.type === 'function_call_output')
      add(
        v.is_error ? 'Tool error' : 'Tool result',
        toolOutput(v.content ?? v.output),
        v.is_error ? 'tool_error' : 'tool_result',
        v.tool_use_id ?? v.call_id,
      );
    else if (v.type === 'message' || v.role) message(v);
    else add(`${label(role)} · ${v.type ?? 'content'}`, v, 'other');
  }
  /** Arguments sent as a JSON string read like arguments sent as JSON. */
  function toolInput(input: unknown): unknown {
    if (typeof input !== 'string') return input;
    try {
      return JSON.parse(input);
    } catch {
      return input;
    }
  }
  /** Text parts of a tool result, joined; anything else stays structured. */
  function toolOutput(output: unknown): unknown {
    if (!Array.isArray(output)) return output;
    if (
      output.every((part) => object(part) && part.type === 'text' && typeof part.text === 'string')
    )
      return output.map((part) => part.text).join('\n');
    return output;
  }
  function message(v: any) {
    if (!object(v)) {
      block(v, 'Message');
      return;
    }
    const role = typeof v.role === 'string' ? v.role : 'assistant';
    if (typeof v.content === 'string') add(label(role), v.content, roleKind(role));
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
    if ('system' in v) add('System', systemText(v.system), 'system');
    if ('instructions' in v) add('Instructions', v.instructions, 'system');
    for (const m of list(v.messages)) message(m);
    if (typeof v.input === 'string') add('User', v.input, 'user');
    else for (const item of list(v.input)) block(item, 'Input');
    for (const item of list(v.output)) block(item, 'Assistant');
    if ('content' in v) message(v);
    if (v.error) add('Provider error', v.error, 'error');
    if (v.tools)
      add(
        'Available tools',
        list(v.tools).length ? toolNames(v.tools) : v.tools,
        'tools',
        `${list(v.tools).length}`,
      );
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
  /** Anthropic system prompts are a string or text blocks. */
  function systemText(system: unknown): unknown {
    if (Array.isArray(system) && system.every((b) => object(b) && typeof b.text === 'string'))
      return system.map((b) => b.text).join('\n\n');
    return system;
  }
  /** One line per tool: its name and the first line of its description. */
  function toolNames(tools: unknown[]): string {
    return tools
      .map((tool) => {
        if (!object(tool)) return String(tool);
        const name = tool.name ?? tool.function?.name ?? tool.type ?? 'unnamed';
        const description = String(tool.description ?? tool.function?.description ?? '')
          .split('\n')[0]
          .slice(0, 160);
        return description ? `${name} — ${description}` : String(name);
      })
      .join('\n');
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
      add('SSE data', event, 'other');
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
      add(type ?? 'Unknown SSE event', event, type === 'error' ? 'error' : 'other');
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
