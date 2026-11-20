// The command palette's pure logic (⌘K): every place the UI has, each
// project's own pages, and a search of recorded history, narrowed by what
// the operator types. Pure: no DOM.

import { Views, type Destination } from './view.ts';

/** One thing the palette offers: where it goes, and how it is said. */
export interface Command {
  kind: 'go' | 'search';
  label: string;
  /** Where it sits: its destination, or its project. */
  detail: string;
  href: string;
}

const DESTINATIONS: readonly Destination[] = ['projects', 'inspect', 'host'];

export class Commands {
  /** Every page: each destination's, then each project's own. */
  static all(projects: readonly { name: string }[]): Command[] {
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
    return [...places, ...own];
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
