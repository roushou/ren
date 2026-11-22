/**
 * The workbench's views: which one a route is, where it sits in the tree
 * (docs/plans/hierarchy.md), and its title. Pure, so the layout's choices
 * are tested without a browser.
 *
 * Home (`/`, under the wordmark) is where the operator lands: what needs
 * them, what is moving, what happened since they looked, everywhere. Three
 * places hold every other view, and never show each other: Projects (each
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
  | 'search'
  | 'usage'
  | 'packages'
  | 'github'
  | 'release'
  | 'recorder';

/** A page of a place, or of a project: a session's calls, report and
 * turns sit under Monitor's Sessions (`recordings`: the gateway's sessions,
 * as it records them). */
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
  | 'packages'
  | 'github'
  | 'usage'
  | 'recorder';

/** Where a view sits: Home, or one of the three places. */
export type Place = 'home' | 'projects' | 'monitor' | 'system';

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
    { place: 'projects', href: '/projects', label: 'Projects' },
    { place: 'monitor', href: '/monitor/overview', label: 'Monitor' },
    { place: 'system', href: '/system/packages', label: 'System' },
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
      case '/monitor/overview':
        return 'overview';
      case '/monitor/live':
        return 'live';
      case '/monitor/sessions':
        return 'recordings';
      case '/monitor/sessions/[id]':
        return 'recording';
      case '/monitor/sessions/[id]/report':
        return 'report';
      case '/monitor/search':
        return 'search';
      case '/monitor/usage':
        return 'usage';
      case '/system/packages':
        return 'packages';
      case '/system/github':
        return 'github';
      case '/system/recorder':
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
      case 'packages':
      case 'github':
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
        return 'projects';
      case 'overview':
      case 'live':
      case 'traffic':
      case 'recordings':
      case 'search':
      case 'usage':
        return 'monitor';
      case 'packages':
      case 'github':
      case 'recorder':
        return 'system';
      default: {
        const never: never = tab;
        return never;
      }
    }
  }

  /** A place's pages, in the order its bar shows them; inside the project
   * named `project`, the project's own. Home and the Projects list
   * have none: the place's name leads there. */
  static pages(place: Place, project: string | null = null): readonly PageLink[] {
    switch (place) {
      case 'home':
        return [];
      case 'projects': {
        if (project === null) return [];
        const base = `/projects/${encodeURIComponent(project)}`;
        return [
          { tab: 'sessions', href: `${base}/sessions`, label: 'Sessions' },
          { tab: 'release', href: `${base}/release`, label: 'Release' },
        ];
      }
      case 'monitor':
        return [
          { tab: 'overview', href: '/monitor/overview', label: 'Overview' },
          { tab: 'live', href: '/monitor/live', label: 'Live' },
          { tab: 'traffic', href: '/monitor/traffic', label: 'Traffic' },
          { tab: 'recordings', href: '/monitor/sessions', label: 'Sessions' },
          { tab: 'search', href: '/monitor/search', label: 'Search' },
          { tab: 'usage', href: '/monitor/usage', label: 'Usage' },
        ];
      case 'system':
        return [
          { tab: 'packages', href: '/system/packages', label: 'Packages' },
          { tab: 'github', href: '/system/github', label: 'GitHub' },
          { tab: 'recorder', href: '/system/recorder', label: 'Recorder' },
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
      case 'packages':
      case 'github':
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
        return 'Sessions';
      case 'recording':
        return 'Session calls';
      case 'report':
        return 'Session report';
      case 'search':
        return 'Search';
      case 'usage':
        return 'Usage';
      case 'packages':
        return 'Packages';
      case 'github':
        return 'GitHub';
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
