// The Projects view's board, pure (docs/plans/hierarchy.md): the
// registered projects with what is under way for each, and what waits on
// the operator across them: a canary to promote, a preview to check, work
// still queued. Read from `/api/projects` and `/api/work`.
import type { Project } from './api.ts';
import {
  Runs,
  Sessions,
  type ApprovalAsk,
  type ApprovalId,
  type RunClient,
  type RunSnapshot,
  type SessionBody,
  type SessionId,
} from './runs.ts';
import { Work, type WorkItem } from './work.ts';
import type { Said } from './layer/toasts.svelte.ts';
import { SignIns } from './signIn.ts';
import type { Mark } from './marks.ts';

/** An approval a running session waits on, with the project it works for. */
export interface Approval {
  session: SessionId;
  approval: ApprovalId;
  ask: ApprovalAsk;
  /** The client that asks. */
  harness: RunSnapshot['harness'];
  /** The project's name; null when the session works for none. */
  project: string | null;
  /** The first line of the session's first prompt, to know it by. */
  prompt: string | null;
}

/** A project at a glance, as its row on Projects shows it. */
export interface ProjectGlance {
  /** Its state's glyph, and the words for it. */
  mark: Mark;
  state: string;
  /** The one thing to do about it, worst first; null when nothing is. */
  action: { kind: 'review'; approval: Approval } | { kind: 'promote' } | { kind: 'release' } | null;
  /** When one of its sessions last started or ended a run; null for none. */
  active: number | null;
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
  /** Each client and its sign-in on the daemon's host. */
  readonly clients: readonly RunClient[];

  constructor(
    projects: readonly Project[],
    work: readonly WorkItem[],
    live: readonly RunSnapshot[] = [],
    sessions: readonly SessionBody[] = [],
    only: string | null = null,
    clients: readonly RunClient[] = [],
  ) {
    this.projects = projects;
    this.work = work;
    this.live = live;
    this.sessions = sessions;
    this.only = only;
    this.clients = clients;
  }

  /** Clients signed out on the daemon's host: a run cannot use them until
   * the operator signs them in. */
  get signedOut(): RunClient[] {
    return this.clients.filter((client) => SignIns.signedOut(client.sign_in));
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
        harness: run.harness,
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

  /** How many things wait on the operator: what the top bar's Pending
   * counts. */
  get waiting(): number {
    return (
      this.approvals.length +
      this.toPromote.length +
      this.previews.length +
      this.queued.length +
      this.signedOut.length
    );
  }

  /**
   * What newly waits on the operator between two boards, said as toasts: an
   * approval a run started asking, a canary that newly passed. What stopped
   * waiting, and what waited already, says nothing.
   */
  static news(before: ProjectBoard, after: ProjectBoard): Said[] {
    const asked = new Set(before.approvals.map(ProjectBoard.#approvalKey));
    const approvals = after.approvals
      .filter((waiting) => !asked.has(ProjectBoard.#approvalKey(waiting)))
      .map((waiting): Said => {
        const said = Runs.ask(waiting.ask);
        return {
          mark: 'yours',
          text: said.action,
          detail: [waiting.project ?? 'No project', waiting.prompt]
            .filter((part) => part !== null)
            .join(' · '),
          link: {
            label: 'Open session',
            href: Sessions.deskHref(waiting.project, { session: waiting.session }),
          },
        };
      });
    const passed = new Set(before.toPromote.map(ProjectBoard.#canaryKey));
    const canaries = after.toPromote
      .filter((project) => !passed.has(ProjectBoard.#canaryKey(project)))
      .map((project): Said => {
        const release = project.release;
        const canary = release?.canary ?? null;
        const checks = canary?.checks ?? [];
        return {
          mark: 'passed',
          text: `Canary ${canary?.commit.slice(0, 12) ?? ''} ${checks.filter((check) => check.passed).length}/${checks.length}`,
          detail: `${project.name} · production ${release?.behind ? `↓${release.behind}` : 'up to date'}`,
          // Where Ren promotes, the toast asks it of Release's modal.
          link:
            project.factory === 'ren' && canary
              ? {
                  label: 'Promote…',
                  href: `${ProjectBoard.releaseHref(project)}?promote=${encodeURIComponent(canary.commit)}`,
                  emphasis: 'primary',
                }
              : { label: 'Release', href: ProjectBoard.releaseHref(project) },
        };
      });
    return [...approvals, ...canaries];
  }

  static #approvalKey(waiting: Approval): string {
    return `${waiting.session}:${waiting.approval}`;
  }

  /** A project's canary by its build: a newer one passing is news again. */
  static #canaryKey(project: Project): string {
    return `${project.id}:${project.release?.canary?.commit ?? ''}`;
  }

  /** `project` at a glance: waiting on the operator first, then a failed
   * canary, then running, else idle; its one action; when it was last
   * active. */
  glance(project: Project): ProjectGlance {
    const approval = this.approvals.find((waiting) => waiting.project === project.name) ?? null;
    const release = project.release;
    const failed = release?.decision === 'failed' || release?.decision === 'promotion_failed';
    const promotable = release?.decision === 'awaiting_operator';
    const [mark, state]: [Mark, string] = approval
      ? ['yours', 'Waits for you']
      : failed
        ? ['failed', 'Canary failed']
        : project.running
          ? ['responding', 'Running']
          : promotable
            ? ['yours', 'Ready to promote']
            : ['idle', 'Idle'];
    const action: ProjectGlance['action'] = approval
      ? { kind: 'review', approval }
      : promotable && project.factory === 'ren'
        ? { kind: 'promote' }
        : promotable || failed
          ? { kind: 'release' }
          : null;
    const times = this.sessions.flatMap((body) =>
      body.stored?.project === project.id
        ? body.stored.runs.flatMap((run) => [
            run.started_at,
            ...(run.ended_at ? [run.ended_at] : []),
          ])
        : [],
    );
    return { mark, state, action, active: times.length ? Math.max(...times) : null };
  }

  /** Whether anything waits on the operator. */
  get pending(): boolean {
    return (
      this.approvals.length > 0 ||
      this.toPromote.length > 0 ||
      this.previews.length > 0 ||
      this.queued.length > 0 ||
      this.signedOut.length > 0
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

  /** Where a project opens: its sessions. */
  static href(name: string): string {
    return `/projects/${encodeURIComponent(name)}/sessions`;
  }

  /** Where a project's release is shown in full: its Release page. */
  static releaseHref(project: Project): string {
    return `/projects/${encodeURIComponent(project.name)}/release`;
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
