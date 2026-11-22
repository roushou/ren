// Where the release stands, as one status-line entry: how far production
// is behind main, the canary's verdict, and what promotion decided. Pure,
// read from `/api/release` (`ReleaseView`); the factory writes it.
import type { CheckView, Preview, Pull, Pulls, Revision } from './api.ts';
import type { ReleaseView } from './api.ts';
import { assertNever, unrecognised } from './decode.ts';
import type { Mark } from './marks.ts';
import { Refusals } from './refusal.ts';
import type { WorkItem } from './work.ts';

/** The tone the entry is drawn in. */
export type ReleaseTone = 'ok' | 'warning' | 'bad';

/** The status line's release entry: short text, its tone, and the detail on hover. */
export class ReleaseLine {
  readonly text: string;
  readonly tone: ReleaseTone;
  readonly detail: string;

  constructor(text: string, tone: ReleaseTone, detail: string) {
    this.text = text;
    this.tone = tone;
    this.detail = detail;
  }

  /** A commit as people read it: its first 12 digits. */
  static #commit(id: string): string {
    return id.slice(0, 12);
  }

  static #revision(revision: Revision): string {
    switch (revision.kind) {
      case 'commit':
        return revision.tree === 'clean'
          ? ReleaseLine.#commit(revision.commit)
          : `${ReleaseLine.#commit(revision.commit)}, modified`;
      case 'unknown':
        return 'unknown';
      case 'unrecognized':
        return revision.text;
      default: {
        const never: never = revision;
        return never;
      }
    }
  }

  static of(release: ReleaseView): ReleaseLine {
    const behind =
      release.behind === null
        ? 'Production not counted against main'
        : release.behind === 0
          ? 'Production on main'
          : `Production ${release.behind} behind main`;
    const canary = release.canary;
    const verdict = canary ? `canary ${canary.passed ? 'passed' : 'failed'}` : 'no canary';
    const [decision, tone] = ReleaseLine.decision(release);
    const checks = canary
      ? canary.checks
          .map((check) =>
            check.passed
              ? `${check.name ?? check.check} passed`
              : `${check.name ?? check.check} failed${check.detail ? `: ${check.detail}` : ''}`,
          )
          .join('\n  ')
      : '';
    const detail = [
      `main ${ReleaseLine.#commit(release.main)}`,
      `production ${ReleaseLine.#revision(release.production)}`,
      canary ? `canary ${ReleaseLine.#commit(canary.commit)}\n  ${checks}` : 'canary: none checked',
      `promotion (${release.policy}): ${decision}`,
    ].join('\n');
    return new ReleaseLine(`${behind} · ${verdict}`, tone, detail);
  }

  /** What promotion decided, as a sentence, and the tone it is drawn in. */
  static decision(release: ReleaseView): [string, ReleaseTone] {
    const promotion = Promotion.of(release.decision);
    return [promotion.sentence, promotion.tone];
  }
}

/** What promotion decided, read two ways: a word that names where the
 * release stands at a glance, and the sentence that explains it. */
export class Promotion {
  readonly word: string;
  readonly sentence: string;
  readonly tone: ReleaseTone;
  /** Its glyph: waiting on the operator is theirs to act on. */
  readonly mark: Mark;

  constructor(word: string, sentence: string, tone: ReleaseTone, mark: Mark) {
    this.word = word;
    this.sentence = sentence;
    this.tone = tone;
    this.mark = mark;
  }

  static of(decision: ReleaseView['decision']): Promotion {
    switch (decision) {
      case 'promote':
        return new Promotion(
          'Promoting',
          "production takes the canary's build now",
          'ok',
          'running',
        );
      case 'current':
        return new Promotion('Live', "production runs the canary's build", 'ok', 'passed');
      case 'failed':
        return new Promotion(
          'Blocked',
          "the canary's build failed; production keeps its own",
          'bad',
          'failed',
        );
      case 'unchecked':
        return new Promotion('Unchecked', 'no build has been checked', 'warning', 'unknown');
      case 'production_ahead':
        return new Promotion(
          'Diverged',
          "production has commits the canary's build lacks",
          'warning',
          'warning',
        );
      case 'unconfirmed':
        return new Promotion(
          'Unknown build',
          'production runs a build the factory cannot place',
          'warning',
          'unknown',
        );
      case 'awaiting_operator':
        return new Promotion(
          'Waiting on you',
          'passed; waiting for the operator to promote it',
          'warning',
          'yours',
        );
      case 'outside_window':
        return new Promotion(
          'Scheduled',
          'passed; production takes it when the window opens',
          'ok',
          'queued',
        );
      case 'promotion_failed':
        return new Promotion(
          'Rolled back',
          'production failed to take it and was rolled back',
          'bad',
          'failed',
        );
      default:
        return new Promotion(unrecognised(decision), unrecognised(decision), 'warning', 'unknown');
    }
  }
}

