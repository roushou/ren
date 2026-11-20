// The steps of a conversation, read for the folded transcript: each command,
// tool, and file change as what it did, a verb and its targets, taken from
// what its client said wherever it said something (Claude Code's and
// OpenCode's description of a command, Codex's parts of one, a tool's
// effects) and from the command itself only where it said nothing. The steps
// between two things said gather into one group, folded to a line. Pure: no
// DOM, no fetches.

import type { EffectKind } from './api.ts';
import { Board } from './board.ts';
import { assertNever } from './decode.ts';
import type { Stored } from './decode.ts';
import { Runs, type CommandPart, type Entry } from './runs.ts';

/** An entry that is a step: something the agent did, not something said. */
export type StepEntry = Extract<Entry, { kind: 'command' | 'file_change' | 'tool' }>;

/** What a step did. */
export type Act = 'read' | 'search' | 'list' | 'edit' | 'run' | 'delegate' | 'fetch' | 'tool';

/** How a step stands. */
export type StepState = 'running' | 'done' | 'failed' | 'declined';

/** How a group of steps stands: failed if one failed, running while one runs. */
export type GroupState = 'running' | 'done' | 'failed';

export interface Step {
  entry: StepEntry;
  act: Act;
  /** What it acted on: paths as read from the session's folder, a pattern,
   * a command, or a tool's name. */
  targets: string[];
  /** The client's own words for it, when it gave some. */
  said: string | null;
  state: StepState;
  /** How it failed, when it did: its exit code, or that it failed. */
  failure: string | null;
}

/** What a turn shows: something said as it is, or a group of steps. */
export type Block =
  | { kind: 'entry'; entry: Exclude<Entry, StepEntry> }
  | { kind: 'steps'; key: string; steps: Step[]; state: GroupState };

export class Steps {
  /** The verb a step's row starts with. */
  static readonly VERBS: Record<Act, string> = {
    read: 'Read',
    search: 'Search',
    list: 'List',
    edit: 'Edit',
    run: 'Run',
    delegate: 'Delegate',
    fetch: 'Fetch',
    tool: 'Tool',
  };

  static isStep(entry: Entry): entry is StepEntry {
    return entry.kind === 'command' || entry.kind === 'file_change' || entry.kind === 'tool';
  }

  /**
   * A turn's entries as blocks: what was said as it is, and each run of
   * steps between two things said as one group, keyed by its first step.
   * Thinking that finished shows nothing, so it parts no group.
   */
  static blocks(entries: readonly Entry[], workdir: string | null): Block[] {
    const blocks: Block[] = [];
    for (const entry of entries) {
      if (entry.kind === 'reasoning' && entry.done) continue;
      if (!Steps.isStep(entry)) {
        blocks.push({ kind: 'entry', entry });
        continue;
      }
      const step = Steps.read(entry, workdir);
      const last = blocks.at(-1);
      if (last?.kind === 'steps') {
        last.steps.push(step);
        last.state = Steps.state(last.steps);
      } else {
        blocks.push({ kind: 'steps', key: entry.item, steps: [step], state: Steps.state([step]) });
      }
    }
    return blocks;
  }

  static state(steps: readonly Step[]): GroupState {
    if (steps.some((step) => step.state === 'failed')) return 'failed';
    if (steps.some((step) => step.state === 'running')) return 'running';
    return 'done';
  }

  /** What one step did. */
  static read(entry: StepEntry, workdir: string | null): Step {
    switch (entry.kind) {
      case 'command':
        return Steps.#command(entry, workdir);
      case 'file_change':
        return {
          entry,
          act: 'edit',
          targets: entry.paths.map((path) => Runs.within(path, workdir)),
          said: null,
          ...Steps.#status(entry.status, null),
        };
      case 'tool': {
        const first = entry.effects[0];
        if (!first) {
          return {
            entry,
            act: 'tool',
            targets: [entry.name],
            said: null,
            state: entry.done ? 'done' : 'running',
            failure: null,
          };
        }
        const act = Steps.#effect(first.kind);
        return {
          entry,
          act,
          targets: entry.effects
            .filter((effect) => effect.kind === first.kind)
            .map((effect) => Steps.#target(act, effect.target, workdir)),
          said: null,
          state: entry.done ? 'done' : 'running',
          failure: null,
        };
      }
      default:
        return assertNever(entry);
    }
  }

  /**
   * A group's line: what its steps did, by kind, in the order each kind
   * first came. A file read or edited again counts once, with how many
   * times; more than two of a kind are counted instead of named.
   */
  static summary(steps: readonly Step[]): string {
    const acts: Act[] = [];
    for (const step of steps) if (!acts.includes(step.act)) acts.push(step.act);
    return acts
      .map((act) =>
        Steps.#phrase(
          act,
          steps.filter((step) => step.act === act),
        ),
      )
      .join(' · ');
  }

