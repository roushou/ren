/**
 * Work items in the UI: what an operator marked to be done, read from
 * `/api/work`, queued and moved on through `/control/work`, and the brief
 * an agent gets for one or several. A Redline send records one; the Work
 * view lists them and sends queued ones together. Pure but for `WorkApi`,
 * which reads and writes through a capability-supplied `fetch`.
 */
import { API_VERSION, revision, type Revision, type Shelf } from './api.ts';
import {
  Uuid,
  array,
  count,
  isRecord,
  label,
  nullable,
  object,
  string,
  unrecognised,
  ShapeError,
  type AnyDecoder,
  type Brand,
  type Stored,
} from './decode.ts';
import { Redline, type Mark, type PageContext } from 'redline';
import { Briefing, type RenPage, type WorkPlace } from './briefing.ts';
import { Refusals, type Refusal } from './refusal.ts';
import { SessionId } from './runs.ts';
import type { EvidenceBody as WireEvidence } from './wire/EvidenceBody.ts';
import type { MarkedPageBody as WireMarkedPage } from './wire/MarkedPageBody.ts';
import type { WorkAnswer as WireWorkAnswer } from './wire/WorkAnswer.ts';
import type { WorkItemView as WireWorkItem } from './wire/WorkItemView.ts';
import type { WorkItems as WireWorkItems } from './wire/WorkItems.ts';
import type { WorkMarkBody as WireWorkMark } from './wire/WorkMarkBody.ts';
import type { WorkOrderBody as WireWorkOrder } from './wire/WorkOrderBody.ts';

export type WorkItemId = Brand<string, 'WorkItemId'>;
export const WorkItemId = new Uuid<'WorkItemId'>();

/** Where a work item stands. */
export const workStates = ['queued', 'sent', 'landed', 'shipped', 'dismissed'] as const;
export type WorkState = (typeof workStates)[number];

/** The page a work item was marked on. */
export interface MarkedPage {
  path: string;
  route: string | null;
  title: string;
  width: number;
  height: number;
  theme: Stored<'light' | 'dark'>;
  revision: Revision | null;
}

/** What else was on screen: the errors the page logged, the reads behind it. */
export interface Evidence {
  console: string[];
  reads: string[];
}

/** What was marked. */
export interface WorkOrder {
  page: MarkedPage;
  marks: WireWorkMark[];
  evidence: Evidence;
}

/** One work item, as the daemon keeps it. */
export interface WorkItem {
  id: WorkItemId;
  created_at: number;
  updated_at: number;
  title: string;
  order: WorkOrder;
  state: Stored<WorkState>;
  session: SessionId | null;
  landed: string | null;
  shipped: Revision | null;
}

/** A change to a work item. */
export type WorkChange =
  | { kind: 'send'; session: SessionId }
  | { kind: 'land'; commit: string }
  | { kind: 'ship'; revision: string }
  | { kind: 'dismiss' };

/** A work request the daemon refused. */
export class WorkError extends Error {
  readonly status: number;
  readonly code: Stored<Refusal> | null;
  constructor(status: number, code: Stored<Refusal> | null, message: string | null) {
    super(message ?? (code ? Refusals.say(code) : `The daemon answered ${status}.`));
    this.status = status;
    this.code = code;
  }
}

const themes = ['light', 'dark'] as const;
const workMark = object<WireWorkMark>({
  selector: string,
  text: string,
  source: nullable(string),
  width: count,
  height: count,
  note: string,
});
const workItem = object<WorkItem, WireWorkItem>({
  id: WorkItemId.decode,
  created_at: count,
  updated_at: count,
  title: string,
  order: object<WorkOrder, WireWorkOrder>({
    page: object<MarkedPage, WireMarkedPage>({
      path: string,
      route: nullable(string),
      title: string,
      width: count,
      height: count,
      theme: label(themes),
      revision: nullable(revision),
    }),
    marks: array(workMark),
    evidence: object<Evidence, WireEvidence>({ console: array(string), reads: array(string) }),
  }),
  state: label(workStates),
  session: nullable(SessionId.decode),
  landed: nullable(string),
  shipped: nullable(revision),
});
const workItems = object<{ work: WorkItem[] }, WireWorkItems>({ work: array(workItem) });
const workAnswer = object<{ id: WorkItemId; state: Stored<WorkState> }, WireWorkAnswer>({
  id: WorkItemId.decode,
  state: label(workStates),
});

/** Reads and writes work items through a capability-supplied `fetch`. */
export class WorkApi {
  readonly #fetch: typeof fetch;

  constructor(fetcher: typeof fetch) {
    this.#fetch = fetcher;
  }

