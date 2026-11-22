// System · Integrations: what Ren reaches outside itself. Today that is
// GitHub: each project's watched repository, its open pull requests by
// where they stand, and the newest answer. Pure, read from each project's
// `/api/projects/{name}/pulls`; tested in tests/integrations.test.ts.
import type { Pull, Pulls } from './api.ts';
import type { Mark } from './marks.ts';
import { PullChips, PullWatch } from './release.ts';

/** Where a pull request stands, as the watched repositories count them. */
export type PullStanding = 'ready' | 'review' | 'failing' | 'running' | 'draft' | 'other';

/** One project's watch. */
export interface WatchRow {
  project: string;
  repository: string | null;
  /** How the watch stands: answered, failing, waiting for a first answer, or off. */
  mark: Mark;
  state: Pulls['watch'] | 'unread';
  pulls: Pull[];
  /** Its pull requests by standing. */
  standings: Record<PullStanding, number>;
  /** When GitHub last answered; null before it did. */
  at: number | null;
  /** Why the newest poll failed. */
  error: string | null;
}

/** One open pull request across the watched repositories. */
export interface PullRow {
  project: string;
  pull: Pull;
  standing: PullStanding;
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
  static readonly standings: readonly PullStanding[] = [
    'ready',
    'review',
    'failing',
    'running',
    'draft',
    'other',
  ];

  /** Where a pull request stands, from its status chip. */
  static standing(pull: Pull): PullStanding {
    switch (PullChips.status(pull).mark) {
      case 'passed':
        return 'ready';
      case 'yours':
        return 'review';
      case 'failed':
        return 'failing';
      case 'running':
        return 'running';
      case 'draft':
        return 'draft';
      default:
        return 'other';
    }
  }

  /** Each project's watch; a project whose pulls did not read is `unread`. */
  static rows(watches: readonly { project: string; watch: Pulls | null }[]): WatchRow[] {
    return watches.map(({ project, watch }): WatchRow => {
      const pulls = PullWatch.pulls(watch);
      const standings = Object.fromEntries(
        GitHubWatches.standings.map((standing) => [standing, 0]),
      ) as Record<PullStanding, number>;
      for (const pull of pulls) standings[GitHubWatches.standing(pull)] += 1;
      const state = watch?.watch ?? 'unread';
      return {
        project,
        repository: PullWatch.repository(watch),
        state,
        mark:
          state === 'seen'
            ? 'passed'
            : state === 'failed' || state === 'unread'
              ? 'failed'
              : state === 'waiting'
                ? 'running'
                : 'idle',
        pulls,
        standings,
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

  /** The open pull requests of the watched repositories, failing first. */
  static pulls(rows: readonly WatchRow[]): PullRow[] {
    const rank: Record<PullStanding, number> = {
      failing: 0,
      review: 1,
      ready: 2,
      running: 3,
      draft: 4,
      other: 5,
    };
    return rows
      .flatMap((row) =>
        row.pulls.map((pull) => ({
          project: row.project,
          pull,
          standing: GitHubWatches.standing(pull),
        })),
      )
      .sort((a, b) => rank[a.standing] - rank[b.standing] || b.pull.number - a.pull.number);
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
      ready: pulls.filter((row) => row.standing === 'ready').length,
    };
  }
}
