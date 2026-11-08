// A single-select listbox popup (the WAI-ARIA "select-only combobox"), as a
// browser-independent state machine. `Select.svelte` parses DOM events into
// `ListboxEvent`s, feeds them here, and runs the actions; this module never
// touches the DOM, so its keyboard behaviour is tested without a browser.
import { assertNever } from './decode.ts';

/** What the user has typed to jump to an option, and when they last typed. */
export type Search = { text: string; at: number };

export type ListboxState =
  | { kind: 'closed' }
  /** `active` is the highlighted option, always a valid index while open. */
  | { kind: 'open'; active: number; search: Search | null };

/** Keys with a fixed meaning, after `Listbox.key` has parsed them. */
export type ListKey =
  | 'next'
  | 'previous'
  | 'first'
  | 'last'
  | 'page_next'
  | 'page_previous'
  /** Enter: open, or choose the highlighted option. */
  | 'accept'
  /** Alt+ArrowDown: open without moving. */
  | 'reveal'
  /** Alt+ArrowUp: choose the highlighted option and close. */
  | 'accept_close';

export type ListboxEvent =
  | { kind: 'key'; key: ListKey }
  /** A printable character (Space included), typed at `at` milliseconds. */
  | { kind: 'type'; char: string; at: number }
  /** The trigger was clicked. */
  | { kind: 'toggle' }
  /** The pointer moved over an option. */
  | { kind: 'point'; index: number }
  /** An option was clicked. */
  | { kind: 'choose'; index: number }
  /** Tab moved focus on: the highlighted option is chosen, as a native select does. */
  | { kind: 'leave' }
  /** Escape, a click outside, or focus lost: close without choosing. */
  | { kind: 'dismiss' };

export type ListboxAction = { kind: 'commit'; index: number };

/** The options as the machine sees them: their labels, and which one is chosen (-1 for none). */
export type ListboxContext = { labels: readonly string[]; selected: number };

export type ListboxStep =
  | { kind: 'moved'; state: ListboxState; actions: ListboxAction[] }
  /** The event does not apply in this state; the key should keep its default meaning. */
  | { kind: 'ignored'; state: ListboxState };

const moved = (state: ListboxState, ...actions: ListboxAction[]): ListboxStep => ({
  kind: 'moved',
  state,
  actions,
});
const closed: ListboxState = { kind: 'closed' };
const open = (active: number): ListboxState => ({ kind: 'open', active, search: null });

export class Listbox {
  static readonly initial: ListboxState = closed;
  /** Typed characters within this window extend one search. */
  static readonly searchWindowMs = 500;
  static readonly pageSize = 10;

  /** Parse a keydown into an event, or `null` for keys the listbox leaves alone. */
  static key(event: {
    key: string;
    altKey: boolean;
    ctrlKey: boolean;
    metaKey: boolean;
    timeStamp: number;
  }): ListboxEvent | null {
    if (event.ctrlKey || event.metaKey) return null;
    const key = (key: ListKey): ListboxEvent => ({ kind: 'key', key });
    switch (event.key) {
      case 'ArrowDown':
        return key(event.altKey ? 'reveal' : 'next');
      case 'ArrowUp':
        return key(event.altKey ? 'accept_close' : 'previous');
      case 'Home':
        return key('first');
      case 'End':
        return key('last');
      case 'PageDown':
        return key('page_next');
      case 'PageUp':
        return key('page_previous');
      case 'Enter':
        return key('accept');
      case 'Escape':
        return { kind: 'dismiss' };
      case 'Tab':
        return { kind: 'leave' };
      default:
        // One printable character; named keys (`Shift`, `F1`, …) are longer.
        return [...event.key].length === 1 && !event.altKey
          ? { kind: 'type', char: event.key, at: event.timeStamp }
          : null;
    }
  }

