// The Projects view's board, pure (docs/plans/hierarchy.md): the
// registered projects with what is under way for each, and what waits on
// the operator across them: a canary to promote, a preview to check, work
// still queued. Read from `/api/projects` and `/api/work`.
import type { Project } from './api.ts';
import type { WorkItem } from './work.ts';

/** A queued work item's project, by name; null when it names none. */
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

  constructor(projects: readonly Project[], work: readonly WorkItem[]) {
    this.projects = projects;
    this.work = work;
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
      const project = this.nameOf(item.project);
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
    return this.toPromote.length > 0 || this.previews.length > 0 || this.queued.length > 0;
  }

  /** The name of the project with `id`; null for none, or one not listed. */
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

  /** Where a project's release is shown in full: Ren's has its own view. */
  static releaseHref(project: Project): string {
    return project.factory === 'ren' ? '/release' : `${ProjectBoard.href(project.name)}#release`;
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
