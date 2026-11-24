// Where a session's relayed runs reached through the relay
// (`/api/sessions/{id}/passages`), and the session inspector's list of it:
// each place its runs asked for, with how those requests ended. Pure: decoding and grouping only, tested in
// tests/passages.test.ts.
import {
  array,
  assertNever,
  brandedCount,
  count,
  label,
  nullable,
  object,
  string,
} from './decode.ts';
import type { Stored } from './decode.ts';
import type { Mark } from './marks.ts';
import type { PassageView as WirePassageView } from './wire/PassageView.ts';
import type { Passages as WirePassages } from './wire/Passages.ts';

/** How a request to the relay ended (`ren-recorder passage::PassageEnd`). */
export const PASSAGE_ENDS = [
  'closed',
  'broken',
  'unlisted',
  'private_address',
  'unresolved',
  'connect_refused',
  'timed_out',
  'not_a_tunnel',
] as const;
export type PassageEnd = (typeof PASSAGE_ENDS)[number];

/** Why a destination is listed (`ren-recorder passage::Purpose`). */
export const PURPOSES = ['configuration', 'client', 'project', 'grant'] as const;
export type Purpose = (typeof PURPOSES)[number];

/** One place a run asked for, and how those requests ended. */
export interface Passage {
  /** A tunnel's `host:port`, or the method of a request that was not one. */
  asked: string;
  end: Stored<PassageEnd>;
  /** The listed destination that let it through; null when refused. */
  through: string | null;
  /** Why that destination is listed; null when refused or not known. */
  listed_by: Stored<Purpose> | null;
  count: number;
  sent: number;
  received: number;
  first_at: number;
  last_at: number;
}

export const passages = object<{ passages: Passage[] }, WirePassages>({
  passages: array(
    object<Passage, WirePassageView>({
      asked: string,
      end: label(PASSAGE_ENDS),
      through: nullable(string),
      listed_by: nullable(label(PURPOSES)),
      count,
      sent: count,
      received: count,
      first_at: brandedCount<'UnixMillis'>(),
      last_at: brandedCount<'UnixMillis'>(),
    }),
  ),
});

/** One row of the inspector's list: a place, and how its requests ended. */
export interface ReachRow {
  asked: string;
  end: Stored<PassageEnd>;
  mark: Mark;
  /** The glyph's word: how it ended, and through what. */
  said: string;
  count: number;
  last_at: number;
}

export class Reached {
  /** A session's passages as its list: what was refused first, then the
   * most recent. The server groups them, by place and end, across runs. */
  static rows(passages: readonly Passage[]): ReachRow[] {
    return passages
      .map((passage) => ({
        asked: passage.asked,
        end: passage.end,
        mark: Reached.mark(passage.end),
        said: Reached.said(passage),
        count: passage.count,
        last_at: passage.last_at,
      }))
      .sort(
        (a, b) =>
          Number(Reached.refused(b.end)) - Number(Reached.refused(a.end)) ||
          b.last_at - a.last_at ||
          a.asked.localeCompare(b.asked),
      );
  }

  /** Whether the relay refused it: a place the operator can list. */
  static refused(end: Stored<PassageEnd>): boolean {
    const known = Reached.known(end);
    if (known === null) return false;
    switch (known) {
      case 'unlisted':
      case 'private_address':
      case 'not_a_tunnel':
        return true;
      case 'closed':
      case 'broken':
      case 'unresolved':
      case 'connect_refused':
      case 'timed_out':
        return false;
      default:
        return assertNever(known);
    }
  }

  static mark(end: Stored<PassageEnd>): Mark {
    const known = Reached.known(end);
    if (known === null) return 'unknown';
    switch (known) {
      case 'closed':
        return 'passed';
      case 'unlisted':
      case 'private_address':
      case 'not_a_tunnel':
        return 'failed';
      case 'broken':
      case 'unresolved':
      case 'connect_refused':
      case 'timed_out':
        return 'warning';
      default:
        return assertNever(known);
    }
  }

  static said(passage: Passage): string {
    const known = Reached.known(passage.end);
    const listed =
      passage.listed_by === null ? '' : `, listed by ${Reached.list(passage.listed_by)}`;
    const through = passage.through === null ? '' : ` through ${passage.through}${listed}`;
    if (known === null) return passage.end;
    switch (known) {
      case 'closed':
        return `Reached${through}`;
      case 'broken':
        return `Broke off${through}`;
      case 'unlisted':
        return 'Refused: not listed';
      case 'private_address':
        return 'Refused: a private address';
      case 'not_a_tunnel':
        return 'Refused: not a tunnel';
      case 'unresolved':
        return `Not found${through}`;
      case 'connect_refused':
        return `Connection refused${through}`;
      case 'timed_out':
        return `Timed out${through}`;
      default:
        return assertNever(known);
    }
  }

  /** The list a destination is on, as a run's page names it. */
  static list(purpose: Stored<Purpose>): string {
    const known = (PURPOSES as readonly string[]).includes(purpose) ? (purpose as Purpose) : null;
    if (known === null) return purpose;
    switch (known) {
      case 'configuration':
        return "Ren's configuration";
      case 'client':
        return 'its client';
      case 'project':
        return 'its project';
      case 'grant':
        return 'its grant';
      default:
        return assertNever(known);
    }
  }

  static known(end: Stored<PassageEnd>): PassageEnd | null {
    return (PASSAGE_ENDS as readonly string[]).includes(end) ? (end as PassageEnd) : null;
  }
}
