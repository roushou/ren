// Where the release stands, as one status-line entry: how far production
// is behind main, the canary's verdict, and what promotion decided. Pure,
// read from `/api/release` (`ReleaseView`); the factory writes it.
import type { Revision } from './api.ts';
import type { ReleaseView } from './api.ts';
import { unrecognised } from './decode.ts';

/** The tone the entry is drawn in. */
export type ReleaseTone = 'ok' | 'warning' | 'bad';

/** The status line's release entry: short text, its tone, and the detail on hover. */
export interface ReleaseLine {
  text: string;
  tone: ReleaseTone;
  detail: string;
}

/** A commit as people read it: its first 12 digits. */
const commit = (id: string) => id.slice(0, 12);

const revisionText = (revision: Revision) => {
  switch (revision.kind) {
    case 'commit':
      return revision.tree === 'clean' ? commit(revision.commit) : `${commit(revision.commit)}, modified`;
    case 'unknown':
      return 'unknown';
    case 'unrecognized':
      return revision.text;
  }
};

export const ReleaseLine = {
  of(release: ReleaseView): ReleaseLine {
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
            check.passed ? `${check.check} passed` : `${check.check} failed${check.detail ? `: ${check.detail}` : ''}`,
          )
          .join('\n  ')
      : '';
    const detail = [
      `main ${commit(release.main)}`,
      `production ${revisionText(release.production)}`,
      canary ? `canary ${commit(canary.commit)}\n  ${checks}` : 'canary: none checked',
      `promotion (${release.policy}): ${decision}`,
    ].join('\n');
    return { text: `${behind} · ${verdict}`, tone, detail };
  },

  /** What promotion decided, as a sentence, and the tone it is drawn in. */
  decision(release: ReleaseView): [string, ReleaseTone] {
    switch (release.decision) {
      case 'promote':
        return ["production takes the canary's build now", 'ok'];
      case 'current':
        return ["production runs the canary's build", 'ok'];
      case 'failed':
        return ["the canary's build failed; production keeps its own", 'bad'];
      case 'unchecked':
        return ['no build has been checked', 'warning'];
      case 'production_ahead':
        return ["production has commits the canary's build lacks", 'warning'];
      case 'unconfirmed':
        return ['production runs a build the factory cannot place', 'warning'];
      case 'awaiting_operator':
        return ['passed; waiting for the operator to promote it', 'warning'];
      case 'outside_window':
        return ['passed; production takes it when the window opens', 'ok'];
      case 'promotion_failed':
        return ['production failed to take it and was rolled back', 'bad'];
      default:
        return [unrecognised(release.decision), 'warning'];
    }
  },
};
