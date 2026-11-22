// The command palette's pure logic (⌘K): what the operator can do from
// anywhere (start a session in a project, promote a canary that waits),
// every place the UI has, each project's own pages and work items, and a
// search of recorded history, narrowed by what the operator types. Pure:
// no DOM.

import { assertNever } from './decode.ts';
import { Views, type Destination } from './view.ts';

/** One thing the palette offers: where it goes, and how it is said. */
export interface Command {
  /** Do something, open a work item, a recorded call, go to a place, or
   * search recorded history: the palette's groups, in that order. */
  kind: CommandKind;
  label: string;
  /** Where it sits: its destination, or its project. */
  detail: string;
  href: string;
  /** The project it belongs to; null for one that belongs to none. */
  project: string | null;
}

export const commandKinds = ['act', 'work', 'call', 'go', 'search'] as const;
export type CommandKind = (typeof commandKinds)[number];

/** The palette's commands of one kind, under its heading. */
export interface CommandGroup {
  kind: CommandKind;
  label: string;
  commands: Command[];
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
        project: null,
      })),
    );
    const own = projects.flatMap((project) =>
      Views.pages('projects', project.name).map((link): Command => ({
        kind: 'go',
        label: link.label,
        detail: project.name,
        href: link.href,
        project: project.name,
      })),
    );
    const actions: Command[] = [
      ...sources.promotable.map(({ project, commit }): Command => ({
        kind: 'act',
        label: `Promote ${commit.slice(0, 12)} to production`,
        detail: project,
        href: `/projects/${encodeURIComponent(project)}/release?promote=${encodeURIComponent(commit)}`,
        project,
      })),
      ...projects.map((project): Command => ({
        kind: 'act',
        label: 'New session',
        detail: project.name,
        href: `/projects/${encodeURIComponent(project.name)}/sessions?new=1`,
        project: project.name,
      })),
    ];
    const work = sources.work.flatMap((item): Command[] =>
      item.href
        ? [
            {
              kind: 'work',
              label: item.title,
              detail: item.project ?? 'Work',
              href: item.href,
              project: item.project,
            },
          ]
        : [],
    );
    const desk: Command = { kind: 'go', label: 'Desk', detail: 'Ren', href: '/', project: null };
    // Scratch is picked like a project, so it is no page of a row.
    const scratch: Command = {
      kind: 'go',
      label: 'Scratch',
      detail: 'Sessions for no project',
      href: '/scratch',
      project: null,
    };
    return [...actions, desk, ...places, scratch, ...own, ...work];
  }

  /**
   * The commands every word of `query` appears in (label or detail, any
   * case), in their order; inside a project (`here`), only its own and
   * those of none. With a query, a search of recorded history comes last.
   */
  static match(commands: readonly Command[], query: string, here: string | null = null): Command[] {
    const words = Commands.#words(query);
    const scoped = commands.filter(
      (command) => here === null || command.project === null || command.project === here,
    );
    if (!words.length) return scoped;
    const found = scoped.filter((command) => {
      const said = `${command.label} ${command.detail}`.toLowerCase();
      return words.every((word) => said.includes(word));
    });
    const trimmed = query.trim();
    const search = new URLSearchParams({ q: trimmed });
    if (here) search.set('project', here);
    return [
      ...found,
      {
        kind: 'search',
        label: `Search recorded history for “${trimmed}”`,
        detail: here ?? 'Search',
        href: `/search?${search}`,
        project: here,
      },
    ];
  }

  /** `commands` under their kinds' headings, in the palette's order. */
  static groups(commands: readonly Command[]): CommandGroup[] {
    return commandKinds.flatMap((kind) => {
      const own = commands.filter((command) => command.kind === kind);
      return own.length ? [{ kind, label: Commands.heading(kind), commands: own }] : [];
    });
  }

  static heading(kind: CommandKind): string {
    switch (kind) {
      case 'act':
        return 'Actions';
      case 'work':
        return 'Work items';
      case 'call':
        return 'Calls';
      case 'go':
        return 'Go to';
      case 'search':
        return 'Search';
      default:
        return assertNever(kind);
    }
  }

  /** `text` in parts, each word of `query` it contains marked, for the
   * palette to set the matched letters apart. */
  static marks(text: string, query: string): { text: string; match: boolean }[] {
    const words = Commands.#words(query);
    if (!words.length) return [{ text, match: false }];
    const lower = text.toLowerCase();
    const hit = Array.from({ length: text.length }, () => false);
    for (const word of words) {
      for (let at = lower.indexOf(word); at !== -1; at = lower.indexOf(word, at + word.length))
        for (let index = at; index < at + word.length; index++) hit[index] = true;
    }
    const parts: { text: string; match: boolean }[] = [];
    for (let index = 0; index < text.length; index++) {
      const last = parts.at(-1);
      if (last && last.match === hit[index]) last.text += text[index];
      else parts.push({ text: text[index]!, match: hit[index]! });
    }
    return parts;
  }

  static #words(query: string): string[] {
    return query.toLowerCase().split(/\s+/).filter(Boolean);
  }
}
