/**
 * The workbench's views: which one a route is, the tab it sits under, its
 * title, and the shape of its placeholder while its data loads. Pure, so
 * the layout's choices are tested without a browser.
 */

/** A view of the workbench, one per page. */
export type View =
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
  | 'recorder';

/** A tab of the header: reports, turns, and a recording sit under Recordings. */
export type Tab =
  | 'overview'
  | 'live'
  | 'sessions'
  | 'traffic'
  | 'recordings'
  | 'search'
  | 'usage'
  | 'tools'
  | 'recorder';

/**
 * How a view is laid out, which its placeholder follows: tiles over panels,
 * a list of rows, or a sidebar beside a main pane.
 */
export type Shape = 'dashboard' | 'list' | 'split';

export class Views {
  /** The view a SvelteKit route id is; an unknown route is Traffic, the
   * workbench's own view. */
  static of(route: string | null): View {
    switch (route) {
      case '/':
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
      case '/recorder':
        return 'recorder';
      default:
        return 'traffic';
    }
  }

  /** The header tab `view` is shown under. */
  static tab(view: View): Tab {
    switch (view) {
      case 'recording':
      case 'report':
      case 'turn':
        return 'recordings';
      case 'overview':
      case 'live':
      case 'sessions':
      case 'traffic':
      case 'recordings':
      case 'search':
      case 'usage':
      case 'tools':
      case 'recorder':
        return view;
      default: {
        const never: never = view;
        return never;
      }
    }
  }

  /** What the toolbar and the document title call `view`. */
  static title(view: View): string {
    switch (view) {
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
      case 'recorder':
        return 'Recorder & storage';
      default: {
        const never: never = view;
        return never;
      }
    }
  }

  /** The shape of `view`'s placeholder. */
  static shape(view: View): Shape {
    switch (view) {
      case 'overview':
      case 'usage':
      case 'report':
      case 'recorder':
        return 'dashboard';
      case 'live':
      case 'traffic':
      case 'recordings':
      case 'recording':
      case 'turn':
      case 'search':
        return 'list';
      case 'sessions':
      case 'tools':
        return 'split';
      default: {
        const never: never = view;
        return never;
      }
    }
  }
}
