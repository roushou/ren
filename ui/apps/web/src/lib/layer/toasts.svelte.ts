// Layer 5: what happened away from where the operator is looking, said once
// and gone after a while. One stack for the whole UI, drawn by
// `Toasts.svelte` in the layout; anything may add to it.

import type { Mark } from '#lib/marks.ts';

/** Where a toast leads: the page that has the rest. */
export interface ToastLink {
  label: string;
  href: string;
}

/** One thing that happened: its mark, what happened, its detail, and where
 * to see more. */
export interface Toast {
  id: number;
  mark: Mark;
  text: string;
  detail: string | null;
  link: ToastLink | null;
}

/** What a caller says: a toast before the stack numbers it. */
export type Said = Omit<Toast, 'id' | 'link'> & { link?: ToastLink | null };

/** How long a toast stays, unless dismissed first. */
const STAY_MS = 6_000;

class ToastStack {
  items = $state<Toast[]>([]);
  #next = 0;

  show(toast: Said): void {
    const id = ++this.#next;
    this.items = [...this.items, { ...toast, link: toast.link ?? null, id }];
    setTimeout(() => this.dismiss(id), STAY_MS);
  }

  dismiss(id: number): void {
    this.items = this.items.filter((toast) => toast.id !== id);
  }
}

export const toasts = new ToastStack();