/** The canary's verdict on its build, or that none has been checked. */
export type CanaryMark = 'passed' | 'failed' | 'none';

/** Where production stands against main. */
export type ProductionMark =
  | { kind: 'on_main' }
  | { kind: 'behind'; commits: number }
  /** Production's revision is not a commit of main's history. */
  | { kind: 'uncounted' };

/** A release drawn as its pipeline, main → canary → production, with the
 * word for what promotion decided; the sentences stay on hover. */
export class ReleaseStrip {
  readonly canary: CanaryMark;
  readonly production: ProductionMark;
  readonly promotion: Promotion;
  /** The same release as words, for assistive technology and hover. */
  readonly line: ReleaseLine;

  constructor(
    canary: CanaryMark,
    production: ProductionMark,
    promotion: Promotion,
    line: ReleaseLine,
  ) {
    this.canary = canary;
    this.production = production;
    this.promotion = promotion;
    this.line = line;
  }

  static of(release: ReleaseView): ReleaseStrip {
    const canary: CanaryMark = release.canary
      ? release.canary.passed
        ? 'passed'
        : 'failed'
      : 'none';
    const production: ProductionMark =
      release.behind === null
        ? { kind: 'uncounted' }
        : release.behind === 0
          ? { kind: 'on_main' }
          : { kind: 'behind', commits: release.behind };
    return new ReleaseStrip(
      canary,
      production,
      Promotion.of(release.decision),
      ReleaseLine.of(release),
    );
  }

  /** The strip as one phrase: the status line's text and the decision. */
  get label(): string {
    return `${this.line.text} · ${this.promotion.word}`;
  }
}

/** What the Release view shows of one release, worked out once. */
export class ReleasePage {
  readonly release: ReleaseView;
  /** The build the operator asked production to take, if one waits. */
  readonly requested: string | null;

  constructor(release: ReleaseView, requested: string | null) {
    this.release = release;
    this.requested = requested;
  }

  /** The commit production runs, when it is a clean or modified commit. */
  get production(): string | null {
    const { production } = this.release;
    return production.kind === 'commit' ? production.commit : null;
  }

  /** Whether the canary's build can be asked for: it passed, and
   * production does not run it already. */
  get promotable(): boolean {
    const canary = this.release.canary;
    return canary !== null && canary.passed && this.release.decision !== 'current';
  }

  /** Whether the operator already asked for the canary's build. */
  get asked(): boolean {
    return this.requested !== null && this.requested === this.release.canary?.commit;
  }

  /** How far behind main production is, in words. */
  get behind(): string {
    const { behind } = this.release;
    if (behind === null) return 'not counted against main';
    if (behind === 0) return 'up to date with main';
    return behind === 1 ? '1 commit behind main' : `${behind} commits behind main`;
  }

  /** The work items production's build shipped. */
  shipped(items: readonly WorkItem[]): WorkItem[] {
    const production = this.production;
    return items.filter(
      (item) =>
        item.state === 'shipped' &&
        item.shipped?.kind === 'commit' &&
        item.shipped.commit === production,
    );
  }

  /** The work items on main that production does not run yet. */
  static waiting(items: readonly WorkItem[]): WorkItem[] {
    return items.filter((item) => item.state === 'landed');
  }

  /** A commit as people read it. */
  static short(commit: string | null): string {
    return commit ? commit.slice(0, 12) : 'none';
  }
}

/** What a pull request's chip on the flow says: how it stands, its checks,
 * or its preview. */
export type PullMode = 'status' | 'checks' | 'preview';

/** A pull request's chip: its glyph and what the glyph says. */
export interface PullChip {
  mark: Mark;
  label: string;
}

export class PullChips {
  static readonly modes: readonly { mode: PullMode; label: string }[] = [
    { mode: 'status', label: 'Status' },
    { mode: 'checks', label: 'Checks' },
    { mode: 'preview', label: 'Preview' },
  ];

  static of(pull: Pull, mode: PullMode, previews: readonly Preview[]): PullChip {
    switch (mode) {
      case 'status':
        return PullChips.status(pull);
      case 'checks':
        return PullChips.checks(pull);
      case 'preview':
        return PullChips.preview(
          previews.find((each) => each.of.kind === 'pull' && each.of.number === pull.number) ??
            null,
        );
      default: {
        const never: never = mode;
        return never;
      }
    }
  }

