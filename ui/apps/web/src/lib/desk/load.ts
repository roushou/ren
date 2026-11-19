import type { LoadEvent } from '@sveltejs/kit';
import { Arrival, type Arriving } from '#lib/arrival.ts';
import { RunId, RunsClient, SessionId, Sessions, type StoredConversation } from '#lib/runs.ts';
import type { DeskServerData } from '#lib/server/sessionDesk.ts';

/** The selected session's newest run's conversation, as the page receives it. */
export interface GivenConversation {
  run: RunId;
  conversation: Arriving<StoredConversation>;
}

/** What a session desk renders with, wherever it is. */
export type DeskData = DeskServerData & {
  runsControl: boolean;
  /** The session the address names. */
  session: SessionId | null;
  conversation: GivenConversation | null;
};

/**
 * A session desk's load: the server's first reads come through; the
 * selected session's newest run's conversation is read here too: in full
 * for a first load, so the page renders with it, and handed over on its way
 * on a navigation in the browser, so the page appears at once with its
 * outline. A link to a run (`?run=`) shows the session it served.
 */
export class SessionDeskLoad {
  static async load({
    url,
    parent,
    data,
    fetch,
  }: LoadEvent<Record<string, string>, DeskServerData>): Promise<DeskData> {
    const { runsControl } = (await parent()) as { runsControl: boolean };
    const start = data.opening.kind === 'read' ? data.opening.start : null;
    const bodies = [
      ...(start?.sessions.state === 'read' ? start.sessions.value : []),
      ...(start?.archivedSessions.state === 'read' ? start.archivedSessions.value : []),
    ];
    const linkedRun = RunId.parse(url.searchParams.get('run'));
    const session =
      SessionId.parse(url.searchParams.get('session')) ??
      (linkedRun ? Sessions.of(bodies, linkedRun) : null);
    const body = session ? bodies.find((entry) => entry.session === session) : undefined;
    const run = body ? (body.served?.process ?? body.stored?.runs[0]?.id ?? null) : null;
    // A failed read is handed over too: the page reads it again, and says why
    // when it cannot.
    const conversation: GivenConversation | null = run
      ? {
          run,
          conversation: await Arrival.read(`sessions:conversation:${run}`, () =>
            new RunsClient(fetch).conversation(run),
          ),
        }
      : null;
    return { ...data, runsControl, session, conversation };
  }
}
