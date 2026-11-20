// The factory's previews of Ren (docs/plans/self-build.md, phase 9), pure
// apart from `PreviewsApi`: each preview in its slot, of an open pull
// request or a commit the operator asked for (`/api/projects/{name}/previews`),
// where a browser reaches it, and the asks for one
// (`/control/previews/ask`, `/control/previews/withdraw`).
import { ApiError, versioned } from './api.ts';
import {
  array,
  count,
  isRecord,
  label,
  object,
  ShapeError,
  string,
  tagged,
  tagValue,
  type Stored,
  type Variant,
} from './decode.ts';
import { Refusals } from './refusal.ts';
import type { PreviewBody as WirePreview } from './wire/PreviewBody.ts';
import type { PreviewOfBody as WirePreviewOf } from './wire/PreviewOfBody.ts';
import type { PreviewStateBody as WirePreviewState } from './wire/PreviewStateBody.ts';
import type { PreviewsBody as WirePreviews } from './wire/PreviewsBody.ts';

/** The tone a state is drawn in. */
export type Tone = 'ok' | 'warning' | 'bad' | 'quiet';

/** Where a running preview is reached from. */
export const reaches = ['loopback', 'tailnet'] as const;
export type Reach = (typeof reaches)[number];

export type PreviewOf = { kind: 'pull'; number: number } | { kind: 'commit' } | { kind: 'unknown' };
export type PreviewState =
  | { state: 'building' }
  | { state: 'running'; reach: Stored<Reach> }
  | { state: 'failed'; detail: string }
  /** A state a newer daemon reports. */
  | { state: 'unknown'; text: string };

export interface Preview {
  slot: number;
  of: PreviewOf;
  commit: string;
  state: PreviewState;
  ui_port: number;
  published_ui_port: number;
}

export interface PreviewList {
  previews: Preview[];
  /** The commits asked for, the newest ask first. */
  requested: string[];
}

const previewOf = tagged<PreviewOf, WirePreviewOf, 'kind'>(
  'kind',
  {
    pull: object<Extract<PreviewOf, { kind: 'pull' }>, Variant<WirePreviewOf, 'kind', 'pull'>>({
      kind: tagValue('pull'),
      number: count,
    }),
    commit: object<Variant<WirePreviewOf, 'kind', 'commit'>>({ kind: tagValue('commit') }),
  },
  () => ({ kind: 'unknown' }),
);

const previewState = tagged<PreviewState, WirePreviewState, 'state'>(
  'state',
  {
    building: object<Variant<WirePreviewState, 'state', 'building'>>({
      state: tagValue('building'),
    }),
    running: object<
      Extract<PreviewState, { state: 'running' }>,
      Variant<WirePreviewState, 'state', 'running'>
    >({ state: tagValue('running'), reach: label(reaches) }),
    failed: object<Variant<WirePreviewState, 'state', 'failed'>>({
      state: tagValue('failed'),
      detail: string,
    }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { state: 'unknown', text: string(value.state, `${path}.state`) };
  },
);

const previewList = object<PreviewList, WirePreviews>({
  previews: array(
    object<Preview, WirePreview>({
      slot: count,
      of: previewOf,
      commit: string,
      state: previewState,
      ui_port: count,
      published_ui_port: count,
    }),
  ),
  requested: array(string),
});

/** How previews read, and where a browser reaches them. */
export class Previews {
  /** The preview of pull request `number`, if one holds a slot. */
  static ofPull(list: PreviewList, number: number): Preview | null {
    return (
      list.previews.find((preview) => preview.of.kind === 'pull' && preview.of.number === number) ??
      null
    );
  }

  /** The previews of commits asked for, and each ask still waiting for a
   * slot, the newest ask first. */
  static asked(list: PreviewList): { commit: string; preview: Preview | null }[] {
    return list.requested.map((commit) => ({
      commit,
      preview:
        list.previews.find(
          (preview) => preview.of.kind === 'commit' && preview.commit === commit,
        ) ?? null,
    }));
  }

  /** Where a browser at `page` opens the preview: on the tailnet port of
   * the host it reached this UI on, when the preview is published, or on
   * this host's loopback when this UI is itself on loopback. Null when it
   * cannot be reached from here. */
  static url(preview: Preview, page: Pick<URL, 'hostname'>): string | null {
    if (preview.state.state !== 'running') return null;
    const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(page.hostname);
    if (loopback) return `http://${page.hostname}:${preview.ui_port}`;
    return preview.state.reach === 'tailnet'
      ? `https://${page.hostname}:${preview.published_ui_port}`
      : null;
  }

  /** Where it stands, in a word or two, and the tone it is drawn in. */
  static state(preview: Preview): [string, Tone] {
    switch (preview.state.state) {
      case 'building':
        return ['building', 'warning'];
      case 'running':
        return preview.state.reach === 'tailnet' ? ['running', 'ok'] : ['host only', 'quiet'];
      case 'failed':
        return ['failed', 'bad'];
      case 'unknown':
        return [preview.state.text, 'warning'];
      default: {
        const never: never = preview.state;
        return never;
      }
    }
  }

  /** A pull request's preview, as a note on its row: `preview` when it
   * runs on the tailnet, else `preview <state>`. */
  static note(preview: Preview): [string, Tone] {
    const [said, tone] = Previews.state(preview);
    const tailnet = preview.state.state === 'running' && preview.state.reach === 'tailnet';
    return [tailnet ? 'preview' : `preview ${said}`, tone];
  }

  static short(commit: string): string {
    return commit.slice(0, 12);
  }
}

/** Reads a project's previews and asks for one. */
export class PreviewsApi {
  readonly #fetch: typeof fetch;

  constructor(fetcher: typeof fetch) {
    this.#fetch = fetcher;
  }

  async list(project: string, signal?: AbortSignal): Promise<PreviewList> {
    const response = await this.#fetch(`/api/projects/${encodeURIComponent(project)}/previews`, {
      signal,
      cache: 'no-store',
      headers: { Accept: 'application/json' },
    });
    if (!response.ok) throw new ApiError(response.status, null);
    let body: unknown;
    try {
      body = await response.json();
    } catch {
      throw new ShapeError('', 'JSON');
    }
    return versioned(previewList)(body, '');
  }

  /** Asks for a preview of `commit`, for the factory's next pass. */
  ask(commit: string): Promise<void> {
    return this.#change('ask', commit);
  }

  /** Withdraws the ask; the factory's next pass stops its preview. */
  withdraw(commit: string): Promise<void> {
    return this.#change('withdraw', commit);
  }

  async #change(action: 'ask' | 'withdraw', commit: string): Promise<void> {
    const response = await this.#fetch(`/control/previews/${action}`, {
      method: 'POST',
      cache: 'no-store',
      headers: { accept: 'application/json', 'content-type': 'application/json' },
      body: JSON.stringify({ commit }),
    });
    if (response.ok) return;
    let parsed: unknown = null;
    try {
      parsed = await response.json();
    } catch {
      parsed = null;
    }
    const code = Refusals.read(parsed);
    throw new Error(code ? Refusals.say(code) : `The daemon answered ${response.status}.`);
  }
}