  /** Work items on `shelf`, most recently changed first. */
  async list(shelf: Shelf = 'current', signal?: AbortSignal): Promise<WorkItem[]> {
    const path = shelf === 'archived' ? '/api/work?shelf=archived' : '/api/work';
    return (await this.#request('GET', path, undefined, workItems, signal)).work;
  }

  /** Queue `order`, and return the new item's id. */
  async queue(order: WireWorkOrder): Promise<WorkItemId> {
    return (await this.#request('POST', '/control/work', order, workAnswer)).id;
  }

  /** Move `item` on. */
  async move(item: WorkItemId, change: WorkChange): Promise<Stored<WorkState>> {
    const body = (() => {
      switch (change.kind) {
        case 'send':
          return { session: change.session };
        case 'land':
          return { commit: change.commit };
        case 'ship':
          return { revision: change.revision };
        case 'dismiss':
          return undefined;
        default: {
          const never: never = change;
          return never;
        }
      }
    })();
    return (await this.#request('POST', `/control/work/${item}/${change.kind}`, body, workAnswer))
      .state;
  }

  async #request<T>(
    method: 'GET' | 'POST',
    path: string,
    body: unknown,
    decode: AnyDecoder<T>,
    signal?: AbortSignal,
  ): Promise<T> {
    const response = await this.#fetch(path, {
      method,
      cache: 'no-store',
      headers:
        body === undefined
          ? { accept: 'application/json' }
          : { accept: 'application/json', 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal,
    });
    let parsed: unknown = null;
    try {
      parsed = await response.json();
    } catch {
      parsed = null;
    }
    if (!response.ok) {
      const message =
        isRecord(parsed) && typeof parsed.message === 'string' ? parsed.message : null;
      throw new WorkError(response.status, Refusals.read(parsed), message);
    }
    if (!isRecord(parsed) || parsed.api_version !== API_VERSION) {
      throw new ShapeError('api_version', `${API_VERSION}`);
    }
    return decode(parsed, '');
  }
}

/** The errors and reads a page made, kept for the work item marked on it. */
export class PageEvidence {
  /** The most of each kept, newest last. */
  static readonly KEEP = 50;
  #console: string[] = [];
  #reads: string[] = [];

  /** An error the page logged. */
  error(text: string) {
    PageEvidence.#push(this.#console, text.slice(0, 2000));
  }

