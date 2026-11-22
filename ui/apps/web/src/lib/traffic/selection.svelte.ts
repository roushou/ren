// The call open beside the listing. The address names it (`?call=`), so Back,
// Forward, and a reload show the same; a clicked row is selected at once and
// the address catches up a moment later.
import { untrack } from 'svelte';
import { goto } from '$app/navigation';
import { page } from '$app/state';
import { CallId, type CallView } from '../api.ts';
import { offPageSelection, type OffPageSelection } from '../loaders.ts';
import { neighbour, selectedCall } from '../traffic.ts';

export class CallSelection {
  readonly #calls: () => CallView[];
  /** A picked call the address has not caught up with yet. */
  #pending = $state<CallId | null>(null);
  /** The inspector's newest live observation of its call. */
  #observed = $state<CallView | undefined>();
  /** A linked call that is not on this page, read on its own; the listing
   * never reloads for a selection. */
  #offPage = $state.raw<OffPageSelection>({ kind: 'none' });
  /** The call the address names, as written there. */
  readonly linked = $derived(page.url.searchParams.get('call'));
  /** The call open beside the list: only one the operator picked. */
  readonly id = $derived(
    this.#pending ?? (this.linked !== null ? CallId.parse(this.linked) : null),
  );
  /** The selected call as the inspector shows it. */
  readonly call: CallView | undefined;
  /** Why a linked call could not be shown, or empty. */
  readonly error: string;

  /** Made while a component starts: it follows the address for that component's life. */
  constructor(calls: () => CallView[], fetcher: typeof fetch) {
    this.#calls = calls;
    this.call = $derived(
      selectedCall(
        this.id,
        this.#calls(),
        this.#observed,
        this.#offPage.kind === 'loaded' ? this.#offPage.call : undefined,
      ),
    );
    this.error = $derived(
      this.linked !== null && !this.call && this.#offPage.kind === 'failed'
        ? this.#offPage.message
        : '',
    );
    $effect(() => {
      const raw = this.linked;
      const listed = this.#calls();
      if (raw === null || listed.some((call) => call.id === raw)) {
        this.#offPage = { kind: 'none' };
        return;
      }
      const current = untrack(() => this.#offPage);
      if (current.kind === 'loaded' && current.call.id === raw) return;
      const controller = new AbortController();
      void offPageSelection(fetcher, raw, listed, controller.signal).then((result) => {
        if (!controller.signal.aborted) this.#offPage = result;
      });
      return () => controller.abort();
    });
  }

  /** The address that selects `id`, on this page. */
  href = (id: CallId): string => {
    const url = new URL(page.url.href);
    url.searchParams.set('call', id);
    url.searchParams.delete('inspect');
    return url.pathname + url.search;
  };

  /** Select `id` from a plain click; a modified click opens the link as usual. */
  select = (event: MouseEvent, id: CallId): void => {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)
      return;
    event.preventDefault();
    this.#go(id);
  };

  /** A click anywhere on a row selects it, except on its own links and
   * buttons, or when the click ended a text selection. */
  selectRow = (event: MouseEvent, id: CallId): void => {
    if (event.target instanceof Element && event.target.closest('a,button,input,select')) return;
    if (window.getSelection()?.toString()) return;
    this.select(event, id);
  };

  /** `j` and `k` select the next and previous call, as in a mail reader. */
  step = (direction: 1 | -1): void => {
    const next = neighbour(this.#calls(), this.id, direction);
    if (!next || next.id === this.id) return;
    this.#go(next.id);
    document.getElementById(`call-${next.id}`)?.closest('tr')?.scrollIntoView({ block: 'nearest' });
  };

  /** Close the inspector and return focus to the call's row. */
  close = async (): Promise<void> => {
    const id = this.id;
    const url = new URL(page.url.href);
    url.searchParams.delete('call');
    await goto(url, { reset: false, state: page.state, persistState: true });
    if (id) document.getElementById(`call-${id}`)?.focus();
  };

  /** What the inspector last observed of its call. */
  observe = (record: CallView): void => {
    this.#observed = record;
  };

  #go(id: CallId): void {
    this.#pending = id;
    void goto(this.href(id), {
      reset: false,
      state: page.state,
      persistState: true,
    }).finally(() => {
      // Whether it landed or was superseded, the address decides from here.
      if (this.#pending === id) this.#pending = null;
    });
  }
}