  static step(state: ListboxState, event: ListboxEvent, context: ListboxContext): ListboxStep {
    const ignored: ListboxStep = { kind: 'ignored', state };
    const count = context.labels.length;
    if (count === 0) return state.kind === 'open' ? moved(closed) : ignored;
    const last = count - 1;
    const clamp = (index: number) => Math.min(last, Math.max(0, index));
    const selected = context.selected >= 0 && context.selected < count ? context.selected : -1;
    switch (event.kind) {
      case 'toggle':
        return state.kind === 'open' ? moved(closed) : moved(open(Math.max(0, selected)));
      case 'dismiss':
        return state.kind === 'open' ? moved(closed) : ignored;
      case 'leave':
        return state.kind === 'open'
          ? moved(closed, ...Listbox.commit(clamp(state.active), selected))
          : ignored;
      case 'point':
        return state.kind === 'open' && event.index >= 0 && event.index < count
          ? moved({ ...state, active: event.index })
          : ignored;
      case 'choose':
        return state.kind === 'open' && event.index >= 0 && event.index < count
          ? moved(closed, ...Listbox.commit(event.index, selected))
          : ignored;
      case 'type':
        return Listbox.type(state, event, context.labels, selected);
      case 'key': {
        if (state.kind === 'closed') {
          switch (event.key) {
            case 'next':
            case 'previous':
            case 'accept':
            case 'reveal':
              return moved(open(Math.max(0, selected)));
            case 'first':
              return moved(open(0));
            case 'last':
              return moved(open(last));
            case 'page_next':
            case 'page_previous':
            case 'accept_close':
              return ignored;
            default:
              return assertNever(event.key);
          }
        }
        const active = clamp(state.active);
        switch (event.key) {
          case 'next':
            return moved(open(clamp(active + 1)));
          case 'previous':
            return moved(open(clamp(active - 1)));
          case 'first':
            return moved(open(0));
          case 'last':
            return moved(open(last));
          case 'page_next':
            return moved(open(clamp(active + Listbox.pageSize)));
          case 'page_previous':
            return moved(open(clamp(active - Listbox.pageSize)));
          case 'reveal':
            return moved({ ...state, active });
          case 'accept':
          case 'accept_close':
            return moved(closed, ...Listbox.commit(active, selected));
          default:
            return assertNever(event.key);
        }
      }
      default:
        return assertNever(event);
    }
  }

  /** Choosing the option that is already chosen changes nothing. */
  private static commit(index: number, selected: number): ListboxAction[] {
    return index === selected ? [] : [{ kind: 'commit', index }];
  }

  /**
   * Type to jump: the first option, after the highlighted one, whose label
   * starts with what was typed. Repeating one character cycles through the
   * options that start with it. Space outside a search opens or chooses.
   */
  private static type(
    state: ListboxState,
    event: { char: string; at: number },
    labels: readonly string[],
    selected: number,
  ): ListboxStep {
    const previous =
      state.kind === 'open' && state.search && event.at - state.search.at <= Listbox.searchWindowMs
        ? state.search.text
        : '';
    if (event.char === ' ' && previous === '') {
      if (state.kind === 'closed') return moved(open(Math.max(0, selected)));
      return moved(closed, ...Listbox.commit(state.active, selected));
    }
    const text = previous + event.char;
    // With nothing chosen yet, the search starts at the first option.
    const from = state.kind === 'open' ? state.active : selected;
    const repeated = [...text].every((char) => char === text[0]);
    // A new or repeated character looks past the current option; a longer
    // search may still match it.
    const needle = (repeated ? text[0] : text).toLocaleLowerCase();
    const start = repeated ? from + 1 : Math.max(0, from);
    let match = -1;
    for (let offset = 0; offset < labels.length; offset++) {
      const index = (start + offset) % labels.length;
      if (labels[index].toLocaleLowerCase().startsWith(needle)) {
        match = index;
        break;
      }
    }
    const search = { text, at: event.at };
    if (state.kind === 'open')
      return moved({ kind: 'open', active: match === -1 ? state.active : match, search });
    return moved({ kind: 'open', active: match === -1 ? Math.max(0, selected) : match, search });
  }
}
