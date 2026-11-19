/**
 * The workbench's views: which one a route is, where it sits in the tree
 * (docs/plans/hierarchy.md), and its title. Pure, so the layout's choices
 * are tested without a browser.
 *
 * Three destinations hold every view: Projects (the factory: each
 * project, and the sessions, work, and releases under them), Inspect
 * (everything recorded), and Host (this machine). The header names the
 * three; under it, the current destination's own pages.
 */

/** A view of the workbench, one per page. */
export type View =
  | 'projects'
  | 'project'
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

/** A page of a destination: a project's page sits under Projects, and a
 * recording's report and turns under Recordings. */
export type Tab =
  | 'projects'
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
      case '/overview':
        return 'overview';
      case '/live':
        return 'live';
      case '/sessions':
        return 'sessions';
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
      case '/work':
        return 'work';
      case '/release':
        return 'release';
      case '/recorder':
        return 'recorder';
      default:
        return 'traffic';
    }
  }

  /** The page `view` is shown under. */
  static tab(view: View): Tab {
    switch (view) {
      case 'project':
        return 'projects';
      case 'recording':
      case 'report':
      case 'turn':
        return 'recordings';
      case 'projects':
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

  /** A destination's pages, in the order its row shows them. */
  static pages(destination: Destination): readonly PageLink[] {
    switch (destination) {
      case 'projects':
        return [
          { tab: 'projects', href: '/', label: 'Projects' },
          { tab: 'work', href: '/work', label: 'Work' },
          { tab: 'sessions', href: '/sessions', label: 'Sessions' },
          { tab: 'release', href: '/release', label: 'Release' },
        ];
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

  /** What the toolbar and the document title call `view`. */
  static title(view: View): string {
    switch (view) {
      case 'projects':
        return 'Projects';
      case 'project':
        return 'Project';
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
