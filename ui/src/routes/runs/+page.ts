import { RunId, RunsClient, type StoredConversation } from '#lib/runs.ts';
import type { PageLoad } from './$types';

// The server's first reads come through; the selected run's conversation is
// read here too, so the page renders with it. The feed follows from there.
export const load: PageLoad = async ({ url, parent, data, fetch }) => {
  const { runsControl } = await parent();
  const selected = RunId.parse(url.searchParams.get('run'));
  let conversation: { run: RunId; conversation: StoredConversation } | null = null;
  if (selected) {
    try {
      conversation = {
        run: selected,
        conversation: await new RunsClient(fetch).conversation(selected),
      };
    } catch {
      // The page reads it again, and says why when it cannot.
    }
  }
  return { ...data, runsControl, selected, conversation };
};
