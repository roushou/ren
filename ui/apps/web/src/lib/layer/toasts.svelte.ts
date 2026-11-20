// Layer 5: what happened away from where the operator is looking, said once
// and gone after a while. One stack for the whole UI, drawn by
// `Toasts.svelte` in the layout; anything may add to it.

import type { Mark } from '#lib/marks.ts';

/** One thing that happened: its mark, what happened, and its detail. */
export interface Toast {
  id: number;
  mark: Mark;
  text: string;
  detail: string | null;
}

/** How long a toast stays, unless dismissed first. */
const STAY_MS = 6_000;

class ToastStack {
  items = $state<Toast[]>([]);
  #next = 0;

  show(toast: Omit<Toast, 'id'>): void {
    const id = ++this.#next;
    this.items = [...this.items, { ...toast, id }];
    setTimeout(() => this.dismiss(id), STAY_MS);
  }

  dismiss(id: number): void {
    this.items = this.items.filter((toast) => toast.id !== id);
  }
}

export const toasts = new ToastStack();
