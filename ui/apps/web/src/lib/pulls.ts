// A project's pull requests on GitHub (docs/plans/self-build.md, phase 8),
// pure apart from `PullsApi`'s one read: what the daemon last saw of the
// repository the project is watched on (`/api/projects/{name}/pulls`), and
// how a pull request reads at a glance. GitHub's own text for a state Ren
// does not know is kept, never rounded to a known one.
import { ApiError, versioned } from './api.ts';
import {
  array,
  boolean,
  count,
  isRecord,
  label,
  nullable,
  object,
  ShapeError,
  string,
  tagged,
  tagValue,
  type Stored,
  unrecognised,
  type Variant,
} from './decode.ts';
import { WorkItemId } from './work.ts';
import type { BranchBody as WireBranch } from './wire/BranchBody.ts';
import type { CheckBody as WireCheck } from './wire/CheckBody.ts';
import type { PullBody as WirePull } from './wire/PullBody.ts';
import type { PullsBody as WirePulls } from './wire/PullsBody.ts';
import type { SightingBody as WireSighting } from './wire/SightingBody.ts';

export const readinesses = ['draft', 'ready'] as const;
export type Readiness = (typeof readinesses)[number];
/** Whether a pull request can merge now, as GitHub works it out. */
export const mergeStates = [
  'clean',
  'behind',
  'blocked',
  'conflicting',
  'draft',
  'unstable',
  'has_hooks',
  'computing',
] as const;
export type MergeState = (typeof mergeStates)[number];
export const reviewDecisions = [
  'approved',
  'changes_requested',
  'review_required',
  'not_required',
] as const;
export type ReviewDecision = (typeof reviewDecisions)[number];
/** A head's checks, summed up. */
export const checksStates = [
  'passing',
  'failing',
  'errored',
  'pending',
  'expected',
  'none',
] as const;
export type ChecksState = (typeof checksStates)[number];
/** One check's state. */
export const checkStates = [
  'queued',
  'running',
  'expected',
  'passed',
  'failed',
  'errored',
  'cancelled',
  'timed_out',
  'skipped',
  'neutral',
  'action_required',
  'stale',
  'startup_failed',
] as const;
export type CheckState = (typeof checkStates)[number];

export interface PullCheck {
  name: string;
  state: Stored<CheckState>;
}

export interface PullRequest {
  number: number;
  title: string;
  url: string;
  /** A GitHub login; null for a deleted account. */
  author: string | null;
  readiness: Stored<Readiness>;
  head_branch: string;
  head: string;
  base_branch: string;
  merge: Stored<MergeState>;
  review: Stored<ReviewDecision>;
  checks: Stored<ChecksState>;
  check_list: PullCheck[];
  /** The work items its description names. */
  work_items: WorkItemId[];
}

export interface Branch {
  name: string;
  head: string;
}

/** One answer from GitHub. */
export interface Sighting {
  at: number;
  default_branch: Branch | null;
  pull_requests: PullRequest[];
  /** More pull requests, or checks on one, are open than were listed. */
  truncated: boolean;
}

/** What was seen of a project's repository. */
export type Pulls =
  | { watch: 'off' }
  | { watch: 'waiting'; repository: string }
  | { watch: 'seen'; repository: string; seen: Sighting }
  /** The newest poll failed; `last` is the last answer, not current. */
  | { watch: 'failed'; repository: string; error: string; at: number; last: Sighting | null }
  /** A state a newer daemon reports. */
  | { watch: 'unknown'; text: string };

const sighting = object<Sighting, WireSighting>({
  at: count,
  default_branch: nullable(object<Branch, WireBranch>({ name: string, head: string })),
  pull_requests: array(
    object<PullRequest, WirePull>({
      number: count,
      title: string,
      url: string,
      author: nullable(string),
      readiness: label(readinesses),
      head_branch: string,
      head: string,
      base_branch: string,
      merge: label(mergeStates),
      review: label(reviewDecisions),
      checks: label(checksStates),
      check_list: array(object<PullCheck, WireCheck>({ name: string, state: label(checkStates) })),
      work_items: array(WorkItemId.decode),
    }),
  ),
  truncated: boolean,
});

