// Where the Work and Sessions pages' old addresses now lead: each work item
// and session lives under its project (`/projects/<name>/work`,
// `/projects/<name>/sessions`), or, for a session of no project, in
// Scratch. Read here with the browser's login, as the pages would.
import { ManagementApi } from '#lib/api.ts';
import { RunId, RunsClient, SessionId, Sessions } from '#lib/runs.ts';
import { Work, WorkApi, WorkItemId } from '#lib/work.ts';
import { RunsStartRead } from './runsStart.ts';

export class OldAddress {
  /** Where `/sessions?session=` or `?run=` is now; Projects for anything
   * else, or a session that does not read. */
  static async sessions(request: Request, url: URL): Promise<string> {
    const fetcher = RunsStartRead.fetcher(request);
    const runs = new RunsClient(fetcher);
    const session = SessionId.parse(url.searchParams.get('session'));
    const run = RunId.parse(url.searchParams.get('run'));
    try {
      const body = session
        ? await runs.session(session)
        : run
          ? [...(await runs.sessions()), ...(await runs.sessions('archived'))].find(
              (entry) => Sessions.of([entry], run) !== null,
            )
          : undefined;
      if (!body) return '/';
      const projects = await new ManagementApi(fetcher).projects();
      const query: Record<string, string> = run ? { run } : { session: body.session };
      return Sessions.deskHref(Sessions.projectOf(body, projects)?.name ?? null, query);
    } catch {
      return '/';
    }
  }

  /** Where `/work?item=` is now; Projects for anything else, or an item
   * that does not read. */
  static async work(request: Request, url: URL): Promise<string> {
    const fetcher = RunsStartRead.fetcher(request);
    const id = WorkItemId.parse(url.searchParams.get('item'));
    if (!id) return '/';
    try {
      const [item, projects] = await Promise.all([
        new WorkApi(fetcher).item(id),
        new ManagementApi(fetcher).projects(),
      ]);
      const home = Work.home(item, projects);
      if (!home) return '/';
      const archived = item.state === 'shipped' || item.state === 'dismissed';
      return Work.href(home.name, archived ? { shelf: 'archived', item: id } : { item: id });
    } catch {
      return '/';
    }
  }
}
