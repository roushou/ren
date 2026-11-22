/**
 * The workbench's views: which one a route is, where it sits in the tree
 * (docs/plans/hierarchy.md), and its title. Pure, so the layout's choices
 * are tested without a browser.
 *
 * Home (`/`, under the wordmark) is where the operator lands: what needs
 * them, what is moving, what happened since they looked, everywhere. Three
 * places hold every other view, and never show each other: Work (each
 * project, with its work, sessions, and release under it, and Scratch for
 * sessions that work for none), Monitor (what agents and models did), and
 * System (the machine Ren runs on). Home's bar names the three; inside one,
 * the bar holds only that place's pages, or inside a project, the
 * project's.
 */

/** A view of the workbench, one per page. */
export type View =
  | 'home'
  | 'projects'
  | 'work'
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
  | 'release'
  | 'recorder';

/** A page of a place, or of a project: a recording's report and
 * turns sit under Recordings. */
export type Tab =
  | 'home'
  | 'projects'
  | 'scratch'
  | 'sessions'
  | 'release'
  | 'overview'
  | 'live'
  | 'traffic'
  | 'recordings'
  | 'search'
  | 'tools'
  | 'usage'
  | 'recorder';

/** Where a view sits: Home, or one of the three places. */
export type Place = 'home' | 'work' | 'monitor' | 'system';

/** A place's page, as its row of links shows it. */
export interface PageLink {
  tab: Tab;
  href: string;
  label: string;
}

export class Views {
  /** The three places, in Home's order, each with where it opens. */
  static readonly places: readonly {
    place: Exclude<Place, 'home'>;
    href: string;
    label: string;
  }[] = [
    { place: 'work', href: '/projects', label: 'Work' },
    { place: 'monitor', href: '/overview', label: 'Monitor' },
    { place: 'system', href: '/tools', label: 'System' },
  ];

  /** The view a SvelteKit route id is; an unknown route is Traffic, the
   * workbench's own view. */
  static of(route: string | null): View {
    switch (route) {
      case '/':
        return 'home';
      case '/projects':
        return 'projects';
      case '/projects/[name]/work/[id]':
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
      case 'work':
        return 'release';
      case 'home':
      case 'projects':
      case 'scratch':
      case 'overview':
      case 'live':
      case 'sessions':
      case 'traffic':
      case 'recordings':
      case 'search':
      case 'usage':
      case 'tools':
      case 'release':
      case 'recorder':
        return view;
      default: {
        const never: never = view;
        return never;
      }
    }
  }

  /** The place `tab` belongs to. */
  static place(tab: Tab): Place {
    switch (tab) {
      case 'home':
        return 'home';
      case 'projects':
      case 'scratch':
      case 'sessions':
      case 'release':
        return 'work';
      case 'overview':
      case 'live':
      case 'traffic':
      case 'recordings':
      case 'search':
      case 'usage':
        return 'monitor';
      case 'tools':
      case 'recorder':
        return 'system';
      default: {
        const never: never = tab;
        return never;
      }
    }
  }

  /** A place's pages, in the order its bar shows them; inside the project
   * named `project`, the project's own. Home and Work's own home
   * (Projects) have none: the place's name leads there. */
  static pages(place: Place, project: string | null = null): readonly PageLink[] {
    switch (place) {
      case 'home':
        return [];
      case 'work': {
        if (project === null) return [];
        const base = `/projects/${encodeURIComponent(project)}`;
        return [
          { tab: 'sessions', href: `${base}/sessions`, label: 'Sessions' },
          { tab: 'release', href: `${base}/release`, label: 'Release' },
        ];
      }
      case 'monitor':
        return [
          { tab: 'overview', href: '/overview', label: 'Overview' },
          { tab: 'live', href: '/live', label: 'Live' },
          { tab: 'traffic', href: '/traffic', label: 'Traffic' },
          { tab: 'recordings', href: '/recordings', label: 'Recordings' },
          { tab: 'search', href: '/search', label: 'Search' },
          { tab: 'usage', href: '/usage', label: 'Usage' },
        ];
      case 'system':
        return [
          { tab: 'tools', href: '/tools', label: 'Tools' },
          { tab: 'recorder', href: '/recorder', label: 'Recorder' },
        ];
      default: {
        const never: never = place;
        return never;
      }
    }
  }

  /** Whether `tab` can be scoped to one project with `?project=`: every
   * Monitor page that reads one. */
  static scoped(tab: Tab): boolean {
    switch (tab) {
      case 'traffic':
      case 'recordings':
      case 'overview':
      case 'live':
      case 'search':
        return true;
      case 'home':
      case 'projects':
      case 'scratch':
      case 'sessions':
      case 'release':
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
      case 'home':
        return 'Home';
      case 'projects':
        return 'Projects';
      case 'work':
        return 'Work item';
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