  /** What stands between it and merging, the most pressing first. */
  /** Where a pull request stands: the first that holds, in this order. */
  static standing(pull: Pull): PullStanding {
    if (pull.readiness === 'draft' || pull.merge === 'draft') return 'draft';
    if (pull.checks === 'failing' || pull.checks === 'errored') return 'failing';
    if (pull.merge === 'conflicting') return 'conflicting';
    if (pull.checks === 'pending' || pull.checks === 'expected') return 'running';
    if (pull.review === 'changes_requested') return 'changes';
    if (pull.review === 'review_required') return 'review';
    if (pull.merge === 'behind') return 'behind';
    if (pull.merge === 'blocked') return 'blocked';
    if (pull.merge === 'clean' || pull.merge === 'unstable' || pull.merge === 'has_hooks')
      return 'ready';
    return 'unknown';
  }

  /** Its status chip: where it stands. */
  static status(pull: Pull): PullChip {
    const standing = PullChips.standing(pull);
    switch (standing) {
      case 'draft':
        return { mark: 'draft', label: 'Draft' };
      case 'failing':
        return { mark: 'failed', label: 'Checks failing' };
      case 'conflicting':
        return { mark: 'failed', label: 'Conflicts with its base' };
      case 'running':
        return { mark: 'running', label: 'Checks running' };
      case 'changes':
        return { mark: 'warning', label: 'Changes requested' };
      case 'review':
        return { mark: 'yours', label: 'Review required' };
      case 'behind':
        return { mark: 'warning', label: 'Behind its base' };
      case 'blocked':
        return { mark: 'warning', label: 'Blocked' };
      case 'ready':
        return { mark: 'passed', label: 'Ready to merge' };
      case 'unknown':
        return { mark: 'unknown', label: 'Not worked out yet' };
      default:
        return assertNever(standing);
    }
  }

  /** Its checks taken together, with how many passed. */
  static checks(pull: Pull): PullChip {
    const passed = pull.check_list.filter((check) => check.state === 'passed').length;
    const of = `${passed}/${pull.check_list.length} checks passed`;
    switch (pull.checks) {
      case 'passing':
        return { mark: 'passed', label: of };
      case 'failing':
      case 'errored':
        return { mark: 'failed', label: of };
      case 'pending':
      case 'expected':
        return { mark: 'running', label: of };
      case 'none':
        return { mark: 'idle', label: 'No checks' };
      default:
        return { mark: 'unknown', label: unrecognised(pull.checks) };
    }
  }

  static preview(preview: Preview | null): PullChip {
    if (!preview) return { mark: 'idle', label: 'No preview' };
    return Previewing.chip(preview);
  }
}

/** Where a pull request stands, as its status chip says it. */
export type PullStanding =
  | 'draft'
  | 'failing'
  | 'conflicting'
  | 'running'
  | 'changes'
  | 'review'
  | 'behind'
  | 'blocked'
  | 'ready'
  | 'unknown';

/** One of the figures under the flow. */
export interface StripItem {
  label: string;
  value: string;
  mark: Mark | null;
}

/** The figures under the flow: production against main, the canary, the
 * policy, the open pull requests, and what comes next. */
export class ReleaseFigures {
  static of(view: ReleasePage, pulls: readonly Pull[]): StripItem[] {
    const release = view.release;
    const canary = release.canary;
    const failing = pulls.filter((pull) => {
      const standing = PullChips.standing(pull);
      return standing === 'failing' || standing === 'conflicting';
    }).length;
    const promotion = Promotion.of(release.decision);
    return [
      {
        label: 'Production',
        value:
          release.behind === null
            ? 'not on main'
            : release.behind === 0
              ? 'on main'
              : `${release.behind} behind main`,
        mark: release.behind ? 'warning' : null,
      },
      {
        label: 'Canary',
        value: canary
          ? `${canary.checks.filter((check) => check.passed).length}/${canary.checks.length}`
          : 'none checked',
        mark: canary ? (canary.passed ? 'passed' : 'failed') : null,
      },
      { label: 'Policy', value: release.policy, mark: null },
      {
        label: 'Open pull requests',
        value: failing ? `${pulls.length} · ${failing} failing` : String(pulls.length),
        mark: failing ? 'failed' : null,
      },
      {
        label: 'Next',
        value:
          view.promotable && canary && !view.asked
            ? `Promote ${ReleasePage.short(canary.commit).slice(0, 7)}`
            : promotion.word,
        mark: promotion.mark,
      },
    ];
  }

  /** Where a canary check comes from: Ren's own, or one the project declares. */
  static source(check: CheckView): string {
    return check.check === 'declared' ? 'declared' : 'Ren';
  }
}

