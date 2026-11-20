// Where the release stands, as one status-line entry: how far production
// is behind main, the canary's verdict, and what promotion decided. Pure,
// read from `/api/release` (`ReleaseView`); the factory writes it.
import type { Revision } from './api.ts';
import type { ReleaseView } from './api.ts';
import { unrecognised } from './decode.ts';
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

  constructor(word: string, sentence: string, tone: ReleaseTone) {
    this.word = word;
    this.sentence = sentence;
    this.tone = tone;
  }

  static of(decision: ReleaseView['decision']): Promotion {
    switch (decision) {
      case 'promote':
        return new Promotion('Promoting', "production takes the canary's build now", 'ok');
      case 'current':
        return new Promotion('Live', "production runs the canary's build", 'ok');
      case 'failed':
        return new Promotion(
          'Blocked',
          "the canary's build failed; production keeps its own",
          'bad',
        );
      case 'unchecked':
        return new Promotion('Unchecked', 'no build has been checked', 'warning');
      case 'production_ahead':
        return new Promotion(
          'Diverged',
          "production has commits the canary's build lacks",
          'warning',
        );
      case 'unconfirmed':
        return new Promotion(
          'Unknown build',
          'production runs a build the factory cannot place',
          'warning',
        );
      case 'awaiting_operator':
        return new Promotion(
          'Waiting on you',
          'passed; waiting for the operator to promote it',
          'warning',
        );
      case 'outside_window':
        return new Promotion(
          'Scheduled',
          'passed; production takes it when the window opens',
          'ok',
        );
      case 'promotion_failed':
        return new Promotion(
          'Rolled back',
          'production failed to take it and was rolled back',
          'bad',
        );
      default:
        return new Promotion(unrecognised(decision), unrecognised(decision), 'warning');
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

/** The Release view's one write: asking production to take the canary's
 * build, which the factory's next pass carries out. */
export class ReleaseApi {
  readonly #fetch: typeof fetch;

  constructor(fetcher: typeof fetch) {
    this.#fetch = fetcher;
  }

  async promote(commit: string): Promise<void> {
    const response = await this.#fetch('/control/release/promote', {
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
