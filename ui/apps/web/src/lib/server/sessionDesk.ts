import { Memory, type RunsStart } from '#lib/runs.ts';
import { RunsStartRead } from '#lib/server/runsStart.ts';
import { Arrival } from '#lib/arrival.ts';
import type { ServerLoadEvent } from '@sveltejs/kit';

/**
 * What this server read for the page: everything it starts with, nothing
 * because runs are off here, or nothing yet, on a navigation in the
 * browser whose read was not in within the grace period, where the page
 * reads it itself and draws its outline meanwhile.
 */
export type Opening = { kind: 'read'; start: RunsStart } | { kind: 'off' } | { kind: 'unread' };

class OpeningRead {
  /** On a navigation: what was read within the grace period, else nothing
   * yet, and the page reads it itself while it draws its outline. */
  static async within(read: Promise<RunsStart>): Promise<Opening> {
    const within = await Arrival.within(read, Arrival.GRACE_MS);
    switch (within.state) {
      case 'in':
        return { kind: 'read', start: within.value };
      case 'late':
        return { kind: 'unread' };
      default: {
        const never: never = within;
        return never;
      }
    }
  }
}

/** What a session desk's server reads, wherever the desk is. */
export type DeskServerData = {
  opening: Opening;
  /** The choices last started with, as this browser kept them. */
  remembered: string | null;
};

/**
 * A session desk's server read: a first load renders with what runs,
 * places, and clients are now; the feed keeps it current from there. The
 * URL is not read here, so choosing a run does not read everything again.
 */
export class SessionDeskServer {
  static async load({
    request,
    parent,
    cookies,
    isDataRequest,
  }: ServerLoadEvent<Record<string, string>, Record<string, unknown>>): Promise<DeskServerData> {
    const { runsControl } = (await parent()) as { runsControl: boolean };
    const remembered = cookies.get(Memory.COOKIE) ?? null;
    const opening: Opening = !runsControl
      ? { kind: 'off' }
      : isDataRequest
        ? await OpeningRead.within(RunsStartRead.read(request))
        : { kind: 'read', start: await RunsStartRead.read(request) };
    return { opening, remembered };
  }
}
