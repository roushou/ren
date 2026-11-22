// System · GitHub: each project's watched repository, its open pull requests by
// where they stand, and the newest answer. Pure, read from each project's
// `/api/projects/{name}/pulls`; tested in tests/github.test.ts.
import type { Pull, Pulls } from './api.ts';
import type { Mark } from './marks.ts';
import { assertNever } from './decode.ts';
import { PullChips, PullWatch } from './release.ts';

/** Where a pull request stands, as the watched repositories group them:
 * held is waiting on its author or its base (changes requested, behind,
 * blocked). */
export type PullGroup = 'ready' | 'review' | 'held' | 'failing' | 'running' | 'draft' | 'unknown';

/** Each current project's GitHub watch; null where its pulls did not read. */
export interface GitHubData {
  watches: { project: string; watch: Pulls | null }[];
  failed: string | null;
}

/** One project's watch. */
export interface WatchRow {
  project: string;
  repository: string | null;
  /** How the watch stands: answered, failing, waiting for a first answer, or off. */
  mark: Mark;
  state: Pulls['watch'] | 'unread';
  pulls: Pull[];
  /** Its pull requests by group. */
  groups: Record<PullGroup, number>;
  /** When GitHub last answered; null before it did. */
  at: number | null;
  /** Why the newest poll failed. */
  error: string | null;
}

/** One open pull request across the watched repositories. */
export interface PullRow {
  project: string;
  pull: Pull;
  group: PullGroup;
}

export interface WatchSummary {
  watched: number;
  failing: number;
  open: number;
  /** Pull requests whose checks fail. */
  checksFailing: number;
  ready: number;
}

export class GitHubWatches {
  static readonly groups: readonly PullGroup[] = [
    'ready',
    'review',
    'held',
    'failing',
    'running',
    'draft',
    'unknown',
  ];

  /** The group a pull request is counted in. */
  static group(pull: Pull): PullGroup {
    const standing = PullChips.standing(pull);
    switch (standing) {
      case 'ready':
      case 'review':
      case 'failing':
      case 'running':
      case 'draft':
      case 'unknown':
        return standing;
      case 'conflicting':
        return 'failing';
      case 'changes':
      case 'behind':
      case 'blocked':
        return 'held';
      default:
        return assertNever(standing);
    }
  }

  /** Each project's watch; a project whose pulls did not read is `unread`. */
  static rows(watches: readonly { project: string; watch: Pulls | null }[]): WatchRow[] {
    return watches.map(({ project, watch }): WatchRow => {
      const pulls = PullWatch.pulls(watch);
      const groups = Object.fromEntries(GitHubWatches.groups.map((group) => [group, 0])) as Record<
        PullGroup,
        number
      >;
      for (const pull of pulls) groups[GitHubWatches.group(pull)] += 1;
      const state = watch?.watch ?? 'unread';
      return {
        project,
        repository: PullWatch.repository(watch),
        state,
        mark: GitHubWatches.mark(state),
        pulls,
        groups,
        at:
          watch?.watch === 'seen'
            ? watch.seen.at
            : watch?.watch === 'failed'
              ? (watch.last?.at ?? null)
              : null,
        error: watch?.watch === 'failed' ? watch.error : null,
      };
    });
  }

  /** How a watch stands, as its glyph says it. */
  static mark(state: WatchRow['state']): Mark {
    switch (state) {
      case 'seen':
        return 'passed';
      case 'failed':
      case 'unread':
        return 'failed';
      case 'waiting':
        return 'running';
      case 'off':
        return 'idle';
      case 'unknown':
        return 'unknown';
      default:
        return assertNever(state);
    }
  }

  /** The open pull requests of the watched repositories, failing first. */
  static pulls(rows: readonly WatchRow[]): PullRow[] {
    const rank: Record<PullGroup, number> = {
      failing: 0,
      held: 1,
      review: 2,
      ready: 3,
      running: 4,
      draft: 5,
      unknown: 6,
    };
    return rows
      .flatMap((row) =>
        row.pulls.map((pull) => ({
          project: row.project,
          pull,
          group: GitHubWatches.group(pull),
        })),
      )
      .sort((a, b) => rank[a.group] - rank[b.group] || b.pull.number - a.pull.number);
  }

  static summary(rows: readonly WatchRow[]): WatchSummary {
    const watched = rows.filter((row) => row.state !== 'off');
    const pulls = GitHubWatches.pulls(rows);
    return {
      watched: watched.length,
      failing: watched.filter((row) => row.mark === 'failed').length,
      open: pulls.length,
      checksFailing: pulls.filter(
        (row) => row.pull.checks === 'failing' || row.pull.checks === 'errored',
      ).length,
      ready: pulls.filter((row) => row.group === 'ready').length,
    };
  }
}
