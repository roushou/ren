/**
 * What the composer offers as the operator types: the run's client's
 * commands after `/` (or Codex's skills after `$`), and its folder's files
 * after `@`. Pure: the text and caret in, what to offer and what choosing
 * it writes out.
 */
import type { ClientCommand, FileMatch } from './runs.ts';

/** The word at the caret that asks for something to be offered. */
export type Mention =
  | {
      kind: 'command';
      /** `/` offers every command (only at the start of the prompt); `$` only `$` ones. */
      sigil: '/' | '$';
      query: string;
      /** Where the word starts and ends in the text: what choosing replaces. */
      start: number;
      end: number;
    }
  | { kind: 'file'; query: string; start: number; end: number };

/** What choosing an offer leaves: the new text, and where the caret goes. */
export interface Written {
  text: string;
  caret: number;
}

const space = /\s/;

export class Mentions {
  /** The most commands offered at once. */
  static readonly COMMANDS = 50;

  /** What the word at `caret` asks for, if anything. */
  static at(text: string, caret: number): Mention | null {
    if (caret < 0 || caret > text.length) return null;
    let start = caret;
    while (start > 0 && !space.test(text[start - 1]!)) start -= 1;
    let end = caret;
    while (end < text.length && !space.test(text[end]!)) end += 1;
    const typed = text.slice(start, caret);
    const query = typed.slice(1);
    switch (typed[0]) {
      case '@':
        return { kind: 'file', query, start, end };
      case '/':
        // A command opens the prompt; a path elsewhere is just a path.
        if (text.slice(0, start).trim() !== '' || query.includes('/')) return null;
        return { kind: 'command', sigil: '/', query, start, end };
      case '$':
        if (!/^[\w:.-]*$/.test(query)) return null;
        return { kind: 'command', sigil: '$', query, start, end };
      default:
        return null;
    }
  }

  /**
   * The commands to offer for `query`: names starting with it, then names
   * containing it, then descriptions mentioning it; by name within each.
   */
  static commands(
    commands: readonly ClientCommand[],
    mention: Extract<Mention, { kind: 'command' }>,
  ): ClientCommand[] {
    const query = mention.query.toLowerCase();
    const rank = (command: ClientCommand): number | null => {
      const name = command.name.toLowerCase();
      if (name.startsWith(query)) return 0;
      if (name.includes(query)) return 1;
      if (query !== '' && command.description.toLowerCase().includes(query)) return 2;
      return null;
    };
    return commands
      .filter((command) => mention.sigil === '/' || command.sigil === '$')
      .map((command) => ({ command, rank: rank(command) }))
      .filter((entry): entry is { command: ClientCommand; rank: number } => entry.rank !== null)
      .sort((a, b) => a.rank - b.rank || a.command.name.localeCompare(b.command.name))
      .slice(0, Mentions.COMMANDS)
      .map((entry) => entry.command);
  }

  /** The prompt with the mention replaced by `command`, ready for its arguments. */
  static command(text: string, mention: Mention, command: ClientCommand): Written {
    return Mentions.#replace(text, mention, `${command.invocation} `);
  }

  /** The prompt with the mention replaced by `@path`; a folder stays open to go into it. */
  static file(text: string, mention: Mention, file: FileMatch): Written {
    return Mentions.#replace(
      text,
      mention,
      file.kind === 'directory' ? `@${file.path}/` : `@${file.path} `,
    );
  }

  /** `path` cut into runs of matched and unmatched characters, for drawing the matches. */
  static marked(path: string, positions: readonly number[]): { text: string; hit: boolean }[] {
    const hits = new Set(positions);
    const parts: { text: string; hit: boolean }[] = [];
    Array.from(path).forEach((ch, index) => {
      const hit = hits.has(index);
      const last = parts[parts.length - 1];
      if (last && last.hit === hit) last.text += ch;
      else parts.push({ text: ch, hit });
    });
    return parts;
  }

  static #replace(text: string, mention: Mention, insert: string): Written {
    const after = text.slice(mention.end);
    // One space between the insert and what follows, not two.
    const joined = insert.endsWith(' ') && after.startsWith(' ') ? insert.slice(0, -1) : insert;
    return {
      text: text.slice(0, mention.start) + joined + after,
      caret: mention.start + joined.length + (joined === insert ? 0 : 1),
    };
  }
}