  /** A read the page made (a path and query), once each. */
  read(path: string) {
    if (!this.#reads.includes(path)) PageEvidence.#push(this.#reads, path);
  }

  /** A new page: what the last one read and logged no longer applies. */
  reset() {
    this.#console = [];
    this.#reads = [];
  }

  snapshot(): Evidence {
    return { console: [...this.#console], reads: [...this.#reads] };
  }

  static #push(list: string[], entry: string) {
    list.push(entry);
    if (list.length > PageEvidence.KEEP) list.splice(0, list.length - PageEvidence.KEEP);
  }

  /** Whether `path` is a read of the daemon's API worth keeping. */
  static readOf(url: string, origin: string): string | null {
    let parsed: URL;
    try {
      parsed = new URL(url, origin);
    } catch {
      return null;
    }
    if (parsed.origin !== origin) return null;
    return parsed.pathname.startsWith('/api/') && parsed.pathname !== '/api/live'
      ? `${parsed.pathname}${parsed.search}`
      : null;
  }
}

/** A revision as the daemon writes it. */
const revisionText = (value: Revision | null): string | null => {
  if (!value) return null;
  switch (value.kind) {
    case 'commit':
      return value.tree === 'modified' ? `${value.commit}-dirty` : value.commit;
    case 'unknown':
      return 'unknown';
    case 'unrecognized':
      return value.text;
    default: {
      const never: never = value;
      return never;
    }
  }
};

export class Work {
  /** The order a Redline pane sends: the page, its noted marks, and what
   * else was on screen. */
  static order(page: PageContext, ren: RenPage, marks: readonly Mark[], evidence: Evidence): WireWorkOrder {
    return {
      page: {
        path: page.path,
        route: ren.route,
        title: page.title,
        width: Math.round(page.width),
        height: Math.round(page.height),
        theme: page.theme,
        revision: revisionText(ren.revision),
      },
      marks: Redline.noted(marks).map((mark) => ({
        selector: mark.target.selector,
        text: mark.target.text,
        source: mark.target.source,
        width: Math.round(mark.target.width),
        height: Math.round(mark.target.height),
        note: mark.note.trim(),
      })),
      evidence,
    };
  }

  /** What the brief says of what else was on screen. */
  static evidence(evidence: Evidence): string[] {
    const lines: string[] = [];
    if (evidence.console.length) {
      // Inline, not a list: the brief's own list follows its page lines.
      const last = evidence.console
        .slice(-3)
        .map((entry) => Redline.code(entry.split('\n')[0] ?? entry))
        .join('; ');
      lines.push(
        evidence.console.length === 1
          ? `The page logged an error: ${last}.`
          : `The page logged ${evidence.console.length} errors, the last: ${last}.`,
      );
    }
    if (evidence.reads.length) {
      lines.push(
        `It showed what these reads of the daemon's API answered: ${evidence.reads
          .slice(-8)
          .map((path) => Redline.code(path))
          .join(', ')}.`,
      );
    }
    return lines;
  }

  /** One item's brief, as Redline would write it for its page, with its id
   * (none before it is queued, as the pane previews it) and what else was
   * on screen, and how the change is to be made. */
  static brief(id: WorkItemId | null, order: WireWorkOrder, place: WorkPlace): string {
    const { page, ren, marks } = Work.#parts(order);
    const framing = Briefing.framing(ren, marks, place);
    return Redline.brief(page, marks, {
      about: [...framing.about, ...(id ? [`This is work item ${Redline.code(id)}.`] : []), ...Work.evidence(order.evidence)],
      closing: Work.closing(framing.closing, id ? [id] : []),
    });
  }

  /** The closing paragraph, with how the change names the items it does,
   * so Ren knows when it lands on main: a `Work-Item:` trailer per item,
   * in each commit and in the pull request's description, which a squash
   * merge keeps. Before an item is queued, its id is not known yet. */
  static closing(closing: string | null, ids: readonly WorkItemId[]): string | null {
    if (!closing) return null;
    const lines = ids.length
      ? ids.map((id) => Redline.code(`Work-Item: ${id}`)).join(' and ')
      : `the work item's ${Redline.code('Work-Item:')} line`;
    return `${closing} End each commit message with ${lines}, and put the same in the pull request's description: that is how Ren knows when the change lands.`;
  }

  /** One brief for several queued items: each item's marks under its page,
   * and how the change is to be made, once. */
  static batch(items: readonly WorkItem[], place: WorkPlace): string {
    if (items.length === 1 && items[0]) {
      const [item] = items;
      return Work.brief(item.id, Work.wire(item.order), place);
    }
    const sections = items.map((item, index) => {
      const { page, ren, marks } = Work.#parts(Work.wire(item.order));
      const framing = Briefing.framing(ren, marks, place);
      const body = Redline.brief(page, marks, {
        about: [...framing.about, ...Work.evidence(item.order.evidence)],
        closing: null,
      })
        .split('\n')
        .slice(2)
        .join('\n');
      return [`## ${index + 1}. ${item.title} (work item ${Redline.code(item.id)})`, '', body].join('\n');
    });
    const all = items.flatMap((item) => Work.#parts(Work.wire(item.order)).marks);
    const first = items[0];
    const closing = first
      ? Work.closing(
          Briefing.framing(Work.#parts(Work.wire(first.order)).ren, all, place).closing,
          items.map((item) => item.id),
        )
      : null;
    return [
      `Redline: ${items.length} work items`,
      '',
      ...sections.flatMap((section) => [section, '']),
      ...(closing ? [closing] : []),
    ].join('\n');
  }

  /** An item's order as the daemon takes one. */
  static wire(order: WorkOrder): WireWorkOrder {
    const { page } = order;
    return {
      page: {
        path: page.path,
        route: page.route,
        title: page.title,
        width: page.width,
        height: page.height,
        theme: page.theme,
        revision: revisionText(page.revision),
      },
      marks: order.marks,
      evidence: order.evidence,
    };
  }

  static #parts(order: WireWorkOrder): { page: PageContext; ren: RenPage; marks: Mark[] } {
    const { page } = order;
    return {
      page: {
        path: page.path,
        title: page.title,
        width: page.width,
        height: page.height,
        theme: page.theme === 'dark' ? 'dark' : 'light',
      },
      ren: {
        route: page.route,
        revision: page.revision === null ? null : revision(page.revision, 'revision'),
      },
      marks: order.marks.map((mark, index) => ({
        id: index + 1,
        target: {
          selector: mark.selector,
          text: mark.text,
          source: mark.source,
          width: mark.width,
          height: mark.height,
        },
        note: mark.note,
      })),
    };
  }

  /** What the Work view says of a state. */
  static state(state: Stored<WorkState>): string {
    switch (state) {
      case 'queued':
        return 'Queued';
      case 'sent':
        return 'In a session';
      case 'landed':
        return 'Landed on main';
      case 'shipped':
        return 'Shipped';
      case 'dismissed':
        return 'Dismissed';
      default:
        return unrecognised(state);
    }
  }
}
