/**
 * The workbench's views: which one a route is, where it sits in the tree
 * (docs/plans/hierarchy.md), and its title. Pure, so the layout's choices
 * are tested without a browser.
 *
 * Three destinations hold every view: Projects (the factory: each
 * project, with its work, sessions, and release under it, and Scratch for
 * sessions that work for none), Inspect (everything recorded), and
 * Host (this machine). The header names the three; under it, the current
 * destination's own pages, or inside a project, the project's.
 */

/** A view of the workbench, one per page. */
export type View =
  | 'projects'
  | 'project'
  | 'scratch'
  | 'overview'
  | 'live'
  | 'sessions'
  | 'traffic'
  | 'recordings'
  | 'recording'
  | 'report'
  | 'turn'
  | 'search'
  | 'usage'
  | 'tools'
  | 'work'
  | 'release'
  | 'recorder';

/** A page of a destination, or of a project: a recording's report and
 * turns sit under Recordings. */
export type Tab =
  | 'projects'
  | 'project'
  | 'scratch'
  | 'sessions'
  | 'work'
  | 'release'
  | 'overview'
  | 'live'
  | 'traffic'
  | 'recordings'
  | 'search'
  | 'tools'
  | 'usage'
  | 'recorder';

/** The header's three places. */
export type Destination = 'projects' | 'inspect' | 'host';

/** A destination's page, as its row of links shows it. */
export interface PageLink {
  tab: Tab;
  href: string;
  label: string;
}

export class Views {
  /** The destinations, in the header's order, each with where it opens. */
  static readonly destinations: readonly {
    destination: Destination;
    href: string;
    label: string;
  }[] = [
    { destination: 'projects', href: '/', label: 'Projects' },
    { destination: 'inspect', href: '/overview', label: 'Inspect' },
    { destination: 'host', href: '/tools', label: 'Host' },
  ];

  /** The view a SvelteKit route id is; an unknown route is Traffic, the
   * workbench's own view. */
  static of(route: string | null): View {
    switch (route) {
      case '/':
        return 'projects';
      case '/projects/[name]':
        return 'project';
      case '/projects/[name]/work':
        return 'work';
      case '/projects/[name]/sessions':
        return 'sessions';
      case '/projects/[name]/release':
        return 'release';
      case '/scratch':
        return 'scratch';
      case '/overview':
        return 'overview';
      case '/live':
        return 'live';
      case '/recordings':
        return 'recordings';
      case '/recordings/[id]':
        return 'recording';
      case '/recordings/[id]/report':
        return 'report';
      case '/turns/[id]':
        return 'turn';
      case '/search':
        return 'search';
      case '/usage':
        return 'usage';
      case '/tools':
        return 'tools';
      case '/recorder':
        return 'recorder';
      default:
        return 'traffic';
    }
  }

  /** The page `view` is shown under. */
  static tab(view: View): Tab {
    switch (view) {
      case 'recording':
      case 'report':
      case 'turn':
        return 'recordings';
      case 'projects':
      case 'project':
      case 'scratch':
      case 'overview':
      case 'live':
      case 'sessions':
      case 'traffic':
      case 'recordings':
      case 'search':
      case 'usage':
      case 'tools':
      case 'work':
      case 'release':
      case 'recorder':
        return view;
      default: {
        const never: never = view;
        return never;
      }
    }
  }

  /** The destination `tab` belongs to. */
  static destination(tab: Tab): Destination {
    switch (tab) {
      case 'projects':
      case 'project':
      case 'scratch':
      case 'sessions':
      case 'work':
      case 'release':
        return 'projects';
      case 'overview':
      case 'live':
      case 'traffic':
      case 'recordings':
      case 'search':
        return 'inspect';
      case 'tools':
      case 'usage':
      case 'recorder':
        return 'host';
      default: {
        const never: never = tab;
        return never;
      }
    }
  }

  /** A destination's pages, in the order its row shows them; inside the
   * project named `project`, the project's own. */
  static pages(destination: Destination, project: string | null = null): readonly PageLink[] {
    switch (destination) {
      case 'projects': {
        if (project !== null) {
          const base = `/projects/${encodeURIComponent(project)}`;
          return [
            { tab: 'project', href: base, label: 'Overview' },
            { tab: 'work', href: `${base}/work`, label: 'Work' },
            { tab: 'sessions', href: `${base}/sessions`, label: 'Sessions' },
            { tab: 'release', href: `${base}/release`, label: 'Release' },
          ];
        }
        return [
          { tab: 'projects', href: '/', label: 'Projects' },
          { tab: 'scratch', href: '/scratch', label: 'Scratch' },
        ];
      }
      case 'inspect':
        return [
          { tab: 'overview', href: '/overview', label: 'Overview' },
          { tab: 'live', href: '/live', label: 'Live' },
          { tab: 'traffic', href: '/traffic', label: 'Traffic' },
          { tab: 'recordings', href: '/recordings', label: 'Recordings' },
          { tab: 'search', href: '/search', label: 'Search' },
        ];
      case 'host':
        return [
          { tab: 'tools', href: '/tools', label: 'Tools' },
          { tab: 'usage', href: '/usage', label: 'Usage' },
          { tab: 'recorder', href: '/recorder', label: 'Recorder' },
        ];
      default: {
        const never: never = destination;
        return never;
      }
    }
  }

  /** Whether `tab` can be scoped to one project with `?project=`. */
  static scoped(tab: Tab): boolean {
    switch (tab) {
      case 'traffic':
      case 'recordings':
        return true;
      case 'projects':
      case 'project':
      case 'scratch':
      case 'sessions':
      case 'work':
      case 'release':
      case 'overview':
      case 'live':
      case 'search':
      case 'tools':
      case 'usage':
      case 'recorder':
        return false;
      default: {
        const never: never = tab;
        return never;
      }
    }
  }

  /** `link`'s address, carrying `project` when its page can be scoped. */
  static scopedHref(link: PageLink, project: string | null): string {
    return project && Views.scoped(link.tab)
      ? `${link.href}?${new URLSearchParams({ project })}`
      : link.href;
  }

  /** What the toolbar and the document title call `view`. */
  static title(view: View): string {
    switch (view) {
      case 'projects':
        return 'Projects';
      case 'project':
        return 'Project';
      case 'scratch':
        return 'Scratch';
      case 'overview':
        return 'Overview';
      case 'live':
        return 'Live';
      case 'sessions':
        return 'Sessions';
      case 'traffic':
        return 'Traffic';
      case 'recordings':
        return 'Recordings';
      case 'recording':
        return 'Recording activity';
      case 'report':
        return 'Recording report';
      case 'turn':
        return 'Turn';
      case 'search':
        return 'Search';
      case 'usage':
        return 'Usage';
      case 'tools':
        return 'Tools';
      case 'work':
        return 'Work';
      case 'release':
        return 'Release';
      case 'recorder':
        return 'Recorder & storage';
      default: {
        const never: never = view;
        return never;
      }
    }
  }
}
