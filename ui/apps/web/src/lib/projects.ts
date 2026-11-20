// The Projects view's board, pure (docs/plans/hierarchy.md): the
// registered projects with what is under way for each, and what waits on
// the operator across them: a canary to promote, a preview to check, work
// still queued. Read from `/api/projects` and `/api/work`.
import type { Project } from './api.ts';
import {
  Sessions,
  type ApprovalAsk,
  type ApprovalId,
  type RunSnapshot,
  type SessionBody,
  type SessionId,
} from './runs.ts';
import { Work, type WorkItem } from './work.ts';

/** An approval a running session waits on, with the project it works for. */
export interface Approval {
  session: SessionId;
  approval: ApprovalId;
  ask: ApprovalAsk;
  /** The project's name; null when the session works for none. */
  project: string | null;
  /** The first line of the session's first prompt, to know it by. */
  prompt: string | null;
}

/** A queued work item's project, by name; null when none holds it. */
export interface Queued {
  project: string | null;
  count: number;
}

/** One state's work items, in the order a project's page shows them. */
export interface WorkGroup {
  state: 'queued' | 'sent' | 'landed' | 'shipped' | 'dismissed';
  label: string;
  /** What the state means, in a few words. */
  note: string;
  items: WorkItem[];
}

/** The projects and the work under them, as the Projects view shows them. */
export class ProjectBoard {
  readonly projects: readonly Project[];
  readonly work: readonly WorkItem[];
  /** The runs the daemon holds now; none when this UI only reads. */
  readonly live: readonly RunSnapshot[];
  /** The current sessions, for the project each running one works for. */
  readonly sessions: readonly SessionBody[];
  /** The one project the board is for, by name; null for every project. */
  readonly only: string | null;

  constructor(
    projects: readonly Project[],
    work: readonly WorkItem[],
    live: readonly RunSnapshot[] = [],
    sessions: readonly SessionBody[] = [],
    only: string | null = null,
  ) {
    this.projects = projects;
    this.work = work;
    this.live = live;
    this.sessions = sessions;
    this.only = only;
  }

  /** The board of `project` alone: its work (Ren's holds the items from
   * before projects), and the approvals its sessions wait on. */
  static of(
    project: Project,
    work: readonly WorkItem[],
    live: readonly RunSnapshot[],
    sessions: readonly SessionBody[],
  ): ProjectBoard {
    return new ProjectBoard([project], Work.of(work, project), live, sessions, project.name);
  }

  /** Every approval a running session waits on, oldest run first; on a
   * project's board, its sessions' alone. */
  get approvals(): Approval[] {
    const all = this.live.flatMap((run) => {
      if (run.state.phase !== 'working') return [];
      const stored = this.sessions.find((body) => body.session === run.session)?.stored ?? null;
      return run.state.approvals.map((waiting) => ({
        session: run.session,
        approval: waiting.approval,
        ask: waiting.ask,
        project: this.nameOf(stored?.project ?? null),
        prompt: stored?.first_prompt ?? null,
      }));
    });
    return this.only === null ? all : all.filter((waiting) => waiting.project === this.only);
  }

  /** The projects being built: unarchived, Ren itself first, then by name. */
  get active(): Project[] {
    return this.projects
      .filter((project) => project.archived_at === null)
      .toSorted(
        (a, b) =>
          Number(b.factory === 'ren') - Number(a.factory === 'ren') || a.name.localeCompare(b.name),
      );
  }

  get archived(): Project[] {
    return this.projects.filter((project) => project.archived_at !== null);
  }

  /** Projects whose canary passed and waits for the operator to promote it. */
  get toPromote(): Project[] {
    return this.active.filter((project) => project.release?.decision === 'awaiting_operator');
  }

  /** Sent items with a preview up: each waits for the operator to look. */
  get previews(): WorkItem[] {
    return this.work.filter((item) => item.state === 'sent' && item.preview !== null);
  }

  /** Queued items by project, the projects' first, then those for none. */
  get queued(): Queued[] {
    const counts = new Map<string | null, number>();
    for (const item of this.work) {
      if (item.state !== 'queued') continue;
      const project = this.homeOf(item);
      counts.set(project, (counts.get(project) ?? 0) + 1);
    }
    return [...counts]
      .map(([project, count]) => ({ project, count }))
      .toSorted(
        (a, b) =>
          Number(a.project === null) - Number(b.project === null) ||
          (a.project ?? '').localeCompare(b.project ?? ''),
      );
  }

  /** Whether anything waits on the operator. */
  get pending(): boolean {
    return (
      this.approvals.length > 0 ||
      this.toPromote.length > 0 ||
      this.previews.length > 0 ||
      this.queued.length > 0
    );
  }

  /** The name of the project with `id`; null for none, or one not listed. */
  /** The name of the project `item` belongs to: its own, or
   * Ren's for one from before projects. */
  homeOf(item: WorkItem): string | null {
    return Work.home(item, this.projects)?.name ?? null;
  }

  /** How many current sessions work for no project: Scratch's. */
  get scratch(): number {
    return Sessions.within(this.sessions, { kind: 'scratch', projects: this.projects }).length;
  }

  nameOf(id: string | null): string | null {
    if (id === null) return null;
    return this.projects.find((project) => project.id === id)?.name ?? null;
  }

  /** `items` by state, open ones first; a state with none is left out. */
  static groups(items: readonly WorkItem[]): WorkGroup[] {
    const states = [
      ['queued', 'Queued', 'marked, not sent yet'],
      ['sent', 'Sent', 'an agent is on it'],
      ['landed', 'Landed', 'on main, not in production'],
      ['shipped', 'Shipped', 'in production'],
      ['dismissed', 'Dismissed', 'left undone'],
    ] as const;
    return states
      .map(([state, label, note]) => ({
        state,
        label,
        note,
        items: items.filter((item) => item.state === state),
      }))
      .filter((group) => group.items.length > 0);
  }

  /** Where a project's page is. */
  static href(name: string): string {
    return `/projects/${encodeURIComponent(name)}`;
  }

  /** Where a project's release is shown in full: its Release page. */
  static releaseHref(project: Project): string {
    return `${ProjectBoard.href(project.name)}/release`;
  }

  /** A project's open work, in words: what is queued, sent, and landed. */
  static openWork(project: Project): string {
    const { queued, sent, landed } = project.work;
    const parts = [
      queued ? `${queued} queued` : null,
      sent ? `${sent} sent` : null,
      landed ? `${landed} landed` : null,
    ].filter((part) => part !== null);
    return parts.length ? parts.join(' · ') : 'none open';
  }
}