const pulls = tagged<Pulls, WirePulls, 'watch'>(
  'watch',
  {
    off: object<Variant<WirePulls, 'watch', 'off'>>({ watch: tagValue('off') }),
    waiting: object<Variant<WirePulls, 'watch', 'waiting'>>({
      watch: tagValue('waiting'),
      repository: string,
    }),
    seen: object<Extract<Pulls, { watch: 'seen' }>, Variant<WirePulls, 'watch', 'seen'>>({
      watch: tagValue('seen'),
      repository: string,
      seen: sighting,
    }),
    failed: object<Extract<Pulls, { watch: 'failed' }>, Variant<WirePulls, 'watch', 'failed'>>({
      watch: tagValue('failed'),
      repository: string,
      error: string,
      at: count,
      last: nullable(sighting),
    }),
  },
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    return { watch: 'unknown', text: string(value.watch, `${path}.watch`) };
  },
);

/** The tone a state is drawn in. */
export type PullTone = 'ok' | 'warning' | 'bad' | 'quiet';

/** How pull requests read at a glance. */
export class PullRequests {
  /** The newest answer, current or not, and whether it is current. */
  static sighting(pulls: Pulls): { sighting: Sighting; current: boolean } | null {
    switch (pulls.watch) {
      case 'seen':
        return { sighting: pulls.seen, current: true };
      case 'failed':
        return pulls.last ? { sighting: pulls.last, current: false } : null;
      case 'off':
      case 'waiting':
      case 'unknown':
        return null;
      default: {
        const never: never = pulls;
        return never;
      }
    }
  }

  /** A head's checks, in a word or two, and their tone. */
  static checks(checks: Stored<ChecksState>): [string, PullTone] {
    switch (checks) {
      case 'passing':
        return ['checks pass', 'ok'];
      case 'failing':
        return ['checks fail', 'bad'];
      case 'errored':
        return ['a check errored', 'bad'];
      case 'pending':
        return ['checks running', 'warning'];
      case 'expected':
        return ['a check is expected', 'warning'];
      case 'none':
        return ['no checks', 'quiet'];
      default:
        return [`checks: ${unrecognised(checks)}`, 'warning'];
    }
  }

  /** Whether it can merge, in a few words, and the tone; null when nothing
   * stands in its way worth saying. */
  static merge(pull: PullRequest): [string, PullTone] | null {
    switch (pull.merge) {
      case 'clean':
      case 'has_hooks':
        return null;
      case 'unstable':
        return ['mergeable, a check fails', 'warning'];
      case 'behind':
        return [`behind ${pull.base_branch}`, 'warning'];
      case 'blocked':
        return ['blocked', 'warning'];
      case 'conflicting':
        return [`conflicts with ${pull.base_branch}`, 'bad'];
      case 'draft':
        return ['draft', 'quiet'];
      case 'computing':
        return ['merge state not worked out yet', 'quiet'];
      default:
        return [`merge: ${unrecognised(pull.merge)}`, 'warning'];
    }
  }

  /** What its reviews decided, in a few words; null where none is asked. */
  static review(review: Stored<ReviewDecision>): [string, PullTone] | null {
    switch (review) {
      case 'approved':
        return ['approved', 'ok'];
      case 'changes_requested':
        return ['changes requested', 'bad'];
      case 'review_required':
        return ['review required', 'warning'];
      case 'not_required':
        return null;
      default:
        return [`review: ${unrecognised(review)}`, 'warning'];
    }
  }

  /** A check's state, as a person reads it. */
  static checkState(state: Stored<CheckState>): [string, PullTone] {
    switch (state) {
      case 'passed':
        return ['passed', 'ok'];
      case 'failed':
      case 'errored':
      case 'timed_out':
      case 'startup_failed':
        return [state.replaceAll('_', ' '), 'bad'];
      case 'queued':
      case 'running':
      case 'expected':
      case 'action_required':
        return [state.replaceAll('_', ' '), 'warning'];
      case 'cancelled':
      case 'skipped':
      case 'neutral':
      case 'stale':
        return [state, 'quiet'];
      default:
        return [unrecognised(state), 'warning'];
    }
  }
}

/** Reads a project's pull requests. */
export class PullsApi {
  readonly #fetch: typeof fetch;

  constructor(fetcher: typeof fetch) {
    this.#fetch = fetcher;
  }

  async of(project: string, signal?: AbortSignal): Promise<Pulls> {
    const response = await this.#fetch(`/api/projects/${encodeURIComponent(project)}/pulls`, {
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
    return versioned(pulls)(body, '');
  }
}
