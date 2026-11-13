import { RunId, RunsClient, SessionId, Sessions, type StoredConversation } from '#lib/runs.ts';
import type { PageLoad } from './$types';

// The server's first reads come through; the selected session's newest
// run's conversation is read here too, so the page renders with it. A link
// to a run (`?run=`) shows the session it served.
export const load: PageLoad = async ({ url, parent, data, fetch }) => {
  const { runsControl } = await parent();
  const bodies = [
    ...(data.initial?.sessions.state === 'read' ? data.initial.sessions.value : []),
    ...(data.initial?.archivedSessions.state === 'read' ? data.initial.archivedSessions.value : []),
  ];
  const linkedRun = RunId.parse(url.searchParams.get('run'));
  const session =
    SessionId.parse(url.searchParams.get('session')) ??
    (linkedRun ? Sessions.of(bodies, linkedRun) : null);
  const body = session ? bodies.find((entry) => entry.session === session) : undefined;
  const run = body ? (body.served?.process ?? body.stored?.runs[0]?.id ?? null) : null;
  let conversation: { run: RunId; conversation: StoredConversation } | null = null;
  if (run) {
    try {
      conversation = { run, conversation: await new RunsClient(fetch).conversation(run) };
    } catch {
      // The page reads it again, and says why when it cannot.
    }
  }
  return { ...data, runsControl, session, conversation };
};