  static #phrase(act: Act, steps: readonly Step[]): string {
    const targets = steps.flatMap((step) => step.targets);
    switch (act) {
      case 'read':
        return Steps.#named('Read', targets, 'file', 'files');
      case 'edit':
        return Steps.#named('Edited', targets, 'file', 'files');
      case 'search':
        return steps.length === 1 && targets[0]
          ? `Searched ${Steps.#quoted(targets[0])}`
          : `Searched ${steps.length} times`;
      case 'list':
        return Steps.#named('Listed', targets, 'folder', 'folders');
      case 'run': {
        const only = steps.length === 1 ? steps[0] : undefined;
        if (only?.said) return only.said;
        return steps.length === 1 ? 'Ran 1 command' : `Ran ${steps.length} commands`;
      }
      case 'delegate':
        return steps.length === 1 && targets[0]
          ? `Delegated ${targets[0]}`
          : `Delegated ${steps.length} tasks`;
      case 'fetch':
        return Steps.#named('Fetched', targets, 'page', 'pages');
      case 'tool':
        return Steps.#named('Used', targets, 'tool', 'tools');
      default:
        return assertNever(act);
    }
  }

  /** `verb` and the distinct names among `targets`, each with how many times
   * it came when more than once; counted instead past two names. */
  static #named(verb: string, targets: readonly string[], one: string, many: string): string {
    const counts = new Map<string, number>();
    for (const target of targets) {
      const name = Steps.#name(target);
      counts.set(name, (counts.get(name) ?? 0) + 1);
    }
    if (counts.size === 0)
      return `${verb} ${targets.length === 1 ? `1 ${one}` : `${targets.length} ${many}`}`;
    if (counts.size > 2) return `${verb} ${counts.size} ${many}`;
    return `${verb} ${[...counts]
      .map(([name, count]) => (count > 1 ? `${name} ×${count}` : name))
      .join(', ')}`;
  }

  /** A path's last part; anything else as it is. */
  static #name(target: string): string {
    const trimmed = target.length > 1 ? target.replace(/\/+$/, '') : target;
    const slash = trimmed.lastIndexOf('/');
    return slash >= 0 && slash < trimmed.length - 1 ? trimmed.slice(slash + 1) : trimmed;
  }

  static #quoted(text: string): string {
    return `“${text.length > 40 ? `${text.slice(0, 39)}…` : text}”`;
  }

  static #command(entry: Extract<Entry, { kind: 'command' }>, workdir: string | null): Step {
    const status = Steps.#status(entry.status, entry.exit);
    const command = Board.shorten(entry.command, workdir);
    const intent = entry.intent;
    switch (intent.said) {
      case 'described':
        return { entry, act: 'run', targets: [command], said: intent.text, ...status };
      case 'parts': {
        const read = intent.parts.map((part) => Steps.#part(part, workdir));
        const first = read[0];
        // One act its client read in every part: that act on each part's
        // target. Anything else is the command as written.
        if (first && read.every((part) => part !== null && part.act === first?.act)) {
          return {
            entry,
            act: first.act,
            targets: read.flatMap((part) => (part ? [part.target] : [])),
            said: null,
            ...status,
          };
        }
        return { entry, act: 'run', targets: [command], said: null, ...status };
      }
      case 'unsaid':
        return { entry, act: 'run', targets: [command], said: null, ...status };
      default:
        return assertNever(intent);
    }
  }

  static #part(part: CommandPart, workdir: string | null): { act: Act; target: string } | null {
    switch (part.part) {
      case 'read':
        return { act: 'read', target: Runs.within(part.path, workdir) };
      case 'list_files':
        return { act: 'list', target: part.path ? Runs.within(part.path, workdir) : '.' };
      case 'search':
        return {
          act: 'search',
          target: part.query ?? (part.path ? Runs.within(part.path, workdir) : part.command),
        };
      case 'unknown':
        return null;
      default:
        return assertNever(part);
    }
  }

  static #effect(kind: Stored<EffectKind>): Act {
    switch (kind) {
      case 'read_file':
        return 'read';
      case 'write_file':
        return 'edit';
      case 'run_command':
        return 'run';
      case 'search':
        return 'search';
      case 'delegate':
        return 'delegate';
      case 'fetch':
        return 'fetch';
      default:
        return 'tool';
    }
  }

  static #target(act: Act, target: string, workdir: string | null): string {
    switch (act) {
      case 'read':
      case 'edit':
      case 'list':
        return Runs.within(target, workdir);
      case 'run':
        return Board.shorten(target, workdir);
      case 'search':
      case 'delegate':
      case 'fetch':
      case 'tool':
        return target;
      default:
        return assertNever(act);
    }
  }

  /** How a step with `status`, and an `exit` code when it has one, stands. */
  static #status(status: string, exit: number | null): Pick<Step, 'state' | 'failure'> {
    if (status === 'in_progress') return { state: 'running', failure: null };
    if (status === 'declined') return { state: 'declined', failure: null };
    if (exit !== null && exit !== 0) return { state: 'failed', failure: `exit ${exit}` };
    if (status === 'failed') return { state: 'failed', failure: 'failed' };
    return { state: 'done', failure: null };
  }
}
