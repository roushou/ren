/**
 * Ren's side of Redline: where a brief goes in Ren (a new session in Ren's
 * own source checkout, or one running there), how it reaches an open
 * session, and what the brief says of Ren (the route's file, the build, and
 * how the change is to be made). Pure, so it is tested without a browser.
 */
import type { Revision } from './api.ts';
import { Redline, type Framing, type Mark, type PageContext } from './redline.ts';
import { SignIns } from './signIn.ts';
import {
  Sessions,
  type ComposeMode,
  type Drivable,
  type RunClient,
  type DeskSession,
  type RunSnapshot,
  type SessionBody,
} from './runs.ts';

/** Where a brief goes: a new session in the checkout, or one running there. */
export type Destination = { kind: 'new' } | { kind: 'session'; session: DeskSession };

/**
 * Where a brief's change is made: in the checkout itself, when no other
 * session is open there, or in a git worktree, so it stays clear of the
 * sessions that are.
 */
export type WorkPlace = 'checkout' | 'worktree';

/** How a brief reaches an open session: the prompt to send, or why it waits. */
export type Handoff =
  { kind: 'send'; delivery: 'start' | 'queue'; text: string } | { kind: 'wait'; reason: string };

/** What Ren adds to the page a brief was marked on. */
export interface RenPage {
  /** SvelteKit's route id (`/sessions`), when the page has one. */
  route: string | null;
  /** What the daemon serving the page was built from, when it says. */
  revision: Revision | null;
}

/** Where a brief can go: the clients ready here, for a new session, and the
 * sessions running in the checkout. */
export interface Destinations {
  ready: Drivable[];
  /** Each client as the daemon describes it: its approval policies, and
   * what it takes mid-run. */
  clients: RunClient[];
  sessions: DeskSession[];
}

/** Reading the destinations: not yet, under way, read, or failed. */
export type Reading =
  | { state: 'idle' }
  | { state: 'reading' }
  | { state: 'read'; value: Destinations }
  | { state: 'failed'; reason: string };

export class Briefing {
  /**
   * `REN_SOURCE_DIR`: Ren's source checkout, where a brief is sent to be
   * worked on. Absent, Redline marks but cannot send.
   */
  static checkout(value: string | undefined): string | undefined {
    if (value === undefined || value === '') return undefined;
    if (!value.startsWith('/')) throw new Error('REN_SOURCE_DIR must be an absolute path.');
    return value.length > 1 ? value.replace(/\/+$/, '') : value;
  }

  /** The most sessions the pane offers to send into. */
  static readonly OFFERED = 4;

  /**
   * The sessions in `checkout` a brief can go into: open, with their client
   * running now (an ended or never-prompted session is no place to add to),
   * as the Sessions page orders them: one waiting for you first, then one
   * working, then the most recently active.
   */
  static offered(
    bodies: readonly SessionBody[],
    live: readonly RunSnapshot[],
    checkout: string,
  ): DeskSession[] {
    const open = bodies.filter((body) => {
      const session = Sessions.desk(body, live);
      return (
        session !== null &&
        session.shelf === 'open' &&
        session.workdir === checkout &&
        Sessions.running(session)
      );
    });
    return (Sessions.folders(open, live)[0]?.sessions ?? []).slice(0, Briefing.OFFERED);
  }

  /**
   * Another client a brief could go to when `harness` is signed out: the
   * first of `ready` other than it whose sign-in is not known to be out.
   * Offered for the operator to choose, never switched to on its own.
   */
  static instead(
    ready: readonly Drivable[],
    clients: readonly RunClient[],
    harness: Drivable,
  ): Drivable | null {
    return (
      ready.find((drivable) => {
        if (drivable === harness) return false;
        const entry = clients.find((client) => client.harness === drivable);
        return entry !== undefined && !SignIns.signedOut(entry.sign_in);
      }) ?? null
    );
  }

  /**
   * Where a brief sent to `destination` makes its change: in the checkout
   * when no session of `offered` other than the destination is open there,
   * else in a worktree. A worktree in Rust costs a cold build and a target
   * folder of its own, so it is asked for only when another session could
   * be editing the same files or be switched onto the brief's branch.
   */
  static place(destination: Destination, offered: readonly DeskSession[]): WorkPlace {
    const others = offered.filter(
      (session) => destination.kind === 'new' || session.session !== destination.session.session,
    );
    return others.length ? 'worktree' : 'checkout';
  }

  /** How a change is to be made, in the brief's closing paragraph. */
  static #work(place: WorkPlace, branch: string): string {
    switch (place) {
      case 'checkout':
        return `make it on a new branch, ${Redline.code(branch)}, in this checkout, since no other session is open here; if ${Redline.code('git status')} shows uncommitted changes, use a git worktree of this repository instead. Run the project's checks, commit, and switch the checkout back to the branch it was on.`;
      case 'worktree':
        return `make it on a new branch, ${Redline.code(branch)}, in a git worktree of this repository, not in the checkout itself, since other sessions are open there, and run the project's checks.`;
      default: {
        const never: never = place;
        return never;
      }
    }
  }

  /**
   * How `brief` goes into a session whose composer is in `mode`: it starts
   * a turn when none runs, or is queued after the running one (after
   * anything queued before, as one message), never steering it.
   */
  static handoff(mode: ComposeMode, brief: string): Handoff {
    switch (mode.kind) {
      case 'start':
      case 'continue':
        return { kind: 'send', delivery: 'start', text: brief };
      case 'during':
        return {
          kind: 'send',
          delivery: 'queue',
          text: mode.queued ? `${mode.queued}\n\n${brief}` : brief,
        };
      case 'closed':
        return { kind: 'wait', reason: mode.reason };
      default: {
        const never: never = mode;
        return never;
      }
    }
  }

  /** What the brief says of the build the page came from, if anything. */
  static built(revision: Revision | null): string | null {
    if (!revision) return null;
    switch (revision.kind) {
      case 'commit':
        return `This page was served by a build of commit ${Redline.code(revision.commit.slice(0, 12))}${revision.tree === 'modified' ? ' with uncommitted changes' : ''}; the checkout may have moved on since.`;
      case 'unknown':
        return null;
      case 'unrecognized':
        return `This page was served by a build of ${Redline.code(revision.text)}; the checkout may have moved on since.`;
      default: {
        const never: never = revision;
        return never;
      }
    }
  }

  /** The Svelte file that renders a route of Ren's UI. */
  static routeFile(route: string): string {
    return `ui/src/routes${route === '/' ? '' : route}/+page.svelte`;
  }

  /** What Ren adds to a brief: the route's file and the build, and how to
   * work: questions are answered in place, and only a change gets a
   * branch, made where `place` says. */
  static framing(ren: RenPage, marks: readonly Mark[], place: WorkPlace): Framing {
    const about: string[] = [];
    if (ren.route) {
      about.push(`In Ren's UI this page is ${Redline.code(Briefing.routeFile(ren.route))}.`);
    }
    const built = Briefing.built(ren.revision);
    if (built) about.push(built);
    const branch = Redline.branch(Redline.noted(marks));
    return {
      about,
      closing: `Answer what the notes ask without changing anything. If a note asks for a change, ${Briefing.#work(place, branch)} Don't install or restart anything: I'll review the branch.`,
    };
  }

  /** The brief an agent gets in Ren. */
  static brief(page: PageContext, ren: RenPage, marks: readonly Mark[], place: WorkPlace): string {
    return Redline.brief(page, marks, Briefing.framing(ren, marks, place));
  }
}