/** The open pull requests a watch saw, and what it says about the watch. */
export class PullWatch {
  static pulls(watch: Pulls | null): Pull[] {
    if (!watch) return [];
    switch (watch.watch) {
      case 'seen':
        return watch.seen.pull_requests;
      case 'failed':
        return watch.last?.pull_requests ?? [];
      case 'off':
      case 'waiting':
      case 'unknown':
        return [];
      default: {
        const never: never = watch;
        return never;
      }
    }
  }

  /** The repository it watches, when it watches one. */
  static repository(watch: Pulls | null): string | null {
    if (!watch) return null;
    switch (watch.watch) {
      case 'seen':
      case 'failed':
      case 'waiting':
        return watch.repository;
      case 'off':
      case 'unknown':
        return null;
      default: {
        const never: never = watch;
        return never;
      }
    }
  }

  /** Why no pull requests show, when none do for a reason. */
  static note(watch: Pulls | null): string | null {
    if (!watch) return 'Pull requests did not read';
    switch (watch.watch) {
      case 'off':
        return 'No GitHub repository watched';
      case 'waiting':
        return 'Waiting for GitHub';
      case 'failed':
        return `GitHub did not answer: ${watch.error}`;
      case 'seen':
        return null;
      case 'unknown':
        return `GitHub watch ${watch.text}`;
      default: {
        const never: never = watch;
        return never;
      }
    }
  }
}

/** What can be done with a preview from its row. */
export type PreviewAction =
  | { kind: 'open'; href: string }
  /** Withdraw a commit asked for. */
  | { kind: 'stop'; commit: string }
  /** Ask a failed commit again. */
  | { kind: 'retry'; commit: string }
  | { kind: 'none' };

export interface PreviewRow {
  slot: number;
  chip: PullChip;
  /** `#418 Retry webhook deliveries`, or a commit. */
  of: string;
  action: PreviewAction;
}

/** The factory's previews as rows, in slot order. */
export class Previewing {
  static chip(preview: Preview): PullChip {
    switch (preview.state.state) {
      case 'building':
        return { mark: 'running', label: 'Building' };
      case 'running':
        return { mark: 'passed', label: 'Up' };
      case 'failed':
        return { mark: 'failed', label: `Failed: ${preview.state.detail}` };
      case 'unknown':
        return { mark: 'unknown', label: preview.state.text };
      default: {
        const never: never = preview.state;
        return never;
      }
    }
  }

  /** `host` is the page's own: a preview published on the tailnet answers there. */
  static rows(previews: readonly Preview[], pulls: readonly Pull[], host: string): PreviewRow[] {
    return previews
      .toSorted((a, b) => a.slot - b.slot)
      .map((preview) => {
        const pull =
          preview.of.kind === 'pull'
            ? pulls.find((each) => preview.of.kind === 'pull' && each.number === preview.of.number)
            : undefined;
        const of =
          preview.of.kind === 'pull'
            ? `#${preview.of.number}${pull ? ` ${pull.title}` : ''}`
            : ReleasePage.short(preview.commit);
        return {
          slot: preview.slot,
          chip: Previewing.chip(preview),
          of,
          action: Previewing.action(preview, host),
        };
      });
  }

  static action(preview: Preview, host: string): PreviewAction {
    const asked = preview.of.kind === 'commit';
    switch (preview.state.state) {
      case 'running':
        return preview.state.reach === 'tailnet'
          ? { kind: 'open', href: `https://${host}:${preview.published_ui_port}/` }
          : asked
            ? { kind: 'stop', commit: preview.commit }
            : { kind: 'none' };
      case 'failed':
        return asked ? { kind: 'retry', commit: preview.commit } : { kind: 'none' };
      case 'building':
        return asked ? { kind: 'stop', commit: preview.commit } : { kind: 'none' };
      case 'unknown':
        return { kind: 'none' };
      default: {
        const never: never = preview.state;
        return never;
      }
    }
  }
}

/** The Release view's writes: asking production to take the canary's
 * build, and asking for or withdrawing a commit's preview, each carried
 * out by the factory's next pass. */
export class ReleaseApi {
  readonly #fetch: typeof fetch;

  constructor(fetcher: typeof fetch) {
    this.#fetch = fetcher;
  }

  promote(commit: string): Promise<void> {
    return this.#post('/control/release/promote', commit);
  }

  /** Ask for a preview of `commit`, a commit or a branch. */
  ask(commit: string): Promise<void> {
    return this.#post('/control/previews/ask', commit);
  }

  withdraw(commit: string): Promise<void> {
    return this.#post('/control/previews/withdraw', commit);
  }

  async #post(path: string, commit: string): Promise<void> {
    const response = await this.#fetch(path, {
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
