// The command palette's pure logic (⌘K): what the operator can do from
// anywhere (start a session in a project, promote a canary that waits),
// every place the UI has, each project's own pages and work items, and a
// search of recorded history, narrowed by what the operator types. Pure:
// no DOM.

import { Views, type Destination } from './view.ts';

/** One thing the palette offers: where it goes, and how it is said. */
export interface Command {
  /** Go to a place, do something there, or search recorded history. */
  kind: 'go' | 'act' | 'search';
  label: string;
  /** Where it sits: its destination, or its project. */
  detail: string;
  href: string;
}

const DESTINATIONS: readonly Destination[] = ['projects', 'inspect', 'host'];

/** What the palette can offer besides places: canaries waiting to be
 * promoted from here, and work items under way. */
export interface CommandSources {
  promotable: readonly { project: string; commit: string }[];
  work: readonly { title: string; project: string | null; href: string | null }[];
}

const NONE: CommandSources = { promotable: [], work: [] };

export class Commands {
  /** Every command: the actions first, then each destination's pages, each
   * project's own, and its work items. */
  static all(projects: readonly { name: string }[], sources: CommandSources = NONE): Command[] {
    const label = (destination: Destination) =>
      Views.destinations.find((place) => place.destination === destination)?.label ?? destination;
    const places = DESTINATIONS.flatMap((destination) =>
      Views.pages(destination).map((link): Command => ({
        kind: 'go',
        label: link.label,
        detail: label(destination),
        href: link.href,
      })),
    );
    const own = projects.flatMap((project) =>
      Views.pages('projects', project.name).map((link): Command => ({
        kind: 'go',
        label: link.label,
        detail: project.name,
        href: link.href,
      })),
    );
    const actions: Command[] = [
      ...sources.promotable.map(({ project, commit }): Command => ({
        kind: 'act',
        label: `Promote ${commit.slice(0, 12)} to production`,
        detail: project,
        href: `/projects/${encodeURIComponent(project)}/release?promote=${encodeURIComponent(commit)}`,
      })),
      ...projects.map((project): Command => ({
        kind: 'act',
        label: 'New session',
        detail: project.name,
        href: `/projects/${encodeURIComponent(project.name)}/sessions?new=1`,
      })),
    ];
    const work = sources.work.flatMap((item): Command[] =>
      item.href
        ? [{ kind: 'go', label: item.title, detail: item.project ?? 'Work', href: item.href }]
        : [],
    );
    return [...actions, ...places, ...own, ...work];
  }

  /**
   * The commands every word of `query` appears in (label or detail, any
   * case), in their order; with a query, a search of recorded history for
   * it comes last.
   */
  static match(commands: readonly Command[], query: string): Command[] {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return [...commands];
    const found = commands.filter((command) => {
      const said = `${command.label} ${command.detail}`.toLowerCase();
      return words.every((word) => said.includes(word));
    });
    const trimmed = query.trim();
    return [
      ...found,
      {
        kind: 'search',
        label: `Search recorded history for “${trimmed}”`,
        detail: 'Search',
        href: `/search?q=${encodeURIComponent(trimmed)}`,
      },
    ];
  }
}
