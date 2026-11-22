// Home (`/`): what is under way across projects, and what happened since
// the operator last looked, read from the board the layout loads.
// What needs them is the board's own (approvals, signed-out clients,
// canaries, previews). Pure: no DOM, no clock, no storage.

import type { Mark } from './marks.ts';
import type { ProjectBoard } from './projects.ts';
import { Runs, Sessions, type SessionId } from './runs.ts';
import { Work } from './work.ts';

/** A session at work now. */
export interface UnderWay {
  session: SessionId;
  title: string;
  project: string | null;
  mark: Mark;
  /** What it is doing, in words: its state's. */
  doing: string;
  /** When its current run started. */
  since: number | null;
  href: string;
}

/** What happened: a session ended, or a work item landed or shipped. */
export type OutcomeKind = 'ended' | 'landed' | 'shipped';

/** Something that ended or moved on since the operator looked. */
export interface Outcome {
  key: string;
  kind: OutcomeKind;
  mark: Mark;
  /** What happened, for the glyph's label. */
  happened: string;
  title: string;
  project: string | null;
  at: number;
  href: string | null;
}

/** What the outcomes add up to. */
export interface OutcomeCounts {
  ended: number;
  landed: number;
  shipped: number;
  failed: number;
}

export class Home {
  /** Running sessions across projects: the ones waiting on the operator
   * first, then the newest. */
  static underWay(board: ProjectBoard): UnderWay[] {
    const rows = board.live.flatMap((run): UnderWay[] => {
      if (run.state.phase === 'ended') return [];
      const stored = board.sessions.find((body) => body.session === run.session)?.stored ?? null;
      const project = board.nameOf(stored?.project ?? null);
      const said = Runs.mark(run.state);
      return [
        {
          session: run.session,
          title: stored?.first_prompt ?? 'A new session',
          project,
          mark: said.mark,
          doing: said.label,
          since: stored?.runs.at(-1)?.started_at ?? null,
          href: Sessions.deskHref(project, run.session),
        },
      ];
    });
    return rows.toSorted(
      (a, b) =>
        Number(b.mark === 'yours') - Number(a.mark === 'yours') || (b.since ?? 0) - (a.since ?? 0),
    );
  }

  /** What ended or moved on after `since`, newest first: sessions whose
   * last run ended, work that landed or shipped. */
  static outcomes(board: ProjectBoard, since: number): Outcome[] {
    const sessions = board.sessions.flatMap((body): Outcome[] => {
      const stored = body.stored;
      const run = stored?.runs.at(-1);
      if (!stored || !run?.ended_at || run.ended_at <= since) return [];
      const project = board.nameOf(stored.project);
      const ended = run.end ? Runs.endMark(run.end, run.exit) : 'idle';
      return [
        {
          key: `session:${body.session}`,
          kind: 'ended',
          mark: ended === 'idle' ? 'passed' : ended,
          happened: ended === 'idle' ? 'Ended' : 'Ended short',
          title: stored.first_prompt ?? 'A session',
          project,
          at: run.ended_at,
          href: Sessions.deskHref(project, body.session),
        },
      ];
    });
    const work = board.work.flatMap((item): Outcome[] => {
      if (item.updated_at <= since) return [];
      const kind: OutcomeKind | null =
        item.state === 'landed' ? 'landed' : item.state === 'shipped' ? 'shipped' : null;
      if (kind === null) return [];
      const home = board.homeOf(item);
      return [
        {
          key: `work:${item.id}`,
          kind,
          mark: 'passed',
          happened: kind === 'landed' ? 'Landed on main' : 'Shipped',
          title: item.title,
          project: home,
          at: item.updated_at,
          href: home ? Work.href(home, item.id) : null,
        },
      ];
    });
    return [...sessions, ...work].toSorted((a, b) => b.at - a.at);
  }

  /** When the operator last looked, said as a phrase: just now, so long
   * ago within the day, then since its day (`day` names a day). */
  static looked(seen: number, now: number, day?: (ms: number) => string): string {
    const minutes = Math.floor(Math.max(0, now - seen) / 60_000);
    if (minutes < 1) return 'just now';
    if (minutes < 24 * 60) return `${Sessions.age(seen, now, day)} ago`;
    return `since ${Sessions.age(seen, now, day)}`;
  }

  static counts(outcomes: readonly Outcome[]): OutcomeCounts {
    return {
      ended: outcomes.filter((one) => one.kind === 'ended').length,
      landed: outcomes.filter((one) => one.kind === 'landed').length,
      shipped: outcomes.filter((one) => one.kind === 'shipped').length,
      failed: outcomes.filter((one) => one.mark === 'failed' || one.mark === 'warning').length,
    };
  }
}
