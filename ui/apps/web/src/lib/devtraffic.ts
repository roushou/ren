/**
 * The dev toolbar's view of the dev slot's fake traffic: its settings, as the
 * generator reports them, parsed at the boundary, and the requests that
 * change them. Only a development gateway with the slot's traffic controls
 * shows the toolbar.
 */
import { ShapeError, count, isRecord, label, object, type Decoder, type Stored } from './decode.ts';

const flows = ['running', 'paused'] as const;
const paces = ['instant', 'normal', 'slow'] as const;
export type Flow = (typeof flows)[number];
export type Pace = (typeof paces)[number];
export const PACES: readonly Pace[] = paces;

/** One-off calls the toolbar can send, by the reply they ask for. */
export const SINGLE_REPLIES = [
  { reply: 'slow', label: 'Slow call' },
  { reply: 'tools', label: 'Tool call' },
  { reply: 'error', label: 'Error' },
  { reply: 'rate_limited', label: 'Rate limit' },
] as const;

/** Weights by name, each 0 to 100, in the generator's order. */
export type Weights = Record<string, number>;

export interface TrafficSettings {
  flow: Stored<Flow>;
  /** Calls a minute. */
  rate: number;
  pace: Stored<Pace>;
  replies: Weights;
  harnesses: Weights;
  senders: Weights;
}

export interface TrafficStatus {
  settings: TrafficSettings;
  sent: number;
  in_flight: number;
}

const weights: Decoder<Weights> = (value, path) => {
  if (!isRecord(value)) throw new ShapeError(path, 'an object');
  const out: Weights = {};
  for (const [name, weight] of Object.entries(value)) {
    const n = count(weight, `${path}.${name}`);
    if (n > 100) throw new ShapeError(`${path}.${name}`, 'a weight from 0 to 100');
    out[name] = n;
  }
  return out;
};

const settings = object<TrafficSettings>({
  flow: label(flows),
  rate: count,
  pace: label(paces),
  replies: weights,
  harnesses: weights,
  senders: weights,
});

export const trafficStatus = object<TrafficStatus>({ settings, sent: count, in_flight: count });

/** The toolbar's requests, through the UI server's `/dev/traffic` route. */
export class DevTraffic {
  static readonly MAX_RATE = 600;

  private readonly fetcher: typeof fetch;

  /** The default calls the global `fetch` as a function: a browser refuses it as a method. */
  constructor(fetcher: typeof fetch = (input, init) => fetch(input, init)) {
    this.fetcher = fetcher;
  }

  async status(): Promise<TrafficStatus> {
    const response = await this.fetcher('/dev/traffic/status');
    if (!response.ok) throw new Error('The dev traffic generator is not answering.');
    return trafficStatus(await response.json(), 'status');
  }

  async save(next: TrafficSettings): Promise<void> {
    await this.post('/dev/traffic/settings', next);
  }

  async burst(count: number, reply?: string): Promise<void> {
    await this.post('/dev/traffic/burst', reply ? { count, reply } : { count });
  }

  private async post(path: string, body: unknown): Promise<void> {
    const response = await this.fetcher(path, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!response.ok) throw new Error((await response.text()) || `HTTP ${response.status}`);
  }

  /** The label a weight's name reads as: `claude_code` is "claude code". */
  static label(name: string): string {
    return name.replaceAll('_', ' ');
  }
}
